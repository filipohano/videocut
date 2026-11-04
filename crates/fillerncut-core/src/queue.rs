//! The Telegram queue: links posted to a Telegram channel of the user's own wait here until
//! they're opened.
//!
//! A *public* channel can be read by anyone at `https://t.me/s/<name>`, so the app just reads
//! that page itself: no bot, no server, and everyone's queue is the channel they chose. This
//! module knows how to recognise a channel name, pick the links out of that page and keep the
//! queue on disk. The HTTP part is in the app (`src-tauri/src/queue.rs`).

use crate::links::{parse_link, Platform};
use crate::settings::write_json_atomic;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChannelError {
    #[error("Type the name of your Telegram channel, for example @my_links")]
    Empty,
    #[error("That looks like a private channel or an invite link. Telegram only shows public channels to other apps: give the channel a public username (Channel settings → Channel type → Public) and use that.")]
    Private,
    #[error("That isn't a Telegram channel name. It should look like @my_links or https://t.me/my_links (5–32 letters, digits or underscores).")]
    Invalid,
}

/// The channel's username from whatever the user typed: `@name`, `name`, `t.me/name`,
/// `https://t.me/name` or `https://t.me/s/name`. Only a plain username comes out, so it is safe
/// to put into an address.
pub fn parse_channel(input: &str) -> Result<String, ChannelError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(ChannelError::Empty);
    }
    let name = if let Some(rest) = text.strip_prefix('@') {
        rest
    } else if text.contains('/') || text.contains('.') {
        let without_scheme = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .unwrap_or(text);
        let (host, path) = without_scheme.split_once('/').ok_or(ChannelError::Invalid)?;
        let host = host.to_ascii_lowercase();
        if !matches!(
            host.as_str(),
            "t.me" | "www.t.me" | "telegram.me" | "telegram.dog"
        ) {
            return Err(ChannelError::Invalid);
        }
        let path = path.split(['?', '#']).next().unwrap_or("");
        let mut segments = path.split('/').filter(|s| !s.is_empty());
        match segments.next() {
            Some("s") => segments.next().ok_or(ChannelError::Invalid)?,
            Some("c") | Some("joinchat") => return Err(ChannelError::Private),
            Some(first) if first.starts_with('+') => return Err(ChannelError::Private),
            Some(first) => first,
            None => return Err(ChannelError::Invalid),
        }
    } else {
        text
    };
    let valid = (5..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if valid {
        Ok(name.to_string())
    } else if name.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        Err(ChannelError::Private) // a numeric channel id
    } else {
        Err(ChannelError::Invalid)
    }
}

/// A `https://t.me/<name>` address: what "Open the channel" may open. Checked strictly because
/// the address is handed to the operating system.
pub fn is_telegram_link(url: &str) -> bool {
    url.strip_prefix("https://t.me/").is_some_and(|name| {
        (5..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// One post of a channel, with the links in it that FillernCut can download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPost {
    pub id: i64,
    /// When it was posted, as Unix seconds.
    pub at: Option<i64>,
    pub links: Vec<String>,
}

/// Read the posts out of one page of `t.me/s/<name>` (the newest ~20, oldest first). Every
/// post is returned, also those without a usable link, so the queue can tell how far it has read.
pub fn parse_channel_page(html: &str) -> Vec<ChannelPost> {
    const POST: &str = "data-post=\"";
    let starts: Vec<usize> = html.match_indices(POST).map(|(i, _)| i).collect();
    let mut posts = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(html.len());
        let part = &html[start..end];
        let Some(id) = attribute(part, "data-post")
            .and_then(|v| v.rsplit('/').next().map(str::to_string))
            .and_then(|n| n.parse::<i64>().ok())
        else {
            continue;
        };
        let at = attribute(part, "datetime")
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.timestamp());
        let mut links: Vec<String> = Vec::new();
        for href in attributes(part, "href") {
            if let Ok(link) = parse_link(&href) {
                let url = link.url().to_string();
                if !links.contains(&url) {
                    links.push(url);
                }
            }
        }
        posts.push(ChannelPost { id, at, links });
    }
    posts
}

/// The value of the first `name="…"` in `html`.
fn attribute(html: &str, name: &str) -> Option<String> {
    attributes(html, name).into_iter().next()
}

/// The values of every `name="…"` in `html`, with HTML escapes undone.
fn attributes(html: &str, name: &str) -> Vec<String> {
    let needle = format!("{name}=\"");
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(&needle) {
        // `href=` must be a whole attribute, not the end of another one's name.
        let boundary = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == '<');
        let after = &rest[at + needle.len()..];
        let Some(close) = after.find('"') else { break };
        if boundary {
            found.push(unescape(&after[..close]));
        }
        rest = &after[close..];
    }
    found
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let entity = &rest[1..semi];
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .map(|hex| u32::from_str_radix(hex, 16))
                    .or_else(|| entity.strip_prefix('#').map(|dec| dec.parse::<u32>()))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, semi + 1))
        });
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A link waiting in the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: i64,
    pub url: String,
    pub platform: Platform,
    /// When it was posted in the channel, as Unix seconds.
    pub added_at: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueueState {
    /// The channel's username, once one is set.
    pub channel: Option<String>,
    pub items: Vec<QueueItem>,
    pub last_refreshed: Option<i64>,
    /// The newest post already looked at. Posts up to this one are never queued again, so a
    /// link that was opened or removed doesn't come back.
    seen_up_to: i64,
    next_id: i64,
}

