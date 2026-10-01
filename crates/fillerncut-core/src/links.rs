//! Recognising and normalising the links the downloader accepts.

use serde::Serialize;
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    TikTok,
    Instagram,
    Twitter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedLink {
    /// A TikTok video or photo post with a known 19-digit id.
    TikTok {
        id: String,
        url: String,
    },
    /// `vt.tiktok.com/xyz`, `tiktok.com/t/xyz`, ... The id is only known after
    /// following the redirect.
    TikTokShort {
        url: String,
    },
    Instagram {
        url: String,
    },
    Twitter {
        url: String,
    },
}

impl ParsedLink {
    pub fn platform(&self) -> Platform {
        match self {
            ParsedLink::TikTok { .. } | ParsedLink::TikTokShort { .. } => Platform::TikTok,
            ParsedLink::Instagram { .. } => Platform::Instagram,
            ParsedLink::Twitter { .. } => Platform::Twitter,
        }
    }

    pub fn url(&self) -> &str {
        match self {
            ParsedLink::TikTok { url, .. }
            | ParsedLink::TikTokShort { url }
            | ParsedLink::Instagram { url }
            | ParsedLink::Twitter { url } => url,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkError {
    #[error("Paste a link first")]
    Empty,
    #[error("That doesn't look like a link")]
    NotAUrl,
    #[error("Unsupported site. Paste a TikTok, Instagram or X/Twitter link")]
    UnsupportedSite,
    #[error("This {0} link doesn't point to a specific post")]
    NoPost(&'static str),
}

/// Parse whatever the user pasted: a full link, a share link, or a bare
/// 19-digit TikTok post id.
pub fn parse_link(input: &str) -> Result<ParsedLink, LinkError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(LinkError::Empty);
    }

    if input.len() == 19 && input.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(tiktok_from_id(input));
    }

    let url = parse_url_lenient(input)?;
    let host = url.host_str().ok_or(LinkError::NotAUrl)?.to_ascii_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .or_else(|| host.strip_prefix("mobile."))
        .unwrap_or(&host)
        .to_string();
    let segments: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();

    match host.as_str() {
        "tiktok.com" => parse_tiktok(&url, &segments),
        "vt.tiktok.com" | "vm.tiktok.com" => {
            if segments.is_empty() {
                Err(LinkError::NoPost("TikTok"))
            } else {
                Ok(ParsedLink::TikTokShort { url: url.to_string() })
            }
        }
        "instagram.com" => parse_instagram(&segments),
        "twitter.com" | "x.com" | "fxtwitter.com" | "vxtwitter.com" | "fixupx.com" => {
            parse_twitter(&segments)
        }
        _ => Err(LinkError::UnsupportedSite),
    }
}

fn parse_url_lenient(input: &str) -> Result<Url, LinkError> {
    if input.contains(char::is_whitespace) {
        // Share sheets sometimes paste "Check this out https://..." — take the link.
        if let Some(token) = input
            .split_whitespace()
            .find(|t| t.starts_with("http://") || t.starts_with("https://"))
        {
            return Url::parse(token).map_err(|_| LinkError::NotAUrl);
        }
        return Err(LinkError::NotAUrl);
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        Url::parse(input).map_err(|_| LinkError::NotAUrl)
    } else if input.contains('.') {
        Url::parse(&format!("https://{input}")).map_err(|_| LinkError::NotAUrl)
    } else {
        Err(LinkError::NotAUrl)
    }
}

fn tiktok_from_id(id: &str) -> ParsedLink {
    ParsedLink::TikTok {
        id: id.to_string(),
        url: format!("https://www.tiktok.com/@_/video/{id}"),
    }
}

fn is_tiktok_id(s: &str) -> bool {
    s.len() == 19 && s.bytes().all(|b| b.is_ascii_digit())
}

fn parse_tiktok(url: &Url, segments: &[&str]) -> Result<ParsedLink, LinkError> {
    // /@user/video/<id> and /@user/photo/<id>
    if segments.len() >= 3
        && segments[0].starts_with('@')
        && (segments[1] == "video" || segments[1] == "photo")
        && is_tiktok_id(segments[2])
    {
        let kind = segments[1];
        return Ok(ParsedLink::TikTok {
            id: segments[2].to_string(),
            url: format!("https://www.tiktok.com/{}/{}/{}", segments[0], kind, segments[2]),
        });
    }
    // /v/<id>.html (legacy)
    if segments.len() == 2 && segments[0] == "v" {
        let id = segments[1].trim_end_matches(".html");
        if is_tiktok_id(id) {
            return Ok(tiktok_from_id(id));
        }
    }
    // /t/<code> short links
    if segments.len() >= 2 && segments[0] == "t" {
        return Ok(ParsedLink::TikTokShort { url: url.to_string() });
    }
    Err(LinkError::NoPost("TikTok"))
}

fn parse_instagram(segments: &[&str]) -> Result<ParsedLink, LinkError> {
    // /p/<code>, /reel/<code>, /reels/<code>, /tv/<code>, /<user>/reel/<code>,
    // /share/reel/<code>
    let pos = segments
        .iter()
        .position(|s| matches!(*s, "p" | "reel" | "reels" | "tv"));
    if let Some(i) = pos {
        if let Some(code) = segments.get(i + 1) {
            let kind = if segments[i] == "reels" {
                "reel"
            } else {
                segments[i]
            };
            let is_share = i > 0 && segments[i - 1] == "share";
            return Ok(ParsedLink::Instagram {
                url: if is_share {
                    format!("https://www.instagram.com/share/{kind}/{code}/")
                } else {
                    format!("https://www.instagram.com/{kind}/{code}/")
                },
            });
        }
    }
    Err(LinkError::NoPost("Instagram"))
}

fn parse_twitter(segments: &[&str]) -> Result<ParsedLink, LinkError> {
    // /<user>/status/<id>, /i/status/<id>, /i/web/status/<id>
    if let Some(i) = segments.iter().position(|s| *s == "status") {
        if i >= 1 {
            if let Some(id) = segments.get(i + 1) {
                if id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty() {
                    let user = if segments[i - 1] == "web" || segments[i - 1] == "i" {
                        "i"
                    } else {
                        segments[i - 1]
                    };
                    return Ok(ParsedLink::Twitter {
                        url: format!("https://x.com/{user}/status/{id}"),
                    });
                }
            }
        }
    }
    Err(LinkError::NoPost("X/Twitter"))
}

/// Pull the 19-digit post id out of the URL a TikTok short link redirects to.
pub fn tiktok_id_from_resolved(url: &str) -> Option<String> {
    match parse_link(url) {
        Ok(ParsedLink::TikTok { id, .. }) => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tt(id: &str, url: &str) -> ParsedLink {
        ParsedLink::TikTok {
            id: id.into(),
            url: url.into(),
        }
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_link("   "), Err(LinkError::Empty));
    }

    #[test]
    fn bare_tiktok_id() {
        assert_eq!(
            parse_link("7345678901234567890"),
            Ok(tt(
                "7345678901234567890",
                "https://www.tiktok.com/@_/video/7345678901234567890"
            ))
        );
    }

    #[test]
    fn tiktok_full_video_and_photo_links() {
        assert_eq!(
            parse_link("https://www.tiktok.com/@scout2015/video/6718335390845095173?lang=en"),
            Ok(tt(
                "6718335390845095173",
                "https://www.tiktok.com/@scout2015/video/6718335390845095173"
            ))
        );
        assert_eq!(
            parse_link("tiktok.com/@a.b_c/photo/7345678901234567890"),
            Ok(tt(
                "7345678901234567890",
                "https://www.tiktok.com/@a.b_c/photo/7345678901234567890"
            ))
        );
    }

    #[test]
    fn tiktok_short_links_need_resolving() {
        let l = parse_link("https://vt.tiktok.com/ZSabc123/").unwrap();
        assert!(matches!(l, ParsedLink::TikTokShort { .. }));
        assert_eq!(l.platform(), Platform::TikTok);
        assert!(matches!(
            parse_link("https://www.tiktok.com/t/ZTabc123/").unwrap(),
            ParsedLink::TikTokShort { .. }
        ));
    }

    #[test]
    fn tiktok_without_post_is_rejected() {
        assert_eq!(
            parse_link("https://www.tiktok.com/@scout2015"),
            Err(LinkError::NoPost("TikTok"))
        );
    }

    #[test]
    fn resolved_redirect_yields_id() {
        assert_eq!(
            tiktok_id_from_resolved("https://www.tiktok.com/@u/video/7345678901234567890?_r=1&u_code=x")
                .as_deref(),
            Some("7345678901234567890")
        );
        assert_eq!(tiktok_id_from_resolved("https://example.com"), None);
    }

    #[test]
    fn instagram_variants() {
        for (input, want) in [
            (
                "https://www.instagram.com/reel/C1a2B3c4D5e/?igsh=abc",
                "https://www.instagram.com/reel/C1a2B3c4D5e/",
            ),
            (
                "https://instagram.com/p/C1a2B3c4D5e/",
                "https://www.instagram.com/p/C1a2B3c4D5e/",
            ),
            (
                "https://www.instagram.com/someone/reel/C1a2B3c4D5e/",
                "https://www.instagram.com/reel/C1a2B3c4D5e/",
            ),
            (
                "https://www.instagram.com/reels/C1a2B3c4D5e/",
                "https://www.instagram.com/reel/C1a2B3c4D5e/",
            ),
            (
                "https://www.instagram.com/share/reel/BAbCdEf/",
                "https://www.instagram.com/share/reel/BAbCdEf/",
            ),
        ] {
            assert_eq!(
                parse_link(input),
                Ok(ParsedLink::Instagram { url: want.into() }),
                "{input}"
            );
        }
        assert_eq!(
            parse_link("https://www.instagram.com/someone/"),
            Err(LinkError::NoPost("Instagram"))
        );
    }

    #[test]
    fn twitter_variants() {
        for input in [
            "https://twitter.com/jack/status/20?s=20",
            "https://x.com/jack/status/20",
            "https://mobile.twitter.com/jack/status/20",
            "https://fxtwitter.com/jack/status/20",
        ] {
            assert_eq!(
                parse_link(input),
                Ok(ParsedLink::Twitter {
                    url: "https://x.com/jack/status/20".into()
                }),
                "{input}"
            );
        }
        assert_eq!(
            parse_link("https://x.com/i/status/1234567890"),
            Ok(ParsedLink::Twitter {
                url: "https://x.com/i/status/1234567890".into()
            })
        );
        assert_eq!(
            parse_link("https://x.com/jack"),
            Err(LinkError::NoPost("X/Twitter"))
        );
    }

    #[test]
    fn share_text_with_link_is_accepted() {
        assert_eq!(
            parse_link("Look at this https://x.com/jack/status/20 wow"),
            Ok(ParsedLink::Twitter {
                url: "https://x.com/jack/status/20".into()
            })
        );
    }

    #[test]
    fn rejects_other_sites_and_garbage() {
        assert_eq!(
            parse_link("https://youtube.com/watch?v=abc"),
            Err(LinkError::UnsupportedSite)
        );
        assert_eq!(parse_link("hello"), Err(LinkError::NotAUrl));
        assert_eq!(parse_link("12345"), Err(LinkError::NotAUrl));
    }
}
