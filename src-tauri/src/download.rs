//! Downloading posts from TikTok, Instagram and X/Twitter.
//!
//! * Instagram and X go through yt-dlp. An Instagram post made of pictures has no video
//!   for yt-dlp to download, so its pictures are fetched instead: one picture is a photo
//!   to edit, several are built into a video like a TikTok photo post.
//! * TikTok goes through a watermark-free API first (original quality, and the
//!   only way to get photo posts), falling back to yt-dlp for plain videos.

use fillerncut_core::export::{
    build_slideshow_args, slideshow_duration, slideshow_seconds_per_image, SlideshowSpec,
};
use fillerncut_core::links::{parse_link, tiktok_id_from_resolved, ParsedLink, Platform};
use fillerncut_core::tiktok::{api_query_url, parse_response, TikTokError, TikTokMedia, API_HOST};
use fillerncut_core::ytdlp::{
    build_args, build_info_args, explain_error, is_no_video_error, parse_line, parse_photo_post,
    parse_post_music, DownloadOptions, PostMusic, YtDlpLine,
};
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tauri::{AppHandle, Runtime, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::bins;
use crate::editor::{allow_asset, pick_encoder};
use crate::jobs::{self, collect_tail, emit_progress, CANCELLED};
use crate::state::AppState;

const JOB: &str = "download";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    pub path: String,
    pub platform: Platform,
    pub title: Option<String>,
    /// Something worth telling the user about how the download went.
    pub note: Option<String>,
}

/// The TikTok download API. Overridable so the integration tests can point it
/// at a local server.
fn tiktok_api_host() -> String {
    std::env::var("FILLERNCUT_TIKTOK_API_HOST").unwrap_or_else(|_| API_HOST.to_string())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| e.to_string())
}

/// Delete a downloaded video once the user is done with it. Files that aren't
/// temporary downloads are never touched.
#[tauri::command]
pub fn discard_download(state: State<'_, AppState>, path: String) -> bool {
    crate::dirs::discard_download(&state.cache_dir, Path::new(&path))
}

#[tauri::command]
pub async fn download_link<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    url: String,
) -> Result<DownloadResult, String> {
    let link = parse_link(&url).map_err(|e| e.to_string())?;
    let out_dir = crate::dirs::downloads_dir(&state.cache_dir);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Can't create the download folder {}: {e}", out_dir.display()))?;

    // One timestamp name for whatever this download produces.
    let target = crate::dirs::new_video_path(&out_dir).map_err(|e| e.to_string())?;
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string();

    let guard = state.begin_job(JOB)?;
    emit_progress(&app, JOB, None, Some("Looking up the post…"));

    let platform = link.platform();
    let (path, title, note) = match &link {
        ParsedLink::TikTok { .. } | ParsedLink::TikTokShort { .. } => {
            let (path, title) = download_tiktok(&app, &state, &guard.token, &link, &out_dir, &stem).await?;
            (path, title, None)
        }
        ParsedLink::Instagram { url } | ParsedLink::Twitter { url } => {
            let (path, note) =
                download_with_ytdlp(&app, &state, &guard.token, url, platform, &out_dir, &stem).await?;
            (path, None, note)
        }
    };

    allow_asset(&app, &path);
    // Not added to the history: the file is deleted once the export is done.
    Ok(DownloadResult {
        path: path.to_string_lossy().into_owned(),
        platform,
        title,
        note,
    })
}

// ───────────────────────────── yt-dlp ─────────────────────────────

