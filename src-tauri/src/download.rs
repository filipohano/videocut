//! Downloading posts from TikTok, Instagram and X/Twitter.
//!
//! * Instagram and X go through yt-dlp.
//! * TikTok goes through a watermark-free API first (original quality, and the
//!   only way to get photo posts), falling back to yt-dlp for plain videos.

use fillerncut_core::export::{build_slideshow_args, slideshow_duration, SlideshowSpec};
use fillerncut_core::links::{parse_link, tiktok_id_from_resolved, ParsedLink, Platform};
use fillerncut_core::tiktok::{api_query_url, parse_response, TikTokError, TikTokMedia, API_HOST};
use fillerncut_core::ytdlp::{build_args, explain_error, parse_line, DownloadOptions, YtDlpLine};
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
/// How long each photo of a TikTok photo post is shown.
const SECONDS_PER_PHOTO: f64 = 3.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    pub path: String,
    pub platform: Platform,
    pub title: Option<String>,
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
    let result = match &link {
        ParsedLink::TikTok { .. } | ParsedLink::TikTokShort { .. } => {
            download_tiktok(&app, &state, &guard.token, &link, &out_dir, &stem).await
        }
        ParsedLink::Instagram { url } | ParsedLink::Twitter { url } => {
            download_with_ytdlp(&app, &state, &guard.token, url, platform, &out_dir, &stem)
                .await
                .map(|p| (p, None))
        }
    }?;

    let (path, title) = result;
    allow_asset(&app, &path);
    // Not added to the history: the file is deleted once the export is done.
    Ok(DownloadResult {
        path: path.to_string_lossy().into_owned(),
        platform,
        title,
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
) -> Result<PathBuf, String> {
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
    Ok(PathBuf::from(path))
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
                music.as_deref(),
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
        .map(|p| (p, None))
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

#[allow(clippy::too_many_arguments)]
async fn build_photo_slideshow<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    token: &CancellationToken,
    client: &reqwest::Client,
    id: &str,
    images: &[String],
    music: Option<&str>,
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
    if let Some(url) = music {
        let dest = work.join("music.mp3");
        // The sound is nice to have; a missing track shouldn't fail the post.
        match download_file(client, url, &dest, token, |_| {}).await {
            Ok(()) => local_audio = Some(dest.to_string_lossy().into_owned()),
            Err(e) if e == CANCELLED => {
                let _ = std::fs::remove_dir_all(&work);
                return Err(e);
            }
            Err(_) => {}
        }
    }

    let dest = out_dir.join(format!("{stem}.mp4"));
    let partial = out_dir.join(format!("{stem}.partial.mp4"));
    let spec = SlideshowSpec {
        images: local_images,
        audio: local_audio,
        output: partial.to_string_lossy().into_owned(),
        seconds_per_image: SECONDS_PER_PHOTO,
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
