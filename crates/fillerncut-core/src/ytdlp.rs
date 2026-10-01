//! yt-dlp: command line, output parsing and friendly error messages.

use crate::links::Platform;
use crate::settings::CookieBrowser;
use std::path::PathBuf;

/// Line prefixes we ask yt-dlp to emit so they can't be confused with its
/// normal chatter.
const PROGRESS_PREFIX: &str = "FCPROG|";
const FILE_PREFIX: &str = "FCFILE|";

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub url: String,
    pub out_dir: PathBuf,
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
        // Prefer H.264/AAC so the file plays in the preview and edits quickly,
        // then fall back to whatever is best.
        "-f".into(),
        "bv*[vcodec^=avc1]+ba[ext=m4a]/bv*+ba/b".into(),
        "--merge-output-format".into(),
        "mp4".into(),
        "-o".into(),
        opts.out_dir
            .join("%(extractor)s-%(id)s.%(ext)s")
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

/// Turn yt-dlp's stderr into something a person can act on.
pub fn explain_error(stderr: &str, platform: Platform, used_cookies: bool) -> String {
    let lower = stderr.to_ascii_lowercase();
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
    if lower.contains("no video could be found") || lower.contains("there is no video in this post") {
        return "That post doesn't contain a video.".into();
    }
    if lower.contains("unable to download webpage")
        || lower.contains("urlopen error")
        || lower.contains("connection")
        || lower.contains("timed out")
    {
        return "Couldn't reach the site. Check your internet connection and try again.".into();
    }
    if needs_login {
        let site = match platform {
            Platform::Instagram => "Instagram",
            Platform::Twitter => "X",
            Platform::TikTok => "TikTok",
        };
        return if used_cookies {
            format!(
                "{site} wouldn't give out this post even with your browser login. It may be private, deleted, or your login expired — open {site} in that browser and try again."
            )
        } else {
            format!(
                "{site} requires a login for this post. Open Settings → Downloads and pick the browser you're logged into {site} with, then try again."
            )
        };
    }
    // Last resort: show the final error line, trimmed.
    stderr
        .lines()
        .rev()
        .find(|l| l.contains("ERROR"))
        .map(|l| l.trim().trim_start_matches("ERROR:").trim().to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "The download failed.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> DownloadOptions {
        DownloadOptions {
            url: "https://x.com/jack/status/20".into(),
            out_dir: "/Users/me/Movies/FillernCut".into(),
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
            "/Users/me/Movies/FillernCut/%(extractor)s-%(id)s.%(ext)s"
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
    fn unknown_errors_show_the_last_error_line() {
        let msg = explain_error("noise\nERROR: Something odd happened\n", Platform::TikTok, false);
        assert_eq!(msg, "Something odd happened");
        assert_eq!(explain_error("", Platform::TikTok, false), "The download failed.");
    }
}
