//! yt-dlp: command line, output parsing and friendly error messages.

use crate::links::Platform;
use crate::settings::CookieBrowser;
use std::path::PathBuf;

/// Line prefixes we ask yt-dlp to emit so they can't be confused with its
/// normal chatter.
const PROGRESS_PREFIX: &str = "FCPROG|";
const FILE_PREFIX: &str = "FCFILE|";
const LIVE_PREFIX: &str = "FCLIVE|";

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub url: String,
    pub out_dir: PathBuf,
    /// File name without extension (a timestamp), chosen by the caller so it is unique.
    pub file_stem: String,
    /// Directory holding ffmpeg/ffprobe (needed to merge video + audio).
    pub ffmpeg_dir: Option<PathBuf>,
    pub cookies_browser: Option<CookieBrowser>,
}

pub fn build_args(opts: &DownloadOptions) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--no-playlist".into(),
        "--no-warnings".into(),
        "--no-mtime".into(),
        // Re-downloading must always report the final path, so never skip.
        "--force-overwrites".into(),
        "--restrict-filenames".into(),
        // Prefer H.264/AAC so the file plays in the preview and edits quickly, then fall back
        // to whatever is best. Never more than 1080p: sites like YouTube only offer more in
        // VP9/AV1, which are huge and slow to edit, and the export is a social-media video anyway.
        // (`<=?` lets a format through when its height isn't known.)
        "-f".into(),
        "bv*[vcodec^=avc1][height<=?1080]+ba[ext=m4a]/bv*[height<=?1080]+ba/b[height<=?1080]/b".into(),
        // A live stream that is still going would download until it ends. Skip it, but report it
        // first (`--print` is quiet, so the skip itself would be invisible).
        "--match-filters".into(),
        "!is_live".into(),
        "--print".into(),
        format!("pre_process:{LIVE_PREFIX}%(live_status)s"),
        "--merge-output-format".into(),
        "mp4".into(),
        "-o".into(),
        opts.out_dir
            .join(format!("{}.%(ext)s", opts.file_stem))
            .to_string_lossy()
            .into_owned(),
        // `--print` implies --quiet, so progress has to be requested explicitly.
        "--progress".into(),
        "--newline".into(),
        "--progress-template".into(),
        format!(
            "download:{PROGRESS_PREFIX}%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s"
        ),
        "--print".into(),
        format!("after_move:{FILE_PREFIX}%(filepath)s"),
    ];
    if let Some(dir) = &opts.ffmpeg_dir {
        a.push("--ffmpeg-location".into());
        a.push(dir.to_string_lossy().into_owned());
    }
    if let Some(browser) = opts.cookies_browser {
        a.push("--cookies-from-browser".into());
        a.push(browser.ytdlp_name().into());
    }
    a.push("--".into());
    a.push(opts.url.clone());
    a
}

/// Arguments that make yt-dlp print what it knows about a post as one JSON document,
/// without downloading anything and without failing on a post that has no video in it.
/// Used to find the pictures (and music) of a photo post.
pub fn build_info_args(opts: &DownloadOptions) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--no-playlist".into(),
        "--no-warnings".into(),
        "--ignore-no-formats-error".into(),
        "--no-download".into(),
        "--dump-single-json".into(),
        // yt-dlp's JSON leaves out the post's music, but the answers it got from
        // Instagram have it. These are written to the current directory, so the
        // caller must run this in a scratch folder.
        "--write-pages".into(),
    ];
    if let Some(browser) = opts.cookies_browser {
        a.push("--cookies-from-browser".into());
        a.push(browser.ytdlp_name().into());
    }
    a.push("--".into());
    a.push(opts.url.clone());
    a
}

/// yt-dlp refused the post because there is no video in it: a photo post (or X
/// tweet with only pictures). Instagram says "There is no video in this post" for a
/// single photo and "No video formats found!" for a carousel of photos.
pub fn is_no_video_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("there is no video in this post")
        || lower.contains("no video could be found")
        || lower.contains("no video formats found")
}