impl QueueState {
    /// Nothing has been read from this channel yet.
    pub fn first_look(&self) -> bool {
        self.seen_up_to == 0
    }

    /// The newest post already looked at (0 before the first look).
    pub fn seen_up_to(&self) -> i64 {
        self.seen_up_to
    }

    /// Read this channel from now on. Switching to another channel starts a fresh queue;
    /// setting the same one again changes nothing.
    pub fn set_channel(&mut self, channel: &str) {
        if self.channel.as_deref() != Some(channel) {
            *self = QueueState {
                channel: Some(channel.to_string()),
                ..QueueState::default()
            };
        }
    }

    /// Queue the links of the posts that are new. Returns how many links were added.
    pub fn apply_posts(&mut self, mut posts: Vec<ChannelPost>, now: i64) -> usize {
        posts.sort_by_key(|p| p.id);
        posts.dedup_by_key(|p| p.id);
        let mut added = 0;
        let already_read = self.seen_up_to;
        for post in posts.into_iter().filter(|p| p.id > already_read) {
            self.seen_up_to = post.id;
            for url in post.links {
                if self.items.iter().any(|i| i.url == url) {
                    continue;
                }
                let Ok(link) = parse_link(&url) else { continue };
                self.next_id += 1;
                self.items.push(QueueItem {
                    id: self.next_id,
                    url,
                    platform: link.platform(),
                    added_at: post.at.unwrap_or(now),
                });
                added += 1;
            }
        }
        self.last_refreshed = Some(now);
        added
    }

    pub fn remove(&mut self, id: i64) {
        self.items.retain(|i| i.id != id);
    }

    /// Stop reading the channel and forget its links.
    pub fn disconnect(&mut self) {
        *self = QueueState::default();
    }
}

/// The most channels the app reads. Every one of them is read every 30 minutes, and Telegram
/// would start refusing an app that reads very many channels that often.
pub const MAX_QUEUES: usize = 20;

/// One Telegram channel and the links waiting from it.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Queue {
    /// Never reused, so a queue that is removed and another one added can't be mixed up.
    pub id: u64,
    #[serde(flatten)]
    pub state: QueueState,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueueError {
    #[error("t.me/{0} is already one of your queues.")]
    Duplicate(String),
    #[error("You can have at most {MAX_QUEUES} queues. Remove one you don't use first.")]
    TooMany,
    #[error("That queue isn't there any more.")]
    Missing,
    #[error("Couldn't save the queue: {0}")]
    Io(String),
}

/// What `queue.json` holds.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct QueueFile {
    queues: Vec<Queue>,
    next_queue_id: u64,
}

/// The two-column versions wrote `{"slots": [...]}`.
#[derive(Deserialize)]
struct SlotsFile {
    slots: Vec<QueueState>,
}

