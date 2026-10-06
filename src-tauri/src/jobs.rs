//! Running child processes (ffmpeg, yt-dlp) with progress events and cancellation.

use fillerncut_core::progress::{fraction, parse_progress_line, FfmpegProgress};
use serde::Serialize;
use std::collections::VecDeque;
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::bins;

/// The error string the frontend recognises as "the user cancelled".
pub const CANCELLED: &str = "Cancelled";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job: String,
    /// 0..=1, or `None` when indeterminate.
    pub fraction: Option<f64>,
    pub message: Option<String>,
}

pub fn emit_progress<R: Runtime>(
    app: &AppHandle<R>,
    job: &str,
    fraction: Option<f64>,
    message: Option<&str>,
) {
    let _ = app.emit(
        "job-progress",
        JobProgress {
            job: job.to_string(),
            fraction,
            message: message.map(String::from),
        },
    );
}

/// Keep only the last `keep` lines of a stream (ffmpeg's real error is at the end).
pub async fn collect_tail<R: AsyncRead + Unpin>(reader: R, keep: usize) -> String {
    let mut lines = BufReader::new(reader).lines();
    let mut tail: VecDeque<String> = VecDeque::with_capacity(keep);
    while let Ok(Some(line)) = lines.next_line().await {
        if tail.len() == keep {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    tail.into_iter().collect::<Vec<_>>().join("\n")
}

/// Last few meaningful lines of ffmpeg's stderr, for the error toast.
pub fn summarize_ffmpeg_error(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(3);
    let msg = lines[start..].join(" · ");
    if msg.is_empty() {
        "ffmpeg failed without an error message".into()
    } else {
        msg
    }
}

/// Run ffmpeg with `-progress pipe:1`, calling `on_progress(0..=1)` as it goes.
pub async fn run_ffmpeg(
    token: &CancellationToken,
    args: &[String],
    total_seconds: f64,
    on_progress: impl Fn(f64),
) -> Result<(), String> {
    let mut child = bins::hide_window(&mut Command::new(bins::ffmpeg()))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Couldn't start ffmpeg: {e}"))?;

    let stdout = child.stdout.take().ok_or("no ffmpeg stdout")?;
    let stderr = child.stderr.take().ok_or("no ffmpeg stderr")?;
    let stderr_task = tokio::spawn(collect_tail(stderr, 40));
    let mut lines = BufReader::new(stdout).lines();

    on_progress(0.0);
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = child.kill().await;
                return Err(CANCELLED.into());
            }
            line = lines.next_line() => match line {
                Ok(Some(l)) => {
                    if let Some(FfmpegProgress::Time(t)) = parse_progress_line(&l) {
                        on_progress(fraction(t, total_seconds));
                    }
                }
                Ok(None) => break,
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let tail = stderr_task.await.unwrap_or_default();
    if status.success() {
        on_progress(1.0);
        Ok(())
    } else {
        Err(summarize_ffmpeg_error(&tail))
    }
}

/// Run a short command and return its stderr (ffmpeg prints filter info there), whatever the exit code.
pub async fn capture_stderr(program: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = bins::hide_window(&mut Command::new(program))
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Couldn't start {}: {e}", program.display()))?;
    Ok(String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Run a short command to completion and capture stdout (ffprobe, `ffmpeg -encoders`, `yt-dlp --version`).
pub async fn capture(program: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = bins::hide_window(&mut Command::new(program))
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Couldn't start {}: {e}", program.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(summarize_ffmpeg_error(&String::from_utf8_lossy(&out.stderr)))
    }
}
