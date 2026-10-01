//! Parsing of `ffmpeg -progress pipe:1` output.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FfmpegProgress {
    /// Seconds of output produced so far.
    Time(f64),
    End,
}

/// Parse one `key=value` line of ffmpeg's machine-readable progress stream.
pub fn parse_progress_line(line: &str) -> Option<FfmpegProgress> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        // Both are in microseconds (`out_time_ms` is a long-standing misnomer).
        "out_time_us" | "out_time_ms" => {
            let us: i64 = value.trim().parse().ok()?;
            Some(FfmpegProgress::Time((us.max(0) as f64) / 1_000_000.0))
        }
        "progress" if value.trim() == "end" => Some(FfmpegProgress::End),
        _ => None,
    }
}

/// Fraction (0..=1) of a job that is done.
pub fn fraction(done_seconds: f64, total_seconds: f64) -> f64 {
    if total_seconds <= 0.0 {
        return 0.0;
    }
    (done_seconds / total_seconds).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_time_and_end() {
        assert_eq!(
            parse_progress_line("out_time_us=2500000"),
            Some(FfmpegProgress::Time(2.5))
        );
        assert_eq!(
            parse_progress_line("out_time_ms=1000000\n"),
            Some(FfmpegProgress::Time(1.0))
        );
        assert_eq!(parse_progress_line("progress=end"), Some(FfmpegProgress::End));
    }

    #[test]
    fn ignores_everything_else() {
        assert_eq!(parse_progress_line("progress=continue"), None);
        assert_eq!(parse_progress_line("out_time_us=N/A"), None);
        assert_eq!(parse_progress_line("bitrate=1234.5kbits/s"), None);
        assert_eq!(parse_progress_line("garbage"), None);
    }

    #[test]
    fn negative_start_times_clamp_to_zero() {
        assert_eq!(
            parse_progress_line("out_time_us=-33333"),
            Some(FfmpegProgress::Time(0.0))
        );
    }

    #[test]
    fn fraction_is_clamped() {
        assert_eq!(fraction(5.0, 10.0), 0.5);
        assert_eq!(fraction(20.0, 10.0), 1.0);
        assert_eq!(fraction(1.0, 0.0), 0.0);
    }
}