/// Read `queue.json`. Files from older versions (one channel, then two columns) become a list
/// of queues, keeping their order and everything already read. Anything damaged starts empty
/// rather than stopping the app.
fn parse_queue_file(text: &str) -> QueueFile {
    let value: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let states: Vec<QueueState> = if value.get("queues").is_some() {
        let mut file: QueueFile = serde_json::from_value(value).unwrap_or_default();
        file.queues.retain(|q| q.state.channel.is_some());
        file.queues.truncate(MAX_QUEUES);
        // Ids must be unique and below the next one, whatever the file says.
        let mut seen = std::collections::HashSet::new();
        let mut next = file
            .queues
            .iter()
            .map(|q| q.id)
            .max()
            .unwrap_or(0)
            .max(file.next_queue_id);
        for q in &mut file.queues {
            if q.id == 0 || !seen.insert(q.id) {
                next += 1;
                q.id = next;
                seen.insert(q.id);
            }
        }
        file.next_queue_id = next;
        return file;
    } else if value.get("slots").is_some() {
        serde_json::from_value::<SlotsFile>(value)
            .map(|f| f.slots)
            .unwrap_or_default()
    } else {
        serde_json::from_value::<QueueState>(value)
            .map(|s| vec![s])
            .unwrap_or_default()
    };
    let mut file = QueueFile::default();
    for state in states
        .into_iter()
        .filter(|s| s.channel.is_some())
        .take(MAX_QUEUES)
    {
        file.next_queue_id += 1;
        file.queues.push(Queue {
            id: file.next_queue_id,
            state,
        });
    }
    file
}

/// The queues on disk (`queue.json` in the app data folder), in the order they were added.
pub struct QueueStore {
    path: PathBuf,
    file: Mutex<QueueFile>,
}

impl QueueStore {
    pub fn new(path: PathBuf) -> QueueStore {
        let file = std::fs::read_to_string(&path)
            .map(|text| parse_queue_file(&text))
            .unwrap_or_default();
        QueueStore {
            path,
            file: Mutex::new(file),
        }
    }

    pub fn list(&self) -> Vec<Queue> {
        self.file.lock().unwrap().queues.clone()
    }

    pub fn get(&self, id: u64) -> Option<Queue> {
        self.file
            .lock()
            .unwrap()
            .queues
            .iter()
            .find(|q| q.id == id)
            .cloned()
    }

    /// The queue already reading `channel` (Telegram ignores capitals, so this does too).
    pub fn find_channel(&self, channel: &str) -> Option<Queue> {
        self.file
            .lock()
            .unwrap()
            .queues
            .iter()
            .find(|q| {
                q.state
                    .channel
                    .as_deref()
                    .is_some_and(|c| c.eq_ignore_ascii_case(channel))
            })
            .cloned()
    }

    /// Whether another queue may be added for `channel` (before it is read from Telegram).
    pub fn can_add(&self, channel: &str) -> Result<(), QueueError> {
        if let Some(existing) = self.find_channel(channel) {
            return Err(QueueError::Duplicate(existing.state.channel.unwrap_or_default()));
        }
        if self.file.lock().unwrap().queues.len() >= MAX_QUEUES {
            return Err(QueueError::TooMany);
        }
        Ok(())
    }