/// The best picture of each photo in a photo post, in post order, from the JSON of
/// [`build_info_args`]. `None` when this isn't a photo post: yt-dlp lists
/// formats or a duration for anything that is a video, and then we must not quietly
/// turn the post's cover picture into the "video".
pub fn parse_photo_post(json: &str) -> Option<Vec<String>> {
    let post: serde_json::Value = serde_json::from_str(json).ok()?;
    let items: Vec<&serde_json::Value> = match post.get("entries").and_then(|e| e.as_array()) {
        Some(entries) => entries.iter().collect(),
        None => vec![&post],
    };
    if items.is_empty() {
        return None;
    }
    items.into_iter().map(photo_url).collect()
}

fn photo_url(item: &serde_json::Value) -> Option<String> {
    if !item.is_object() {
        return None;
    }
    let has_formats = item
        .get("formats")
        .and_then(|f| f.as_array())
        .is_some_and(|f| !f.is_empty());
    let duration = item.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
    if has_formats || duration > 0.0 {
        return None;
    }
    // yt-dlp lists the pictures smallest first. Instagram gives no sizes, so the last
    // one is the full-size picture; when sizes are known, the biggest wins.
    let candidates: Vec<(&str, u64)> = item
        .get("thumbnails")
        .and_then(|t| t.as_array())?
        .iter()
        .filter_map(|t| {
            let url = t.get("url")?.as_str()?;
            let area = t.get("width").and_then(|w| w.as_u64()).unwrap_or(0)
                * t.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
            (url.starts_with("https://") || url.starts_with("http://")).then_some((url, area))
        })
        .collect();
    let best = candidates
        .iter()
        .enumerate()
        .max_by_key(|(i, (_, area))| (*area, *i))
        .map(|(_, (url, _))| *url)
        .or_else(|| item.get("thumbnail").and_then(|t| t.as_str()))?;
    Some(best.to_string())
}

/// The sound attached to an Instagram photo post.
#[derive(Debug, Clone, PartialEq)]
pub struct PostMusic {
    pub url: String,
    /// Where in the track the post starts playing it, in seconds.
    pub start: f64,
    /// How much of the track the post plays, in seconds, when Instagram says.
    pub length: Option<f64>,
}

/// Look for a post's music in one of the answers yt-dlp got from Instagram (a page
/// written by `--write-pages`). Only the logged-in answer has it, and only when the
/// post has music; anything else, including web pages, gives `None`.
pub fn parse_post_music(page: &str) -> Option<PostMusic> {
    let doc: serde_json::Value = serde_json::from_str(page).ok()?;
    let scope = find_key(&doc, "music_metadata").or_else(|| find_key(&doc, "clips_metadata"))?;
    let asset = find_key(scope, "music_asset_info")?;
    let url = ["progressive_download_url", "fast_start_progressive_download_url"]
        .iter()
        .find_map(|key| asset.get(*key)?.as_str())
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))?;
    let seconds = |key: &str| {
        find_key(scope, key)
            .and_then(|v| v.as_f64())
            .filter(|ms| ms.is_finite() && *ms >= 0.0)
            .map(|ms| ms / 1000.0)
    };
    Some(PostMusic {
        url: url.to_string(),
        start: seconds("audio_asset_start_time_in_ms").unwrap_or(0.0),
        length: seconds("overlap_duration_in_ms").filter(|l| *l > 0.0),
    })
}

/// The first non-null value stored under `key`, at any depth.
fn find_key<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map
            .get(key)
            .filter(|v| !v.is_null())
            .or_else(|| map.values().find_map(|v| find_key(v, key))),
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}

/// Find the SHA-256 for `file_name` in a `SHA2-256SUMS` listing (`<hash>  <name>` per line).
pub fn checksum_for(sums: &str, file_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        (name == file_name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum YtDlpLine {
    Progress {
        /// 0..=1 when the total is known (even if only estimated).
        fraction: Option<f64>,
        /// Bytes per second.
        speed: Option<f64>,
    },
    /// The final path of a finished download.
    File(String),
    /// What yt-dlp says about whether the video is a live stream (`is_live`, `not_live`, `was_live`…).
    LiveStatus(String),
    Other,
}

fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s == "NA" || s == "None" {
        return None;
    }
    s.parse::<f64>().ok()
}

pub fn parse_line(line: &str) -> YtDlpLine {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix(FILE_PREFIX) {
        return YtDlpLine::File(rest.trim().to_string());
    }
    if let Some(rest) = line.strip_prefix(LIVE_PREFIX) {
        return YtDlpLine::LiveStatus(rest.trim().to_string());
    }
    if let Some(rest) = line.strip_prefix(PROGRESS_PREFIX) {
        let f: Vec<&str> = rest.split('|').collect();
        if f.len() >= 4 {
            let downloaded = parse_num(f[0]);
            let total = parse_num(f[1]).or_else(|| parse_num(f[2]));
            let fraction = match (downloaded, total) {
                (Some(d), Some(t)) if t > 0.0 => Some((d / t).clamp(0.0, 1.0)),
                _ => None,
            };
            return YtDlpLine::Progress {
                fraction,
                speed: parse_num(f[3]),
            };
        }
    }
    YtDlpLine::Other
}

/// The name of a site as it is written in messages.
pub fn site_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Instagram => "Instagram",
        Platform::Twitter => "X",
        Platform::TikTok => "TikTok",
        Platform::YouTube => "YouTube",
    }
}