async fn download_with_ytdlp<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    token: &CancellationToken,
    url: &str,
    platform: Platform,
    out_dir: &Path,
    stem: &str,
) -> Result<(PathBuf, Option<String>), String> {
    let settings = state.settings();
    let opts = DownloadOptions {
        url: url.to_string(),
        out_dir: out_dir.to_path_buf(),
        file_stem: stem.to_string(),
        ffmpeg_dir: bins::ffmpeg_dir(),
        cookies_browser: settings.cookies_browser,
    };
    let args = build_args(&opts);
    if !bins::ytdlp_is_managed(&state.data_dir) {
        emit_progress(
            app,
            JOB,
            None,
            Some("Setting up the downloader (first time only)…"),
        );
    }
    let program = crate::settings::ensure_ytdlp(&state.data_dir).await?;

    let mut cmd = Command::new(&program);
    bins::hide_window(&mut cmd);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = bins::ffmpeg_dir() {
        // yt-dlp also shells out to ffprobe/ffmpeg by name.
        let mut paths = vec![dir];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Couldn't start the downloader ({}): {e}", program.display()))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr_task = tokio::spawn(collect_tail(child.stderr.take().ok_or("no stderr")?, 60));
    let mut lines = BufReader::new(stdout).lines();

    let mut file: Option<String> = None;
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = child.kill().await;
                return Err(CANCELLED.into());
            }
            line = lines.next_line() => match line {
                Ok(Some(l)) => match parse_line(&l) {
                    YtDlpLine::Progress { fraction, .. } => emit_progress(app, JOB, fraction, Some("Downloading…")),
                    YtDlpLine::File(f) => file = Some(f),
                    YtDlpLine::Other => {}
                },
                Ok(None) => break,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let stderr = stderr_task.await.unwrap_or_default();
    if !status.success() {
        if matches!(platform, Platform::Instagram) && is_no_video_error(&stderr) {
            if let Some(found) =
                download_instagram_photos(app, state, token, &program, &opts, out_dir, stem).await?
            {
                return Ok(found);
            }
        }
        return Err(explain_error(
            &stderr,
            platform,
            settings.cookies_browser.is_some(),
        ));
    }
    let path = file.ok_or("The download finished but no file was produced")?;
    if !Path::new(&path).is_file() {
        return Err("The download finished but the file is missing".into());
    }
    emit_progress(app, JOB, Some(1.0), None);
    Ok((PathBuf::from(path), None))
}

/// What yt-dlp found out about a photo post.
struct PhotoPost {
    images: Vec<String>,
    music: Option<PostMusic>,
}

const NO_LOGIN_HINT: &str = "If this post has music, choose the browser you're logged into Instagram with in Settings → Downloads, then add the link again to get it.";
const MUSIC_FAILED: &str = "This post's music couldn't be downloaded, so it has no sound.";

/// An Instagram post made of pictures has nothing for yt-dlp to download, but yt-dlp still
/// knows where the pictures are. With music (one photo or several) the result is a video
/// like a TikTok photo post; one picture without music is a photo to edit. `None` when the
/// post isn't a photo post after all, so the caller can show yt-dlp's own error.
async fn download_instagram_photos<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    token: &CancellationToken,
    program: &Path,
    opts: &DownloadOptions,
    out_dir: &Path,
    stem: &str,
) -> Result<Option<(PathBuf, Option<String>)>, String> {
    emit_progress(app, JOB, None, Some("Looking for the photos…"));
    let work = state.cache_dir.join(format!("instagram-{stem}"));
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work)
        .await
        .map_err(|e| e.to_string())?;
    let found = look_up_photo_post(token, program, opts, &work).await;
    let _ = tokio::fs::remove_dir_all(&work).await;
    let Some(post) = found? else {
        return Ok(None);
    };

    let client = http_client()?;
    let single = post.images.len() == 1;
    let sound = post.music.as_ref().map(|m| Sound {
        url: &m.url,
        start: m.start,
        length: m.length,
    });
    let had_music = sound.is_some();
    if had_music || !single {
        // A single photo only becomes a video when its music can be had.
        match build_photo_slideshow(
            app,
            state,
            token,
            &client,
            stem,
            &post.images,
            sound,
            single,
            out_dir,
            stem,
        )
        .await
        {
            Ok(path) => {
                let note = (!had_music && opts.cookies_browser.is_none()).then(|| NO_LOGIN_HINT.to_string());
                return Ok(Some((path, note)));
            }
            Err(e) if e == NO_SOUND => {}
            Err(e) => return Err(e),
        }
    }

    // One picture and no music to go with it: a photo, which the editor opens.
    let Some(url) = post.images.first() else {
        return Ok(None);
    };
    let dest = out_dir.join(format!("{stem}.jpg"));
    emit_progress(app, JOB, Some(0.0), Some("Downloading the photo…"));
    download_file(&client, url, &dest, token, |f| {
        emit_progress(app, JOB, Some(f), Some("Downloading the photo…"))
    })
    .await?;
    let note = if had_music {
        Some(MUSIC_FAILED.to_string())
    } else if opts.cookies_browser.is_none() {
        Some(NO_LOGIN_HINT.to_string())
    } else {
        None
    };
    Ok(Some((dest, note)))
}

