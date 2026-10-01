//! Parsing of `ffprobe` and `ffmpeg -encoders` output.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// Width as *displayed* (rotation metadata already applied) — this is the
    /// coordinate space the crop rectangle lives in, because ffmpeg
    /// auto-rotates before filters run.
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: Option<f64>,
    /// Video stream bitrate in bits/s (estimated from the container when the stream doesn't say).
    pub bitrate: Option<u64>,
    pub video_codec: Option<String>,
    pub has_audio: bool,
    pub audio_codec: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EncoderSupport {
    pub videotoolbox: bool,
    pub libx264: bool,
}

pub fn parse_encoders(ffmpeg_encoders_output: &str) -> EncoderSupport {
    let mut s = EncoderSupport::default();
    for line in ffmpeg_encoders_output.lines() {
        let mut parts = line.split_whitespace();
        let (Some(flags), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        // Encoder rows start with a 6-char flag column such as "V....D".
        if flags.len() != 6 || !flags.starts_with('V') {
            continue;
        }
        match name {
            "h264_videotoolbox" => s.videotoolbox = true,
            "libx264" => s.libx264 = true,
            _ => {}
        }
    }
    s
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

fn rotation_of(stream: &Value) -> i64 {
    if let Some(side) = stream.get("side_data_list").and_then(|v| v.as_array()) {
        for entry in side {
            if let Some(r) = entry.get("rotation").and_then(|r| r.as_f64()) {
                return r.round() as i64;
            }
        }
    }
    stream
        .pointer("/tags/rotate")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// Parse `ffprobe -print_format json -show_streams -show_format`.
pub fn parse_ffprobe(json: &str) -> Result<MediaInfo, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("bad ffprobe output: {e}"))?;
    let streams = root
        .get("streams")
        .and_then(|s| s.as_array())
        .ok_or("ffprobe returned no streams")?;

    let video = streams
        .iter()
        .find(|s| {
            s.get("codec_type").and_then(|c| c.as_str()) == Some("video")
                // Cover art / thumbnails are flagged as attached pictures.
                && s.pointer("/disposition/attached_pic").and_then(|v| v.as_i64()) != Some(1)
        })
        .ok_or("This file has no video stream")?;
    let audio = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("audio"));

    let (mut width, mut height) = (
        video.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        video.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
    );
    if width == 0 || height == 0 {
        return Err("Couldn't read the video dimensions".into());
    }
    if rotation_of(video).rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }

    let parse_f = |v: Option<&Value>| -> Option<f64> {
        v.and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()))
    };
    let duration = parse_f(root.pointer("/format/duration"))
        .or_else(|| parse_f(video.get("duration")))
        .unwrap_or(0.0);

    let bit_rate = |v: Option<&Value>| -> Option<u64> { parse_f(v).filter(|b| *b > 0.0).map(|b| b as u64) };
    let bitrate = bit_rate(video.get("bit_rate")).or_else(|| {
        // Streams often omit it (e.g. WebM, some MKV): take the container's total
        // and subtract the audio stream.
        let total = bit_rate(root.pointer("/format/bit_rate"))?;
        let audio_rate = audio.and_then(|a| bit_rate(a.get("bit_rate"))).unwrap_or(0);
        Some(if audio_rate > 0 {
            total.saturating_sub(audio_rate).max(total / 10)
        } else if audio.is_some() {
            total * 9 / 10
        } else {
            total
        })
    });

    Ok(MediaInfo {
        width,
        height,
        duration,
        bitrate,
        fps: video
            .get("avg_frame_rate")
            .and_then(|v| v.as_str())
            .and_then(parse_rate)
            .or_else(|| {
                video
                    .get("r_frame_rate")
                    .and_then(|v| v.as_str())
                    .and_then(parse_rate)
            }),
        video_codec: video.get("codec_name").and_then(|v| v.as_str()).map(String::from),
        has_audio: audio.is_some(),
        audio_codec: audio
            .and_then(|a| a.get("codec_name"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT_PHONE: &str = r#"{
      "streams": [
        {"codec_type":"video","codec_name":"hevc","width":1920,"height":1080,
         "avg_frame_rate":"30000/1001","disposition":{"attached_pic":0},
         "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]},
        {"codec_type":"audio","codec_name":"aac","bit_rate":"128000"}
      ],
      "format": {"duration":"9.700000","bit_rate":"2600000"}
    }"#;

    #[test]
    fn rotated_video_reports_displayed_size() {
        let info = parse_ffprobe(PORTRAIT_PHONE).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        assert!((info.duration - 9.7).abs() < 1e-9);
        assert!(info.has_audio);
        assert_eq!(info.audio_codec.as_deref(), Some("aac"));
        assert_eq!(info.video_codec.as_deref(), Some("hevc"));
        // No stream bitrate: container total minus the audio stream.
        assert_eq!(info.bitrate, Some(2_472_000));
        assert!((info.fps.unwrap() - 29.97).abs() < 0.01);
    }

    #[test]
    fn legacy_rotate_tag_is_honoured() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":640,"height":360,
            "tags":{"rotate":"270"}}],"format":{"duration":"2"}}"#;
        let info = parse_ffprobe(json).unwrap();
        assert_eq!((info.width, info.height), (360, 640));
        assert!(!info.has_audio);
    }

    #[test]
    fn attached_pictures_are_not_the_video_stream() {
        let json = r#"{"streams":[
            {"codec_type":"video","codec_name":"mjpeg","width":500,"height":500,"disposition":{"attached_pic":1}},
            {"codec_type":"audio","codec_name":"mp3"}],"format":{"duration":"3"}}"#;
        assert!(parse_ffprobe(json).is_err());
    }

    #[test]
    fn stream_bitrate_wins_and_missing_bitrate_is_none() {
        let with = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":10,"height":10,"bit_rate":"1500000"}],"format":{"duration":"1","bit_rate":"9"}}"#;
        assert_eq!(parse_ffprobe(with).unwrap().bitrate, Some(1_500_000));
        let none = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":10,"height":10}],"format":{"duration":"1"}}"#;
        assert_eq!(parse_ffprobe(none).unwrap().bitrate, None);
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_ffprobe("nope").is_err());
        assert!(parse_ffprobe(r#"{"streams":[]}"#).is_err());
    }

    #[test]
    fn encoder_detection() {
        let out = " V....D h264_videotoolbox   VideoToolbox H.264 Encoder (codec h264)\n \
                   V....D libx264              libx264 H.264 / AVC\n \
                   A....D aac                  AAC\n";
        assert_eq!(
            parse_encoders(out),
            EncoderSupport {
                videotoolbox: true,
                libx264: true
            }
        );
        assert_eq!(parse_encoders("A....D aac AAC"), EncoderSupport::default());
    }
}