/// The message to show when a live stream was skipped instead of downloaded.
pub const LIVE_STREAM_MESSAGE: &str =
    "That's a live stream that hasn't ended yet. Download it once it is over.";

/// Turn yt-dlp's stderr into something a person can act on.
pub fn explain_error(stderr: &str, platform: Platform, used_cookies: bool) -> String {
    let lower = stderr.to_ascii_lowercase();
    let site = site_name(platform);
    let youtube = platform == Platform::YouTube;
    let noun = if youtube { "video" } else { "post" };
    let needs_login = lower.contains("login required")
        || lower.contains("log in")
        || lower.contains("sign in")
        || lower.contains("rate-limit")
        || lower.contains("cookies")
        || lower.contains("this content is only available")
        || lower.contains("private")
        || lower.contains("age-restricted")
        || lower.contains("nsfw");

    if lower.contains("unsupported url") {
        return "That link isn't a downloadable post.".into();
    }
    if is_no_video_error(stderr) {
        return "That post doesn't contain a video.".into();
    }
    if lower.contains("unable to download webpage")
        || lower.contains("urlopen error")
        || lower.contains("connection")
        || lower.contains("timed out")
    {
        return "Couldn't reach the site. Check your internet connection and try again.".into();
    }
    if youtube {
        if lower.contains("not a bot") {
            // YouTube asks this of some connections (often VPNs and shared networks).
            return if used_cookies {
                "YouTube is asking to confirm you're not a bot, even with your browser login. Open YouTube in that browser, make sure you're logged in, play any video once, and try again.".into()
            } else {
                "YouTube is asking to confirm you're not a bot, which it does for some connections. Open Settings → Downloads, pick the browser you're logged into YouTube with, and try again.".into()
            };
        }
        if lower.contains("will begin in")
            || lower.contains("premieres in")
            || lower.contains("premiere will begin")
        {
            return "That video hasn't started yet. Try again after it has begun.".into();
        }
        if lower.contains("members-only") || lower.contains("join this channel") {
            return "That video is for channel members only.".into();
        }
        if lower.contains("http error 429") || lower.contains("too many requests") {
            return "YouTube is limiting requests from your connection right now. Try again in a while."
                .into();
        }
        if lower.contains("requested format is not available") {
            return "YouTube didn't offer a format FillernCut can use for that video. Update the downloader (Settings → Downloads → Update now) and try again.".into();
        }
        if lower.contains("video unavailable")
            || lower.contains("this video is unavailable")
            || lower.contains("video is not available")
        {
            return "That video is unavailable. It may have been deleted, made private, or blocked in your country.".into();
        }
    }
    if needs_login {
        return if used_cookies {
            format!(
                "{site} wouldn't give out this {noun} even with your browser login. It may be private, deleted, or your login expired — open {site} in that browser and try again."
            )
        } else {
            format!(
                "{site} requires a login for this {noun}. Open Settings → Downloads and pick the browser you're logged into {site} with, then try again."
            )
        };
    }
    // Last resort: show the final error line, trimmed.
    stderr
        .lines()
        .rev()
        .find(|l| l.contains("ERROR"))
        .map(|l| strip_extractor_prefix(l.trim().trim_start_matches("ERROR:").trim()).to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "The download failed.".into())
}