    /// Add a queue for `channel` at the end, let `fill` put its first links in, and save.
    pub fn add<T>(
        &self,
        channel: &str,
        fill: impl FnOnce(&mut QueueState) -> T,
    ) -> Result<(Queue, T), QueueError> {
        let mut file = self.file.lock().unwrap();
        // Checked again here: the same channel may have been added while it was being read.
        if let Some(existing) = file.queues.iter().find(|q| {
            q.state
                .channel
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(channel))
        }) {
            return Err(QueueError::Duplicate(
                existing.state.channel.clone().unwrap_or_default(),
            ));
        }
        if file.queues.len() >= MAX_QUEUES {
            return Err(QueueError::TooMany);
        }
        let mut state = QueueState::default();
        state.set_channel(channel);
        let result = fill(&mut state);
        file.next_queue_id += 1;
        let queue = Queue {
            id: file.next_queue_id,
            state,
        };
        file.queues.push(queue.clone());
        write_json_atomic(&self.path, &*file).map_err(|e| QueueError::Io(e.to_string()))?;
        Ok((queue, result))
    }

    /// Change one queue and save everything before returning.
    pub fn update<T>(&self, id: u64, change: impl FnOnce(&mut QueueState) -> T) -> Result<T, QueueError> {
        let mut file = self.file.lock().unwrap();
        let queue = file
            .queues
            .iter_mut()
            .find(|q| q.id == id)
            .ok_or(QueueError::Missing)?;
        let result = change(&mut queue.state);
        write_json_atomic(&self.path, &*file).map_err(|e| QueueError::Io(e.to_string()))?;
        Ok(result)
    }

    /// Stop reading a channel and forget its links. Removing one that is already gone is fine.
    pub fn remove(&self, id: u64) -> Result<(), QueueError> {
        let mut file = self.file.lock().unwrap();
        let before = file.queues.len();
        file.queues.retain(|q| q.id != id);
        if file.queues.len() != before {
            write_json_atomic(&self.path, &*file).map_err(|e| QueueError::Io(e.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REEL: &str = "https://www.instagram.com/reel/C1a2B3c4D5e/";
    const TIKTOK: &str = "https://www.tiktok.com/@someone/video/7123456789012345678";
    const TWEET: &str = "https://x.com/jack/status/20";

    fn post(id: i64, links: &[&str]) -> ChannelPost {
        ChannelPost {
            id,
            at: Some(1_800_000_000 + id),
            links: links.iter().map(|l| l.to_string()).collect(),
        }
    }

    #[test]
    fn channel_names_come_out_plain_from_every_way_of_writing_them() {
        for (input, expected) in [
            ("my_links", "my_links"),
            ("@my_links", "my_links"),
            ("  @my_links \n", "my_links"),
            ("t.me/my_links", "my_links"),
            ("https://t.me/my_links", "my_links"),
            ("https://t.me/my_links/", "my_links"),
            ("https://t.me/my_links/123", "my_links"),
            ("https://t.me/s/my_links", "my_links"),
            ("https://t.me/s/my_links?before=40", "my_links"),
            ("http://telegram.me/My_Links", "My_Links"),
            ("https://www.t.me/my_links", "my_links"),
        ] {
            assert_eq!(parse_channel(input).as_deref(), Ok(expected), "{input:?}");
        }
    }

    #[test]
    fn private_channels_and_junk_get_an_explanation() {
        assert_eq!(parse_channel(""), Err(ChannelError::Empty));
        assert_eq!(parse_channel("   "), Err(ChannelError::Empty));
        for private in [
            "https://t.me/+AbCdEfGhIjK",
            "https://t.me/joinchat/AAAA",
            "https://t.me/c/1234567890/5",
            "-1001234567890",
        ] {
            assert_eq!(parse_channel(private), Err(ChannelError::Private), "{private}");
        }
        for bad in [
            "abc",
            "a".repeat(33).as_str(),
            "my channel",
            "my-links",
            "1links",
            "@",
            "https://example.com/my_links",
            "https://t.me/",
            "https://t.me/s/",
            "my_links/../x",
            "https://evil.example/t.me/my_links",
            "ftp://t.me/my_links",
        ] {
            assert!(parse_channel(bad).is_err(), "{bad:?} should not be a channel");
        }
        for tricky in ["my_links?x=1", "my_links#a", "my_links&x"] {
            assert!(parse_channel(tricky).is_err(), "{tricky}");
        }
    }

    #[test]
    fn only_plain_channel_addresses_can_be_opened() {
        assert!(is_telegram_link("https://t.me/my_links"));
        for bad in [
            "http://t.me/my_links",
            "https://t.me/",
            "https://t.me/abc",
            "https://t.me/my_links?start=1",
            "https://t.me/my_links/5",
            "https://t.me/my_links\" & calc & \"",
            "https://t.me.evil.example/my_links",
            "https://evil.example/https://t.me/my_links",
        ] {
            assert!(!is_telegram_link(bad), "{bad}");
        }
    }

    /// Markup as t.me serves it (taken from a real channel page, with our own text).
    fn page() -> String {
        let message = |id: i64, body: &str, time: &str| {
            format!(
                r#"<div class="tgme_widget_message_wrap js-widget_message_wrap"><div class="tgme_widget_message text_not_supported_wrap js-widget_message" data-post="my_links/{id}" data-view="abc">
  <div class="tgme_widget_message_user"><a href="https://t.me/my_links"><i class="tgme_widget_message_user_photo bgcolor4" data-content="M"></i></a></div>
  <div class="tgme_widget_message_bubble">
    <div class="tgme_widget_message_text js-message_text" dir="auto">{body}</div>
    <div class="tgme_widget_message_footer compact js-message_footer"><div class="tgme_widget_message_info short js-message_info">
      <span class="tgme_widget_message_meta"><a class="tgme_widget_message_date" href="https://t.me/my_links/{id}"><time datetime="{time}" class="time">16:08</time></a></span></div></div>
  </div></div></div>"#
            )
        };
        format!(
            r#"<html><body><section class="tgme_channel_history js-message_history"><div class="tgme_widget_message_centered js-messages_more_wrap"><a href="/s/my_links?before=41" class="tme_messages_more js-messages_more" data-before="41"></a></div>
{}{}{}{}</section></body></html>"#,
            message(
                41,
                &format!(
                    r#"<a href="{REEL}?igsh=abc&amp;utm=1" target="_blank" rel="noopener" onclick="return confirm('Open this link?\n\n'+this.href);">{REEL}?igsh=abc&amp;utm=1</a>"#
                ),
                "2026-10-19T08:30:00+00:00"
            ),
            message(
                42,
                "just a note, no link &#39;quoted&#39; &amp; stuff",
                "2026-10-19T09:00:00+00:00"
            ),
            message(
                43,
                &format!(
                    r#"look <a href="{TIKTOK}" target="_blank" rel="noopener">tiktok</a><br/><a href="https://t.me/my_links/41">earlier</a> and <a href="http://x.com/jack/status/20" target="_blank">x.com/jack/status/20</a> <a href="https://x.com/jack/status/20">again</a> <a href="https://vimeo.com/76979871">other</a><a class="tgme_widget_message_link_preview" href="{TIKTOK}"></a>"#
                ),
                "2026-10-19T10:15:30+02:00"
            ),
            message(44, "<b>no date here</b>", "garbage"),
        )
    }

    #[test]
    fn reads_posts_dates_and_supported_links_from_a_real_style_page() {
        let posts = parse_channel_page(&page());
        assert_eq!(
            posts.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![41, 42, 43, 44]
        );
        // Query noise is dropped from the link, and `&amp;` was undone before parsing.
        assert_eq!(posts[0].links, vec![REEL.to_string()]);
        assert_eq!(posts[0].at, Some(1_792_398_600));
        assert!(posts[1].links.is_empty());
        // Telegram's own links, other sites and repeats are left out; order is kept.
        assert_eq!(posts[2].links, vec![TIKTOK.to_string(), TWEET.to_string()]);
        // A time zone is honoured; a broken date is just unknown.
        // 10:15:30 at +02:00 is 08:15:30 UTC, 14 min 30 s before post 41's 08:30:00.
        assert_eq!(posts[2].at, Some(1_792_398_600 - 14 * 60 - 30));
        assert_eq!(posts[3].at, None);
    }

    #[test]
    fn a_page_without_posts_gives_nothing() {
        assert!(parse_channel_page("").is_empty());
        assert!(
            parse_channel_page("<html><div class=\"tgme_page_title\">Some channel</div></html>").is_empty()
        );
        // A post marker without a number, or cut off in the middle.
        assert!(parse_channel_page("<div data-post=\"abc\"><a href=\"x\"></a>").is_empty());
        assert_eq!(
            parse_channel_page("<div data-post=\"n/7\"><a href=\"https://x.com/a/sta").len(),
            1
        );
    }

    #[test]
    fn html_escapes_are_undone() {
        assert_eq!(
            unescape("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#39; &#x26; &nbsp;f"),
            "a & b <c> \"d\" 'e' &  f"
        );
        assert_eq!(
            unescape("fish & chips &unknown; &#zz; &"),
            "fish & chips &unknown; &#zz; &"
        );
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn attribute_names_must_match_whole() {
        let html = r#"<a data-href="https://x.com/no/status/1" href="https://x.com/yes/status/2">"#;
        assert_eq!(
            attributes(html, "href"),
            vec!["https://x.com/yes/status/2".to_string()]
        );
    }

    #[test]
    fn new_links_are_queued_oldest_first_and_only_once() {
        let mut state = QueueState::default();
        state.set_channel("my_links");
        assert!(state.first_look());
        let added = state.apply_posts(
            vec![
                post(12, &[TIKTOK]),
                post(10, &[REEL]),
                post(11, &[]),
                post(10, &[REEL]),
            ],
            5,
        );
        assert_eq!(added, 2);
        assert_eq!(
            state.items.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(),
            vec![REEL, TIKTOK]
        );
        assert_eq!(state.items[0].platform, Platform::Instagram);
        assert_eq!(state.items[0].added_at, 1_800_000_010);
        assert_eq!(state.seen_up_to(), 12);
        assert!(!state.first_look());
        assert_eq!(state.last_refreshed, Some(5));

        // The same page again, and a page that overlaps: nothing new.
        assert_eq!(
            state.apply_posts(vec![post(10, &[REEL]), post(12, &[TIKTOK])], 6),
            0
        );
        // A newer post comes in on top.
        assert_eq!(
            state.apply_posts(vec![post(12, &[TIKTOK]), post(13, &[TWEET])], 7),
            1
        );
        assert_eq!(state.items.len(), 3);
    }

    #[test]
    fn a_link_that_was_dealt_with_does_not_come_back() {
        let mut state = QueueState::default();
        state.set_channel("my_links");
        state.apply_posts(vec![post(1, &[REEL])], 1);
        let id = state.items[0].id;
        state.remove(id);
        assert_eq!(state.apply_posts(vec![post(1, &[REEL])], 2), 0);
        assert!(state.items.is_empty());
        // Posted again later, it is a new post and is queued again.
        assert_eq!(state.apply_posts(vec![post(2, &[REEL])], 3), 1);
    }

    #[test]
    fn the_same_link_twice_while_waiting_is_one_item() {
        let mut state = QueueState::default();
        state.set_channel("my_links");
        assert_eq!(
            state.apply_posts(vec![post(1, &[REEL, REEL]), post(2, &[REEL])], 1),
            1
        );
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.seen_up_to(), 2);
    }

    #[test]
    fn item_numbers_are_never_reused() {
        let mut state = QueueState::default();
        state.set_channel("my_links");
        state.apply_posts(vec![post(1, &[REEL])], 1);
        let first = state.items[0].id;
        state.remove(first);
        state.apply_posts(vec![post(2, &[TWEET])], 2);
        assert_ne!(state.items[0].id, first);
    }

    #[test]
    fn another_channel_starts_fresh_and_the_same_one_keeps_everything() {
        let mut state = QueueState::default();
        state.set_channel("my_links");
        state.apply_posts(vec![post(5, &[REEL])], 1);
        state.set_channel("my_links");
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.seen_up_to(), 5);
        state.set_channel("other_channel");
        assert!(state.items.is_empty());
        assert!(state.first_look());
        assert_eq!(state.channel.as_deref(), Some("other_channel"));
    }

    #[test]
    fn disconnecting_forgets_everything() {
        let mut state = QueueState::default();
        state.set_channel("my_links");
        state.apply_posts(vec![post(5, &[REEL])], 1);
        state.disconnect();
        assert_eq!(state, QueueState::default());
    }

    #[test]
    fn the_queues_survive_a_restart_and_a_damaged_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("queue.json");
        let store = QueueStore::new(path.clone());
        let (first, _) = store
            .add("my_links", |s| s.apply_posts(vec![post(7, &[REEL])], 9))
            .unwrap();
        let (second, _) = store
            .add("other_channel", |s| s.apply_posts(vec![post(3, &[TIKTOK])], 10))
            .unwrap();
        assert_ne!(first.id, second.id);

        let reopened = QueueStore::new(path.clone());
        let list = reopened.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, first.id);
        assert_eq!(list[0].state.channel.as_deref(), Some("my_links"));
        assert_eq!(list[0].state.items.len(), 1);
        assert_eq!(list[0].state.last_refreshed, Some(9));
        assert_eq!(list[1].state.channel.as_deref(), Some("other_channel"));
        assert_eq!(list[1].state.items[0].url, TIKTOK);
        // What was already read stays read.
        let mut again = list[0].state.clone();
        assert_eq!(again.apply_posts(vec![post(7, &[REEL])], 10), 0);

        std::fs::write(&path, "{ not json").unwrap();
        assert!(QueueStore::new(path).list().is_empty());
    }

    #[test]
    fn queues_are_independent_and_removing_one_leaves_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(dir.path().join("queue.json"));
        let ids: Vec<u64> = ["one_channel", "two_channel", "three_channel"]
            .iter()
            .map(|c| store.add(c, |_| ()).unwrap().0.id)
            .collect();
        store
            .update(ids[1], |s| s.apply_posts(vec![post(1, &[REEL])], 1))
            .unwrap();
        assert_eq!(store.get(ids[1]).unwrap().state.items.len(), 1);
        assert!(store.get(ids[0]).unwrap().state.items.is_empty());

        store.remove(ids[1]).unwrap();
        store.remove(ids[1]).unwrap(); // already gone: fine
        let left: Vec<_> = store.list().iter().map(|q| q.id).collect();
        assert_eq!(left, vec![ids[0], ids[2]]);
        assert_eq!(store.update(ids[1], |_| ()), Err(QueueError::Missing));

        // A new queue never gets the number of one that was removed.
        let (fresh, _) = store.add("four_channel", |_| ()).unwrap();
        assert!(fresh.id > ids[2]);
    }

    #[test]
    fn a_channel_is_only_queued_once_whatever_its_capitals() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(dir.path().join("queue.json"));
        store.add("My_Links", |_| ()).unwrap();
        assert_eq!(
            store.can_add("my_links"),
            Err(QueueError::Duplicate("My_Links".into()))
        );
        assert_eq!(
            store.add("MY_LINKS", |_| ()).unwrap_err(),
            QueueError::Duplicate("My_Links".into())
        );
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn there_is_a_limit_on_how_many_channels_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::new(dir.path().join("queue.json"));
        for i in 0..MAX_QUEUES {
            store.add(&format!("channel_{i}"), |_| ()).unwrap();
        }
        assert_eq!(store.can_add("one_too_many"), Err(QueueError::TooMany));
        assert_eq!(
            store.add("one_too_many", |_| ()).unwrap_err(),
            QueueError::TooMany
        );
        assert_eq!(store.list().len(), MAX_QUEUES);
    }

    #[test]
    fn a_queue_from_the_single_channel_versions_is_kept() {
        // `queue.json` exactly as the single-channel versions wrote it.
        let old = r#"{
          "channel": "my_links",
          "items": [{"id": 3, "url": "https://x.com/jack/status/20", "platform": "twitter", "addedAt": 1800000000}],
          "lastRefreshed": 1800000100,
          "seenUpTo": 42,
          "nextId": 3
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.json");
        std::fs::write(&path, old).unwrap();

        let store = QueueStore::new(path.clone());
        let list = store.list();
        assert_eq!(list.len(), 1);
        let mut first = list[0].state.clone();
        assert_eq!(first.channel.as_deref(), Some("my_links"));
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.seen_up_to(), 42);
        // Reading continues where it stopped: nothing old is queued again.
        assert_eq!(
            first.apply_posts(vec![post(42, &[REEL]), post(43, &[TIKTOK])], 5),
            1
        );

        // The next save writes the new format, and it reads back the same.
        store.add("second_channel", |_| ()).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("\"queues\""), "{saved}");
        let again = QueueStore::new(path);
        assert_eq!(again.list()[0].state.items.len(), 1);
        assert_eq!(again.list()[0].state.seen_up_to(), 42);
        assert_eq!(again.list()[1].state.channel.as_deref(), Some("second_channel"));
    }

    #[test]
    fn the_two_columns_of_the_last_version_become_two_queues() {
        let two = r#"{"slots":[{"channel":"left_channel","seenUpTo":9,"items":[{"id":1,"url":"https://x.com/jack/status/20","platform":"twitter","addedAt":1}]},{"channel":"right_channel"}]}"#;
        let file = parse_queue_file(two);
        assert_eq!(file.queues.len(), 2);
        assert_eq!(file.queues[0].state.channel.as_deref(), Some("left_channel"));
        assert_eq!(file.queues[0].state.seen_up_to(), 9);
        assert_eq!(file.queues[0].state.items.len(), 1);
        assert_eq!(file.queues[1].state.channel.as_deref(), Some("right_channel"));
        assert_ne!(file.queues[0].id, file.queues[1].id);
        assert!(file.next_queue_id >= 2);

        // A column without a channel was just an empty column, not a queue.
        let only_right = r#"{"slots":[{},{"channel":"right_channel"}]}"#;
        assert_eq!(parse_queue_file(only_right).queues.len(), 1);
        assert!(parse_queue_file("").queues.is_empty());
        assert!(parse_queue_file("{}").queues.is_empty());
    }

    #[test]
    fn a_hand_edited_file_with_clashing_numbers_is_repaired() {
        let text = r#"{"queues":[{"id":3,"channel":"a_channel"},{"id":3,"channel":"b_channel"},{"id":0,"channel":"c_channel"},{"id":9}],"nextQueueId":1}"#;
        let file = parse_queue_file(text);
        let ids: Vec<u64> = file.queues.iter().map(|q| q.id).collect();
        assert_eq!(file.queues.len(), 3, "a queue without a channel is dropped");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3, "{ids:?}");
        assert!(
            ids.iter().all(|id| *id > 0 && *id <= file.next_queue_id),
            "{ids:?}"
        );
    }
}