/// Ask yt-dlp what is in the post. It also saves the answers it got from Instagram, which
/// is where the music is; they go in `work` and are read before returning.
async fn look_up_photo_post(
    token: &CancellationToken,
    program: &Path,
    opts: &DownloadOptions,
    work: &Path,
) -> Result<Option<PhotoPost>, String> {
    let mut cmd = Command::new(program);
    bins::hide_window(&mut cmd);
    cmd.args(build_info_args(opts))
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::select! {
        _ = token.cancelled() => return Err(CANCELLED.into()),
        o = cmd.output() => o,
    };
    // If this second look fails too, yt-dlp's first answer ("no video") is the useful one.
    let Ok(output) = output else { return Ok(None) };
    if !output.status.success() {
        return Ok(None);
    }
    let Some(images) = parse_photo_post(&String::from_utf8_lossy(&output.stdout)) else {
        return Ok(None);
    };
    Ok(Some(PhotoPost {
        images,
        music: read_post_music(work),
    }))
}

/// The music in any of the answers yt-dlp saved from Instagram.
fn read_post_music(dir: &Path) -> Option<PostMusic> {
    const MAX_PAGE_BYTES: u64 = 16 * 1024 * 1024;
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        let is_page = path.extension().is_some_and(|e| e == "dump");
        if !is_page || entry.metadata().ok()?.len() > MAX_PAGE_BYTES {
            return None;
        }
        parse_post_music(&std::fs::read_to_string(path).ok()?)
    })
}

// ───────────────────────────── TikTok ─────────────────────────────

async fn download_tiktok<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    token: &CancellationToken,
    link: &ParsedLink,
    out_dir: &Path,
    stem: &str,
) -> Result<(PathBuf, Option<String>), String> {
    let client = http_client()?;
    let canonical = match link {
        ParsedLink::TikTok { url, .. } => url.clone(),
        ParsedLink::TikTokShort { url } => resolve_short_link(&client, url).await?,
        _ => unreachable!("download_tiktok only handles TikTok links"),
    };

    match fetch_tiktok_media(&client, &canonical, token).await {
        Ok(TikTokMedia::Video { url, title, .. }) => {
            let dest = out_dir.join(format!("{stem}.mp4"));
            emit_progress(app, JOB, Some(0.0), Some("Downloading…"));
            match download_file(&client, &url, &dest, token, |f| {
                emit_progress(app, JOB, Some(f), Some("Downloading…"))
            })
            .await
            {
                Ok(()) => return Ok((dest, title)),
                Err(e) if e == CANCELLED => return Err(e),
                Err(_) => {} // fall through to yt-dlp
            }
        }
        Ok(TikTokMedia::Photos {
            id,
            images,
            music,
            title,
        }) => {
            let path = build_photo_slideshow(
                app,
                state,
                token,
                &client,
                &id,
                &images,
                music.as_deref().map(Sound::whole),
                false,
                out_dir,
                stem,
            )
            .await?;
            return Ok((path, title));
        }
        Err(e) if e == CANCELLED => return Err(e),
        Err(_) => {} // fall through to yt-dlp
    }

    // Fallback: yt-dlp handles ordinary TikTok videos on its own.
    emit_progress(app, JOB, None, Some("Trying the backup downloader…"));
    download_with_ytdlp(app, state, token, &canonical, Platform::TikTok, out_dir, stem)
        .await
        .map(|(path, _)| (path, None))
}

