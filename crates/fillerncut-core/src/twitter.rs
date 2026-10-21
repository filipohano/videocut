//! X/Twitter posts made of pictures.
//!
//! yt-dlp only knows videos: for a tweet with just pictures it stops with "No video could
//! be found in this tweet" and doesn't say where the pictures are. Twitter's own embed endpoint
//! (`cdn.syndication.twimg.com/tweet-result`, what an embedded tweet on a web page uses) does,
//! for public tweets and without a login. This module builds that request and reads the answer.

use serde_json::Value;
use thiserror::Error;
use url::Url;

/// The post number in a normalised `https://x.com/<user>/status/<id>` link.
pub fn status_id(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let mut segments = url.path_segments()?;
    segments.find(|s| *s == "status" || *s == "statuses")?;
    let id = segments.next()?;
    (!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string())
}

/// The `token` the embed endpoint wants. It only checks that one is present, but this is what
/// Twitter's own embed sends: `((Number(id) / 1e15) * Math.PI).toString(36)` without zeros and dots.
pub fn syndication_token(id: &str) -> String {
    let value = id.parse::<f64>().unwrap_or(0.0) / 1e15 * std::f64::consts::PI;
    js_to_string_radix36(value)
        .chars()
        .filter(|c| *c != '0' && *c != '.')
        .collect()
}

