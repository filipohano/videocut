//! TikTok: interpreting the response of the watermark-free download API.
//!
//! The network call itself lives in the app crate; this module only turns the
//! JSON into a typed result so the parsing can be unit-tested offline.

use serde_json::Value;
use thiserror::Error;

pub const API_HOST: &str = "https://www.tikwm.com";

#[derive(Debug, Error, PartialEq)]
pub enum TikTokError {
    #[error("TikTok service said: {0}")]
    Api(String),
    #[error("Unexpected response from the TikTok service")]
    BadResponse,
    #[error("This post has no downloadable video or photos")]
    NothingToDownload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TikTokMedia {
    Video {
        id: String,
        url: String,
        title: Option<String>,
    },
    /// A photo post: stills plus the post's sound.
    Photos {
        id: String,
        images: Vec<String>,
        music: Option<String>,
        title: Option<String>,
    },
}

impl TikTokMedia {
    pub fn id(&self) -> &str {
        match self {
            TikTokMedia::Video { id, .. } | TikTokMedia::Photos { id, .. } => id,
        }
    }
}

fn absolute(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else if url.starts_with('/') {
        format!("{API_HOST}{url}")
    } else {
        url.to_string()
    }
}

fn non_empty(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(absolute)
}

/// Query URL for the API on `host`. `hd=1` asks for the original-quality file.
pub fn api_query_url(host: &str, tiktok_url: &str) -> String {
    let mut u = url::Url::parse(&format!("{}/api/", host.trim_end_matches('/'))).expect("valid api host");
    u.query_pairs_mut()
        .append_pair("url", tiktok_url)
        .append_pair("hd", "1");
    u.to_string()
}

pub fn parse_response(json: &str) -> Result<TikTokMedia, TikTokError> {
    let root: Value = serde_json::from_str(json).map_err(|_| TikTokError::BadResponse)?;
    let code = root.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = root
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(TikTokError::Api(msg.to_string()));
    }
    let data = root.get("data").ok_or(TikTokError::BadResponse)?;
    let id = data
        .get("id")
        .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
        .ok_or(TikTokError::BadResponse)?;
    let title = data
        .get("title")
        .and_then(|t| t.as_str())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    let images: Vec<String> = data
        .get("images")
        .and_then(|i| i.as_array())
        .map(|a| a.iter().filter_map(|v| non_empty(Some(v))).collect())
        .unwrap_or_default();
    if !images.is_empty() {
        return Ok(TikTokMedia::Photos {
            id,
            images,
            music: non_empty(data.get("music")),
            title,
        });
    }

    // `hdplay` is the original upload; `play` is the re-encoded fallback.
    // Both are free of the TikTok watermark (`wmplay` is the one that has it).
    match non_empty(data.get("hdplay")).or_else(|| non_empty(data.get("play"))) {
        Some(url) => Ok(TikTokMedia::Video { id, url, title }),
        None => Err(TikTokError::NothingToDownload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_hd_without_watermark() {
        let json = r#"{"code":0,"msg":"success","data":{
            "id":"6718335390845095173","title":" Funny cat ",
            "play":"https://cdn.example/play.mp4","hdplay":"https://cdn.example/hd.mp4",
            "wmplay":"https://cdn.example/wm.mp4","images":null}}"#;
        assert_eq!(
            parse_response(json),
            Ok(TikTokMedia::Video {
                id: "6718335390845095173".into(),
                url: "https://cdn.example/hd.mp4".into(),
                title: Some("Funny cat".into()),
            })
        );
    }

    #[test]
    fn falls_back_to_play_and_resolves_relative_urls() {
        let json = r#"{"code":0,"data":{"id":"1","play":"/video/media/play/1.mp4","hdplay":""}}"#;
        assert_eq!(
            parse_response(json),
            Ok(TikTokMedia::Video {
                id: "1".into(),
                url: "https://www.tikwm.com/video/media/play/1.mp4".into(),
                title: None,
            })
        );
    }

    #[test]
    fn photo_posts_return_stills_and_music() {
        let json = r#"{"code":0,"data":{"id":"7345678901234567890",
            "images":["https://cdn.example/1.jpeg","https://cdn.example/2.jpeg"],
            "music":"https://cdn.example/music.mp3","play":"https://cdn.example/slideshow.mp4"}}"#;
        match parse_response(json).unwrap() {
            TikTokMedia::Photos {
                images, music, id, ..
            } => {
                assert_eq!(id, "7345678901234567890");
                assert_eq!(images.len(), 2);
                assert_eq!(music.as_deref(), Some("https://cdn.example/music.mp3"));
            }
            other => panic!("expected photos, got {other:?}"),
        }
    }

    #[test]
    fn numeric_ids_are_accepted() {
        let json = r#"{"code":0,"data":{"id":6718335390845095173,"play":"https://x/y.mp4"}}"#;
        assert_eq!(parse_response(json).unwrap().id(), "6718335390845095173");
    }

    #[test]
    fn api_errors_surface_their_message() {
        assert_eq!(
            parse_response(r#"{"code":-1,"msg":"Free Api Limit: 1 request/second."}"#),
            Err(TikTokError::Api("Free Api Limit: 1 request/second.".into()))
        );
    }

    #[test]
    fn garbage_and_empty_posts() {
        assert_eq!(parse_response("<html>"), Err(TikTokError::BadResponse));
        assert_eq!(
            parse_response(r#"{"code":0,"data":{"id":"1"}}"#),
            Err(TikTokError::NothingToDownload)
        );
    }

    #[test]
    fn query_url_is_encoded() {
        let u = api_query_url(API_HOST, "https://www.tiktok.com/@a/video/1?x=1&y=2");
        assert!(u.starts_with("https://www.tikwm.com/api/?url=https%3A%2F%2Fwww.tiktok.com"));
        assert!(u.ends_with("&hd=1"));
    }
}