async fn resolve_short_link(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|_| "Couldn't open that TikTok link. Check your internet connection".to_string())?;
    let final_url = resp.url().to_string();
    match tiktok_id_from_resolved(&final_url) {
        Some(id) => {
            // Keep the path (video vs photo) when TikTok gives us one.
            match parse_link(&final_url) {
                Ok(ParsedLink::TikTok { url, .. }) => Ok(url),
                _ => Ok(format!("https://www.tiktok.com/@_/video/{id}")),
            }
        }
        None => Err("That TikTok link doesn't point to a post".into()),
    }
}

async fn fetch_tiktok_media(
    client: &reqwest::Client,
    canonical: &str,
    token: &CancellationToken,
) -> Result<TikTokMedia, String> {
    // The free API allows one request per second; retry once on its rate limit.
    for attempt in 0..2 {
        let body = tokio::select! {
            _ = token.cancelled() => return Err(CANCELLED.into()),
            r = async {
                client.get(api_query_url(&tiktok_api_host(), canonical)).timeout(Duration::from_secs(30)).send().await?.text().await
            } => r.map_err(|e| e.to_string())?,
        };
        match parse_response(&body) {
            Ok(media) => return Ok(media),
            Err(TikTokError::Api(msg)) if attempt == 0 && msg.to_ascii_lowercase().contains("limit") => {
                tokio::time::sleep(Duration::from_millis(1300)).await;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("The TikTok service is busy. Try again in a moment".into())
}

async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    token: &CancellationToken,
    on_progress: impl Fn(f64),
) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Download failed: {e}"))?;
    let total = resp.content_length().filter(|t| *t > 0);
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let mut done: u64 = 0;

    let result: Result<(), String> = async {
        loop {
            let chunk = tokio::select! {
                _ = token.cancelled() => return Err(CANCELLED.to_string()),
                c = stream.next() => c,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|e| format!("Download interrupted: {e}"))?;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            done += chunk.len() as u64;
            if let Some(t) = total {
                on_progress((done as f64 / t as f64).clamp(0.0, 1.0));
            }
        }
        file.flush().await.map_err(|e| e.to_string())
    }
    .await;

    if result.is_err() {
        drop(file);
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

/// Length of an audio file in seconds, if ffprobe can tell.
async fn audio_length(path: &str) -> Option<f64> {
    jobs::capture(
        &bins::ffprobe(),
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            path,
        ],
    )
    .await
    .ok()?
    .trim()
    .parse()
    .ok()
}

/// A post's sound and the stretch of it to use.
struct Sound<'a> {
    url: &'a str,
    /// Seconds into the track where the post starts playing it.
    start: f64,
    /// How many seconds of it the post plays; `None` for all the rest.
    length: Option<f64>,
}

impl<'a> Sound<'a> {
    fn whole(url: &'a str) -> Self {
        Sound {
            url,
            start: 0.0,
            length: None,
        }
    }
}

/// What `build_photo_slideshow` fails with when the sound was required and can't be had.
const NO_SOUND: &str = "the sound of the post could not be downloaded";

/// Download a post's sound into `dir`, cut to the stretch the post plays. `None` when it
/// can't be had: the sound is nice to have, a missing track shouldn't fail the post.
async fn fetch_sound(
    client: &reqwest::Client,
    sound: &Sound<'_>,
    dir: &Path,
    token: &CancellationToken,
) -> Result<Option<String>, String> {
    let full = dir.join("music.mp3");
    match download_file(client, sound.url, &full, token, |_| {}).await {
        Ok(()) => {}
        Err(e) if e == CANCELLED => return Err(e),
        Err(_) => return Ok(None),
    }
    let full = full.to_string_lossy().into_owned();
    if sound.start <= 0.0 && sound.length.is_none() {
        return Ok(Some(full));
    }

    let cut = dir.join("music-cut.m4a").to_string_lossy().into_owned();
    let (start, length) = (
        format!("{:.3}", sound.start),
        sound.length.map(|l| format!("{l:.3}")),
    );
    let mut args = vec!["-hide_banner", "-nostdin", "-y", "-ss", &start];
    if let Some(length) = &length {
        args.extend(["-t", length]);
    }
    args.extend(["-i", &full, "-vn", "-c:a", "aac", "-b:a", "192k", &cut]);
    let _ = jobs::capture_stderr(&bins::ffmpeg(), &args).await;
    // Nothing usable (e.g. the start is past the end of the track): use the whole track.
    Ok(Some(match audio_length(&cut).await {
        Some(seconds) if seconds > 0.1 => cut,
        _ => full,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn build_photo_slideshow<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    token: &CancellationToken,
    client: &reqwest::Client,
    id: &str,
    images: &[String],
    music: Option<Sound<'_>>,
    require_sound: bool,
    out_dir: &Path,
    stem: &str,
) -> Result<PathBuf, String> {
    let work = state.cache_dir.join(format!("slideshow-{id}"));
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work)
        .await
        .map_err(|e| e.to_string())?;

    // Photos + music are the "download" half (first 40% of the bar); the ffmpeg
    // build is the rest.
    let steps = images.len() + usize::from(music.is_some());
    let mut local_images = Vec::new();
    for (i, url) in images.iter().enumerate() {
        let dest = work.join(format!("img_{i:02}.jpg"));
        let base = i as f64 / steps as f64;
        download_file(client, url, &dest, token, |f| {
            emit_progress(
                app,
                JOB,
                Some((base + f / steps as f64) * 0.4),
                Some("Downloading photos…"),
            )
        })
        .await
        .inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&work);
        })?;
        local_images.push(dest.to_string_lossy().into_owned());
    }
    let mut local_audio = None;
    if let Some(sound) = &music {
        match fetch_sound(client, sound, &work, token).await {
            Ok(path) => local_audio = path,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&work);
                return Err(e);
            }
        }
        if local_audio.is_none() && require_sound {
            let _ = std::fs::remove_dir_all(&work);
            return Err(NO_SOUND.into());
        }
    }

    // One photo + music is a "picture with a sound": make the video as long as the music.
    let audio_seconds = match &local_audio {
        Some(a) if local_images.len() == 1 => audio_length(a).await,
        _ => None,
    };

    let seconds_per_image = slideshow_seconds_per_image(local_images.len(), audio_seconds);
    let dest = out_dir.join(format!("{stem}.mp4"));
    let partial = out_dir.join(format!("{stem}.partial.mp4"));
    let spec = SlideshowSpec {
        images: local_images,
        audio: local_audio,
        output: partial.to_string_lossy().into_owned(),
        seconds_per_image,
    };
    let encoder = pick_encoder(state).await?;
    let args = build_slideshow_args(&spec, encoder).map_err(|e| e.to_string())?;
    emit_progress(app, JOB, Some(0.4), Some("Building the video from photos…"));

    // Map the ffmpeg 0..1 progress onto the last 60% of the bar.
    let total = slideshow_duration(&spec);
    let res = jobs::run_ffmpeg(token, &args, total, |f| {
        emit_progress(
            app,
            JOB,
            Some(0.4 + f * 0.6),
            Some("Building the video from photos…"),
        )
    })
    .await;
    let _ = std::fs::remove_dir_all(&work);
    match res {
        Ok(()) => {
            std::fs::rename(&partial, &dest).map_err(|e| e.to_string())?;
            Ok(dest)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}