/// `[youtube] dQw4w9WgXcQ: Some message` → `Some message`.
fn strip_extractor_prefix(message: &str) -> &str {
    let Some(rest) = message.strip_prefix('[') else {
        return message;
    };
    let Some((tag, rest)) = rest.split_once("] ") else {
        return message;
    };
    if tag.contains(' ') {
        return message;
    }
    match rest.split_once(": ") {
        Some((id, text)) if !id.contains(' ') && !text.is_empty() => text,
        _ => rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> DownloadOptions {
        DownloadOptions {
            url: "https://x.com/jack/status/20".into(),
            out_dir: "/Users/me/Movies/FillernCut".into(),
            file_stem: "2026-10-01_15-42-07".into(),
            ffmpeg_dir: Some("/App/Contents/MacOS".into()),
            cookies_browser: None,
        }
    }

    #[test]
    fn finds_checksums_by_exact_file_name() {
        let h1 = "a".repeat(64);
        let h2 = "B".repeat(64);
        let sums = format!("{h1}  yt-dlp\n{h2}  yt-dlp_macos\n{h1} *yt-dlp_macos_legacy\n");
        assert_eq!(checksum_for(&sums, "yt-dlp_macos"), Some("b".repeat(64)));
        assert_eq!(checksum_for(&sums, "yt-dlp_macos_legacy"), Some(h1));
        assert_eq!(checksum_for(&sums, "nope"), None);
        assert_eq!(checksum_for("short  yt-dlp_macos", "yt-dlp_macos"), None);
    }

    #[test]
    fn args_are_safe_and_minimal() {
        let a = build_args(&opts());
        assert!(a.contains(&"--no-playlist".to_string()));
        assert!(a.contains(&"--progress".to_string()));
        assert!(!a.contains(&"--cookies-from-browser".to_string()));
        // The URL goes last, after `--`, so a link can never be parsed as a flag.
        assert_eq!(a[a.len() - 2], "--");
        assert_eq!(a[a.len() - 1], "https://x.com/jack/status/20");
        let o = a.iter().position(|x| x == "-o").unwrap();
        assert_eq!(
            a[o + 1],
            "/Users/me/Movies/FillernCut/2026-10-01_15-42-07.%(ext)s"
        );
        let f = a.iter().position(|x| x == "--ffmpeg-location").unwrap();
        assert_eq!(a[f + 1], "/App/Contents/MacOS");
    }

    #[test]
    fn hostile_links_cannot_inject_flags() {
        let mut o = opts();
        o.url = "--exec=rm -rf ~".into();
        let a = build_args(&o);
        let dd = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(a[dd + 1], "--exec=rm -rf ~");
        assert_eq!(dd + 2, a.len());
    }

    #[test]
    fn cookies_flag_only_when_selected() {
        let mut o = opts();
        o.cookies_browser = Some(CookieBrowser::Safari);
        let a = build_args(&o);
        let i = a.iter().position(|x| x == "--cookies-from-browser").unwrap();
        assert_eq!(a[i + 1], "safari");
    }

    #[test]
    fn parses_progress_with_exact_and_estimated_totals() {
        assert_eq!(
            parse_line("FCPROG|500|1000|NA|2048.5"),
            YtDlpLine::Progress {
                fraction: Some(0.5),
                speed: Some(2048.5)
            }
        );
        assert_eq!(
            parse_line("FCPROG|250|NA|1000|NA"),
            YtDlpLine::Progress {
                fraction: Some(0.25),
                speed: None
            }
        );
        assert_eq!(
            parse_line("FCPROG|250|NA|NA|NA"),
            YtDlpLine::Progress {
                fraction: None,
                speed: None
            }
        );
    }

    #[test]
    fn parses_final_file_path_even_with_spaces() {
        assert_eq!(
            parse_line("FCFILE|/Users/me/Movies/Fillern Cut/twitter-20.mp4\n"),
            YtDlpLine::File("/Users/me/Movies/Fillern Cut/twitter-20.mp4".into())
        );
    }

    #[test]
    fn other_output_is_ignored() {
        assert_eq!(parse_line("[twitter] Extracting URL"), YtDlpLine::Other);
        assert_eq!(parse_line("FCPROG|broken"), YtDlpLine::Other);
    }

    #[test]
    fn login_errors_point_to_settings() {
        let msg = explain_error(
            "ERROR: [Instagram] C1a2: Login required. Use --cookies-from-browser",
            Platform::Instagram,
            false,
        );
        assert!(msg.contains("Settings"), "{msg}");
        assert!(msg.contains("Instagram"), "{msg}");
        let with_cookies = explain_error("ERROR: login required", Platform::Instagram, true);
        assert!(!with_cookies.contains("pick the browser"), "{with_cookies}");
    }

    #[test]
    fn network_and_unsupported_errors() {
        assert!(explain_error(
            "ERROR: Unable to download webpage: <urlopen error>",
            Platform::Twitter,
            false
        )
        .contains("internet"));
        assert!(
            explain_error("ERROR: Unsupported URL: https://x", Platform::Twitter, false)
                .contains("isn't a downloadable")
        );
    }

    #[test]
    fn info_args_ask_for_json_even_without_a_video() {
        let mut o = opts();
        o.cookies_browser = Some(CookieBrowser::Chrome);
        let a = build_info_args(&o);
        for flag in [
            "--ignore-no-formats-error",
            "--dump-single-json",
            "--no-download",
            "--write-pages",
        ] {
            assert!(a.contains(&flag.to_string()), "{flag}");
        }
        let i = a.iter().position(|x| x == "--cookies-from-browser").unwrap();
        assert_eq!(a[i + 1], "chrome");
        assert_eq!(a[a.len() - 2], "--");
        assert_eq!(a[a.len() - 1], "https://x.com/jack/status/20");
    }

    #[test]
    fn photo_posts_are_recognised_from_yt_dlps_errors() {
        assert!(is_no_video_error(
            "ERROR: [Instagram] C1: There is no video in this post"
        ));
        assert!(is_no_video_error(
            "ERROR: [Instagram] C1: No video formats found!; please report"
        ));
        assert!(is_no_video_error("ERROR: No video could be found in this tweet"));
        assert!(!is_no_video_error("ERROR: [Instagram] C1: Login required"));
        assert!(!is_no_video_error(""));
    }

    #[test]
    fn single_photo_uses_the_last_and_biggest_picture() {
        // What yt-dlp prints for a real Instagram photo: no sizes, smallest first.
        let json = r#"{"formats":[],"duration":null,"thumbnail":"https://cdn/x/full.jpg",
            "thumbnails":[{"id":"0","url":"https://cdn/x/s150.jpg"},{"id":"1","url":"https://cdn/x/s480.jpg"},{"id":"2","url":"https://cdn/x/full.jpg"}]}"#;
        assert_eq!(
            parse_photo_post(json),
            Some(vec!["https://cdn/x/full.jpg".to_string()])
        );
        // With sizes, the biggest wins whatever the order.
        let sized = r#"{"formats":[],"thumbnails":[
            {"url":"https://cdn/big.jpg","width":1080,"height":1350},
            {"url":"https://cdn/small.jpg","width":320,"height":400}]}"#;
        assert_eq!(
            parse_photo_post(sized),
            Some(vec!["https://cdn/big.jpg".to_string()])
        );
    }

    #[test]
    fn carousel_gives_one_picture_per_slide_in_order() {
        let json = r#"{"_type":"playlist","entries":[
            {"formats":[],"thumbnails":[{"url":"https://cdn/a1.jpg"},{"url":"https://cdn/a2.jpg"}]},
            {"formats":[],"thumbnails":[{"url":"https://cdn/b1.jpg"},{"url":"https://cdn/b2.jpg"}]}]}"#;
        assert_eq!(
            parse_photo_post(json),
            Some(vec![
                "https://cdn/a2.jpg".to_string(),
                "https://cdn/b2.jpg".to_string()
            ])
        );
    }

    #[test]
    fn videos_and_unusable_answers_are_not_photo_posts() {
        let pic = r#"{"url":"https://cdn/c.jpg"}"#;
        // A reel that yt-dlp couldn't get formats for still has a duration.
        let reel = format!(r#"{{"formats":[],"duration":12.5,"thumbnails":[{pic}]}}"#);
        assert_eq!(parse_photo_post(&reel), None);
        let with_formats = format!(r#"{{"formats":[{{"url":"https://v"}}],"thumbnails":[{pic}]}}"#);
        assert_eq!(parse_photo_post(&with_formats), None);
        // One video among the photos of a carousel: don't drop it silently.
        let mixed = format!(
            r#"{{"entries":[{{"formats":[],"thumbnails":[{pic}]}},{{"duration":4,"formats":[],"thumbnails":[{pic}]}}]}}"#
        );
        assert_eq!(parse_photo_post(&mixed), None);
        // A failed slide, no pictures at all, links that aren't web links, junk.
        assert_eq!(parse_photo_post(r#"{"entries":[null]}"#), None);
        assert_eq!(parse_photo_post(r#"{"entries":[]}"#), None);
        assert_eq!(parse_photo_post(r#"{"formats":[],"thumbnails":[]}"#), None);
        assert_eq!(
            parse_photo_post(r#"{"formats":[],"thumbnails":[{"url":"file:///etc/passwd"}]}"#),
            None
        );
        assert_eq!(parse_photo_post("not json"), None);
    }

    fn music_answer(extra: &str) -> String {
        format!(
            r#"{{"items":[{{"code":"C1","media_type":8,"music_metadata":{{"music_canonical_id":"1",
            "music_info":{{"music_asset_info":{{"title":"Song","duration_in_ms":180000,
              "progressive_download_url":"https://cdn/audio/full.m4a",
              "fast_start_progressive_download_url":"https://cdn/audio/fast.m4a"}},
            "music_consumption_info":{{{extra}}}}}}}}}]}}"#
        )
    }

    #[test]
    fn finds_the_music_of_a_photo_post_and_the_part_it_plays() {
        let page = music_answer(r#""audio_asset_start_time_in_ms":42000,"overlap_duration_in_ms":20000"#);
        assert_eq!(
            parse_post_music(&page),
            Some(PostMusic {
                url: "https://cdn/audio/full.m4a".into(),
                start: 42.0,
                length: Some(20.0)
            })
        );
        // Without a stated stretch the whole track is meant.
        assert_eq!(
            parse_post_music(&music_answer("")),
            Some(PostMusic {
                url: "https://cdn/audio/full.m4a".into(),
                start: 0.0,
                length: None
            })
        );
        // A nonsense stretch is ignored rather than trusted.
        let odd = music_answer(r#""audio_asset_start_time_in_ms":-5,"overlap_duration_in_ms":0"#);
        let m = parse_post_music(&odd).unwrap();
        assert_eq!((m.start, m.length), (0.0, None));
    }

    #[test]
    fn music_falls_back_to_the_fast_start_link_and_to_reel_style_metadata() {
        let fast = r#"{"music_metadata":{"music_info":{"music_asset_info":
            {"fast_start_progressive_download_url":"https://cdn/fast.m4a"}}}}"#;
        assert_eq!(parse_post_music(fast).unwrap().url, "https://cdn/fast.m4a");
        let clips = r#"{"media":{"clips_metadata":{"music_info":{"music_asset_info":
            {"progressive_download_url":"https://cdn/clip.m4a"}}}}}"#;
        assert_eq!(parse_post_music(clips).unwrap().url, "https://cdn/clip.m4a");
    }

    #[test]
    fn posts_without_music_or_with_unusable_answers_have_none() {
        assert_eq!(
            parse_post_music(r#"{"items":[{"music_metadata":null,"clips_metadata":null}]}"#),
            None
        );
        assert_eq!(parse_post_music(r#"{"items":[{"media_type":1}]}"#), None);
        // Music is listed but there is nothing to download, or not over the web.
        assert_eq!(
            parse_post_music(r#"{"music_metadata":{"music_info":{"music_asset_info":{"title":"x"}}}}"#),
            None
        );
        assert_eq!(
            parse_post_music(
                r#"{"music_metadata":{"music_info":{"music_asset_info":{"progressive_download_url":"file:///etc/passwd"}}}}"#
            ),
            None
        );
        // The logged-out web page is HTML, not JSON.
        assert_eq!(parse_post_music("<html><script>{}</script></html>"), None);
        assert_eq!(parse_post_music(""), None);
    }

    #[test]
    fn the_format_choice_never_goes_above_1080p() {
        let a = build_args(&opts());
        let f = a.iter().position(|x| x == "-f").unwrap();
        assert!(
            a[f + 1].starts_with("bv*[vcodec^=avc1][height<=?1080]+ba[ext=m4a]/"),
            "{}",
            a[f + 1]
        );
        assert!(
            a[f + 1].ends_with("/b"),
            "a last resort that takes whatever there is"
        );
        assert_eq!(
            a[f + 1].matches("[height<=?1080]").count(),
            3,
            "three capped choices, then the uncapped last resort"
        );
    }

    #[test]
    fn live_streams_are_reported_and_skipped() {
        let a = build_args(&opts());
        let m = a.iter().position(|x| x == "--match-filters").unwrap();
        assert_eq!(a[m + 1], "!is_live");
        assert!(a.contains(&"pre_process:FCLIVE|%(live_status)s".to_string()));
        assert_eq!(
            parse_line("FCLIVE|is_live"),
            YtDlpLine::LiveStatus("is_live".into())
        );
        assert_eq!(
            parse_line("FCLIVE|not_live\n"),
            YtDlpLine::LiveStatus("not_live".into())
        );
    }

    #[test]
    fn site_names_read_well_in_messages() {
        assert_eq!(site_name(Platform::YouTube), "YouTube");
        assert_eq!(site_name(Platform::Twitter), "X");
    }

    #[test]
    fn youtube_errors_say_what_to_do() {
        let yt = |text: &str, cookies: bool| explain_error(text, Platform::YouTube, cookies);
        let bot = "ERROR: [youtube] abc: Sign in to confirm you're not a bot. Use --cookies-from-browser or --cookies";
        let msg = yt(bot, false);
        assert!(msg.contains("not a bot") && msg.contains("Settings"), "{msg}");
        assert!(
            yt(bot, true).contains("even with your browser login"),
            "{}",
            yt(bot, true)
        );
        // The curly apostrophe YouTube really uses is handled as well.
        assert!(yt(
            "ERROR: [youtube] abc: Sign in to confirm you\u{2019}re not a bot.",
            false
        )
        .contains("not a bot"));

        for (text, wanted) in [
            ("ERROR: [youtube] abc: This video is unavailable", "unavailable"),
            (
                "ERROR: [youtube] abc: Video unavailable. This video is no longer available",
                "unavailable",
            ),
            ("ERROR: [youtube] abc: Premieres in 5 hours", "hasn't started"),
            (
                "ERROR: [youtube] abc: This live event will begin in 2 days",
                "hasn't started",
            ),
            (
                "ERROR: [youtube] abc: Join this channel to get access to members-only content",
                "channel members",
            ),
            (
                "ERROR: [youtube] abc: HTTP Error 429: Too Many Requests",
                "limiting requests",
            ),
            (
                "ERROR: [youtube] abc: Requested format is not available",
                "Update now",
            ),
        ] {
            assert!(yt(text, false).contains(wanted), "{text} -> {}", yt(text, false));
        }
        // Login problems talk about a video, not a post.
        let age = yt(
            "ERROR: [youtube] abc: Sign in to confirm your age. This video may be inappropriate",
            false,
        );
        assert!(age.contains("YouTube requires a login for this video"), "{age}");
        assert!(yt(
            "ERROR: [youtube] abc: Private video. Sign in if you've been granted access",
            true
        )
        .contains("this video even with"));
    }

    #[test]
    fn other_sites_keep_their_wording() {
        let msg = explain_error("ERROR: login required", Platform::TikTok, false);
        assert!(msg.contains("TikTok requires a login for this post"), "{msg}");
        // YouTube's own errors only apply to YouTube.
        assert_eq!(
            explain_error("ERROR: Video unavailable", Platform::Instagram, false),
            "Video unavailable"
        );
    }

    #[test]
    fn unknown_errors_lose_the_extractor_prefix() {
        assert_eq!(
            explain_error(
                "ERROR: [youtube] dQw4w9WgXcQ: Something odd happened",
                Platform::YouTube,
                false
            ),
            "Something odd happened"
        );
        assert_eq!(
            strip_extractor_prefix("[generic] Unable to find it"),
            "Unable to find it"
        );
        assert_eq!(strip_extractor_prefix("[not a tag] x: y"), "[not a tag] x: y");
        assert_eq!(strip_extractor_prefix("no prefix here"), "no prefix here");
        assert_eq!(strip_extractor_prefix("[youtube] id: "), "id: ");
    }

    #[test]
    fn unknown_errors_show_the_last_error_line() {
        let msg = explain_error("noise\nERROR: Something odd happened\n", Platform::TikTok, false);
        assert_eq!(msg, "Something odd happened");
        assert_eq!(explain_error("", Platform::TikTok, false), "The download failed.");
    }
}