/// `value.toString(36)` as JavaScript prints it for a positive number of moderate size (V8's
/// `DoubleToRadixCString`): the fraction is written only as far as the number's precision goes.
fn js_to_string_radix36(value: f64) -> String {
    const CHARS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    const RADIX: f64 = 36.0;
    let mut integer = value.floor();
    let mut fraction = value - integer;
    let next_up = f64::from_bits(value.to_bits() + 1);
    let mut delta = (0.5 * (next_up - value)).max(f64::from_bits(1));
    let mut digits: Vec<u8> = Vec::new();

    if fraction >= delta {
        loop {
            fraction *= RADIX;
            delta *= RADIX;
            let digit = fraction as usize;
            digits.push(CHARS[digit]);
            fraction -= digit as f64;
            // Round to even, carrying into the digits already written.
            if (fraction > 0.5 || (fraction == 0.5 && digit % 2 == 1)) && fraction + delta > 1.0 {
                loop {
                    let Some(last) = digits.pop() else {
                        integer += 1.0;
                        break;
                    };
                    let d = if last > b'9' {
                        last - b'a' + 10
                    } else {
                        last - b'0'
                    } as usize;
                    if d + 1 < 36 {
                        digits.push(CHARS[d + 1]);
                        break;
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }

    let mut whole = Vec::new();
    loop {
        let remainder = integer % RADIX;
        whole.push(CHARS[remainder as usize]);
        integer = (integer - remainder) / RADIX;
        if integer <= 0.0 {
            break;
        }
    }
    whole.reverse();
    let mut out = String::from_utf8(whole).unwrap_or_default();
    if !digits.is_empty() {
        out.push('.');
        out.push_str(&String::from_utf8(digits).unwrap_or_default());
    }
    out
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TweetPhotosError {
    /// Deleted, protected, marked sensitive, or an answer that isn't a tweet.
    #[error("the tweet isn't available")]
    Unavailable,
    /// A tweet that is available but has no pictures (or has a video, which is yt-dlp's job).
    #[error("the tweet has no pictures")]
    NoPhotos,
}

const MAX_PHOTOS: usize = 10;
const IMAGE_HOST: &str = "pbs.twimg.com";

/// The pictures of a tweet at full size, in order, from the embed endpoint's answer.
pub fn parse_tweet_photos(json: &str) -> Result<Vec<String>, TweetPhotosError> {
    let tweet: Value = serde_json::from_str(json).map_err(|_| TweetPhotosError::Unavailable)?;
    // For tweets it won't show the endpoint answers `{}`, or a "tombstone".
    if tweet.get("id_str").is_none()
        || tweet.get("__typename").and_then(Value::as_str) == Some("TweetTombstone")
    {
        return Err(TweetPhotosError::Unavailable);
    }

    let details = tweet.get("mediaDetails").and_then(Value::as_array);
    // A video or GIF: yt-dlp handles those, so a picture can't stand in for it.
    let has_other_media = details.into_iter().flatten().any(|m| {
        m.get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t != "photo")
    });
    if has_other_media {
        return Err(TweetPhotosError::NoPhotos);
    }
    let mut urls: Vec<&str> = details
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("media_url_https").and_then(Value::as_str))
        .collect();
    if urls.is_empty() {
        urls = tweet
            .get("photos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.get("url").and_then(Value::as_str))
            .collect();
    }

    let mut photos: Vec<String> = Vec::new();
    for raw in urls {
        if let Some(full) = original_size(raw) {
            if !photos.contains(&full) {
                photos.push(full);
            }
        }
        if photos.len() >= MAX_PHOTOS {
            break;
        }
    }
    if photos.is_empty() {
        Err(TweetPhotosError::NoPhotos)
    } else {
        Ok(photos)
    }
}

/// `https://pbs.twimg.com/media/<id>.jpg` → the same with `?name=orig`, which is the original
/// size (without it Twitter serves a small copy). Only Twitter's own picture host is accepted.
fn original_size(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https" || url.host_str() != Some(IMAGE_HOST) || !url.path().starts_with("/media/") {
        return None;
    }
    url.set_query(Some("name=orig"));
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_post_number() {
        assert_eq!(status_id("https://x.com/jack/status/20").as_deref(), Some("20"));
        assert_eq!(
            status_id("https://x.com/i/web/status/1788000000000000000").as_deref(),
            Some("1788000000000000000")
        );
        assert_eq!(
            status_id("https://x.com/jack/statuses/20?s=1").as_deref(),
            Some("20")
        );
        for bad in [
            "https://x.com/jack",
            "https://x.com/jack/status/",
            "https://x.com/jack/status/abc",
            "https://x.com/jack/status/123456789012345678901",
            "not a url",
        ] {
            assert_eq!(status_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_token_is_what_javascript_makes() {
        // Values printed by `((Number(id) / 1e15) * Math.PI).toString(36).replace(/(0+|\.)/g, "")`.
        for (id, token) in [
            ("440322224407314432", "12fb9qdo78ad"),
            ("20", "6dq1a2xwd93"),
            ("1", "bhi2ay3f28n"),
            ("1000000000000000000", "2f9lc2ug9mm"),
            ("1788000000000000000", "4c161ak6c14"),
            ("1920000000000000000", "4njuvtxzjxc"),
            ("2000000000000000000", "4uj6o5owj98"),
            ("1234567890123456789", "2zqic77uqyk"),
            ("1500000000000000001", "3mwe49oefy"),
            ("999999999999999999", "2f9lc2ug9mm"),
        ] {
            assert_eq!(syndication_token(id), token, "{id}");
        }
    }

    #[test]
    fn radix_conversion_keeps_the_dot_and_zeros_that_the_token_strips() {
        let value = |id: &str| id.parse::<f64>().unwrap() / 1e15 * std::f64::consts::PI;
        assert_eq!(js_to_string_radix36(value("440322224407314432")), "12f.b9qdo78ad");
        assert_eq!(js_to_string_radix36(value("20")), "0.000000006dq1a2xwd93");
        assert_eq!(js_to_string_radix36(35.0), "z");
        assert_eq!(js_to_string_radix36(36.0), "10");
        assert_eq!(js_to_string_radix36(0.5), "0.i");
    }

    fn tweet(media: &str) -> String {
        format!(r#"{{"__typename":"Tweet","id_str":"20","text":"hi","mediaDetails":[{media}]}}"#)
    }

    fn photo(name: &str) -> String {
        format!(
            r#"{{"type":"photo","media_url_https":"https://pbs.twimg.com/media/{name}.jpg","url":"http://t.co/x"}}"#
        )
    }

    #[test]
    fn photos_come_back_at_full_size_and_in_order() {
        let json = tweet(&format!("{},{},{}", photo("AAA"), photo("BBB"), photo("AAA")));
        assert_eq!(
            parse_tweet_photos(&json),
            Ok(vec![
                "https://pbs.twimg.com/media/AAA.jpg?name=orig".to_string(),
                "https://pbs.twimg.com/media/BBB.jpg?name=orig".to_string(),
            ])
        );
        // A picture that already names a size gets the original instead.
        let sized =
            tweet(r#"{"type":"photo","media_url_https":"https://pbs.twimg.com/media/CCC.png?name=small"}"#);
        assert_eq!(
            parse_tweet_photos(&sized),
            Ok(vec!["https://pbs.twimg.com/media/CCC.png?name=orig".to_string()])
        );
    }

    #[test]
    fn the_photos_list_is_used_when_there_are_no_media_details() {
        let json = r#"{"__typename":"Tweet","id_str":"20","photos":[{"url":"https://pbs.twimg.com/media/DDD.jpg","width":1920,"height":1080}]}"#;
        assert_eq!(
            parse_tweet_photos(json),
            Ok(vec!["https://pbs.twimg.com/media/DDD.jpg?name=orig".to_string()])
        );
    }

    #[test]
    fn videos_and_gifs_are_left_to_the_video_downloader() {
        let video = tweet(
            r#"{"type":"video","media_url_https":"https://pbs.twimg.com/ext_tw_video_thumb/1/pu/img/x.jpg"}"#,
        );
        assert_eq!(parse_tweet_photos(&video), Err(TweetPhotosError::NoPhotos));
        let gif = tweet(&format!(
            "{},{}",
            photo("EEE"),
            r#"{"type":"animated_gif","media_url_https":"https://pbs.twimg.com/tweet_video_thumb/g.jpg"}"#
        ));
        assert_eq!(parse_tweet_photos(&gif), Err(TweetPhotosError::NoPhotos));
    }

    #[test]
    fn tweets_without_pictures_or_that_are_not_shown() {
        assert_eq!(parse_tweet_photos(&tweet("")), Err(TweetPhotosError::NoPhotos));
        assert_eq!(parse_tweet_photos("{}"), Err(TweetPhotosError::Unavailable));
        assert_eq!(parse_tweet_photos(""), Err(TweetPhotosError::Unavailable));
        assert_eq!(
            parse_tweet_photos("<!DOCTYPE html><html></html>"),
            Err(TweetPhotosError::Unavailable)
        );
        assert_eq!(
            parse_tweet_photos(r#"{"__typename":"TweetTombstone","id_str":"20"}"#),
            Err(TweetPhotosError::Unavailable)
        );
    }

    #[test]
    fn only_twitters_own_picture_host_is_trusted() {
        for url in [
            "https://evil.example/media/AAA.jpg",
            "http://pbs.twimg.com/media/AAA.jpg",
            "https://pbs.twimg.com.evil.example/media/AAA.jpg",
            "https://pbs.twimg.com/other/AAA.jpg",
            "file:///etc/passwd",
            "https://169.254.169.254/media/AAA.jpg",
        ] {
            let json = tweet(&format!(r#"{{"type":"photo","media_url_https":"{url}"}}"#));
            assert_eq!(
                parse_tweet_photos(&json),
                Err(TweetPhotosError::NoPhotos),
                "{url}"
            );
        }
    }

    #[test]
    fn no_more_than_ten_pictures() {
        let many = (0..30)
            .map(|i| photo(&format!("P{i}")))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(parse_tweet_photos(&tweet(&many)).unwrap().len(), MAX_PHOTOS);
    }
}
