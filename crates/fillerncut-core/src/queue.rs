//! The Telegram link queue: what the app keeps on disk, and how the queue server's
//! answers are merged into it.
//!
//! A Telegram bot collects links; the app picks them up from a small server (see
//! `queue-worker/`). The app proves who it is with a random secret token that is made
//! on this computer and never shown to anyone, so only the chats that were connected
//! with a code from *this* installation feed *this* queue.

use crate::links::{parse_link, Platform};
use crate::settings::write_json_atomic;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

/// How many already-taken server ids are remembered, so a lost acknowledgement can't
/// bring back a link the user has dealt with.
const MAX_SEEN: usize = 500;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueueError {
    #[error("The Telegram queue isn't set up in this version of FillernCut. Add the queue server's address in Settings → Telegram queue.")]
    NotConfigured,
    #[error("The queue server address isn't valid. It should look like https://fillerncut-queue.example.workers.dev")]
    BadServer,
}

/// The server address compiled into this build (`FILLERNCUT_QUEUE_URL` when building), if any.
pub fn built_in_server() -> Option<&'static str> {
    option_env!("FILLERNCUT_QUEUE_URL")
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The server to talk to: the address typed in Settings, else the one built into this
/// version. Plain `http` is only accepted for this computer (development).
pub fn resolve_server(setting: Option<&str>) -> Result<String, QueueError> {
    server_from(setting, built_in_server())
}

fn server_from(setting: Option<&str>, built_in: Option<&str>) -> Result<String, QueueError> {
    let raw = setting
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(built_in)
        .ok_or(QueueError::NotConfigured)?;
    let url = Url::parse(raw).map_err(|_| QueueError::BadServer)?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
    if !scheme_ok || url.host_str().is_none() || url.query().is_some() || url.fragment().is_some() {
        return Err(QueueError::BadServer);
    }
    Ok(raw.trim_end_matches('/').to_string())
}

/// A `https://t.me/<bot>` link, optionally with `?start=<code>`: what "Connect" opens. Checked
/// strictly because the address is handed to the operating system.
pub fn is_telegram_link(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://t.me/") else {
        return false;
    };
    let (bot, code) = match rest.split_once("?start=") {
        Some((bot, code)) => (bot, Some(code)),
        None => (rest, None),
    };
    let bot_ok = (5..=32).contains(&bot.len()) && bot.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let code_ok = match code {
        None => true,
        Some(code) => {
            (1..=64).contains(&code.len())
                && code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
    };
    bot_ok && code_ok
}

/// A fresh secret for this installation: 256 random bits as hex.
pub fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// A link waiting in the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    /// The server's number for it.
    pub id: i64,
    pub url: String,
    pub platform: Platform,
    /// When it was sent, as Unix seconds.
    pub added_at: i64,
}

/// A Telegram chat connected to this installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedChat {
    pub label: String,
    pub linked_at: i64,
}

/// What `GET /v1/queue` answers.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteQueue {
    pub bot: String,
    #[serde(default)]
    pub linked: Vec<LinkedChat>,
    #[serde(default)]
    pub items: Vec<RemoteItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteItem {
    pub id: i64,
    pub url: String,
    pub at: i64,
}

/// What `POST /v1/pair` answers: the code to give the bot, as a link that opens it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairInfo {
    pub code: String,
    pub link: String,
    pub bot: String,
    pub expires_at: i64,
}

/// The outcome of merging one server answer.
#[derive(Debug, PartialEq, Eq)]
pub struct Applied {
    /// Links that are new in the queue.
    pub added: usize,
    /// Every id the server handed over, to tell it they can be forgotten.
    pub acknowledge: Vec<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueueState {
    /// This installation's secret. `None` until the user connects Telegram.
    pub token: Option<String>,
    pub bot: Option<String>,
    pub linked: Vec<LinkedChat>,
    pub items: Vec<QueueItem>,
    seen: Vec<i64>,
    pub last_refreshed: Option<i64>,
}

impl QueueState {
    pub fn ensure_token(&mut self) -> String {
        self.token.get_or_insert_with(new_token).clone()
    }

    /// Merge what the server holds. Links that aren't TikTok/Instagram/X posts, ones already
    /// in the queue and ones taken before are dropped, but all are acknowledged.
    pub fn apply(&mut self, remote: RemoteQueue, now: i64) -> Applied {
        self.bot = Some(remote.bot);
        self.linked = remote.linked;
        self.last_refreshed = Some(now);

        let mut added = 0;
        let mut acknowledge = Vec::new();
        for item in remote.items {
            acknowledge.push(item.id);
            if self.seen.contains(&item.id) {
                continue;
            }
            self.seen.push(item.id);
            let Ok(link) = parse_link(&item.url) else { continue };
            let url = link.url().to_string();
            if self.items.iter().any(|i| i.url == url) {
                continue;
            }
            self.items.push(QueueItem {
                id: item.id,
                url,
                platform: link.platform(),
                added_at: item.at,
            });
            added += 1;
        }
        if self.seen.len() > MAX_SEEN {
            let excess = self.seen.len() - MAX_SEEN;
            self.seen.drain(..excess);
        }
        Applied { added, acknowledge }
    }

    pub fn remove(&mut self, id: i64) {
        self.items.retain(|i| i.id != id);
    }

    /// Forget the connection: the token, the chats and the links.
    pub fn disconnect(&mut self) {
        *self = QueueState::default();
    }
}

/// The queue on disk (`queue.json` in the app data folder).
pub struct QueueStore {
    path: PathBuf,
    state: Mutex<QueueState>,
}

impl QueueStore {
    /// A missing or damaged file starts an empty queue rather than stopping the app.
    pub fn new(path: PathBuf) -> QueueStore {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        QueueStore {
            path,
            state: Mutex::new(state),
        }
    }

    pub fn snapshot(&self) -> QueueState {
        self.state.lock().unwrap().clone()
    }

    /// Change the queue and save it before returning.
    pub fn update<T>(&self, change: impl FnOnce(&mut QueueState) -> T) -> io::Result<T> {
        let mut state = self.state.lock().unwrap();
        let result = change(&mut state);
        write_json_atomic(&self.path, &*state)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REEL: &str = "https://www.instagram.com/reel/C1a2B3c4D5e/";
    const TIKTOK: &str = "https://www.tiktok.com/@someone/video/7123456789012345678";

    fn remote(items: &[(i64, &str)]) -> RemoteQueue {
        RemoteQueue {
            bot: "FillernBot".into(),
            linked: vec![LinkedChat {
                label: "Filip".into(),
                linked_at: 5,
            }],
            items: items
                .iter()
                .map(|(id, url)| RemoteItem {
                    id: *id,
                    url: (*url).into(),
                    at: 100 + id,
                })
                .collect(),
        }
    }

    #[test]
    fn server_comes_from_settings_then_the_build() {
        let built = Some("https://built.example.workers.dev");
        assert_eq!(
            server_from(None, built).unwrap(),
            "https://built.example.workers.dev"
        );
        assert_eq!(
            server_from(Some(" https://mine.example.dev/ "), built).unwrap(),
            "https://mine.example.dev"
        );
        // An empty box in Settings means "use the built-in one".
        assert_eq!(
            server_from(Some("  "), built).unwrap(),
            "https://built.example.workers.dev"
        );
        assert_eq!(server_from(None, None), Err(QueueError::NotConfigured));
    }

    #[test]
    fn server_must_be_https_except_on_this_computer() {
        for ok in [
            "https://q.example.dev",
            "http://localhost:8787",
            "http://127.0.0.1:8787/",
            "http://[::1]:8787",
        ] {
            assert!(server_from(Some(ok), None).is_ok(), "{ok}");
        }
        for bad in [
            "http://q.example.dev",
            "ftp://q.example.dev",
            "q.example.dev",
            "https://",
            "https://q.example.dev/?x=1",
            "https://q.example.dev/#a",
            "javascript:alert(1)",
        ] {
            assert_eq!(server_from(Some(bad), None), Err(QueueError::BadServer), "{bad}");
        }
    }

    #[test]
    fn only_plain_telegram_bot_links_may_be_opened() {
        for ok in [
            "https://t.me/FillernBot",
            "https://t.me/FillernBot?start=ABCD2345",
            "https://t.me/a_b_c_d_e?start=x-y_z",
        ] {
            assert!(is_telegram_link(ok), "{ok}");
        }
        for bad in [
            "http://t.me/FillernBot",
            "https://t.me/",
            "https://t.me/abc",
            "https://t.me/FillernBot?start=",
            "https://t.me/FillernBot?start=a&b=c",
            "https://t.me/FillernBot?start=a b",
            "https://t.me/FillernBot/extra",
            "https://t.me/FillernBot\" & calc & \"",
            "https://t.me.evil.example/FillernBot",
            "https://evil.example/https://t.me/FillernBot",
            "https://t.me/FillernBot?start=\"; rm -rf ~",
        ] {
            assert!(!is_telegram_link(bad), "{bad}");
        }
    }

    #[test]
    fn tokens_are_long_unique_and_acceptable_to_the_server() {
        let (a, b) = (new_token(), new_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        let mut state = QueueState::default();
        assert_eq!(state.ensure_token(), state.ensure_token(), "made once, then kept");
    }

    #[test]
    fn new_links_are_added_in_order_and_everything_is_acknowledged() {
        let mut state = QueueState::default();
        let applied = state.apply(remote(&[(1, REEL), (2, TIKTOK)]), 1000);
        assert_eq!(
            applied,
            Applied {
                added: 2,
                acknowledge: vec![1, 2]
            }
        );
        assert_eq!(state.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(state.items[0].platform, Platform::Instagram);
        assert_eq!(state.items[1].platform, Platform::TikTok);
        assert_eq!(state.items[0].added_at, 101);
        assert_eq!(state.bot.as_deref(), Some("FillernBot"));
        assert_eq!(state.linked[0].label, "Filip");
        assert_eq!(state.last_refreshed, Some(1000));
    }

    #[test]
    fn a_redelivered_link_is_not_added_again_even_after_the_user_removed_it() {
        let mut state = QueueState::default();
        state.apply(remote(&[(1, REEL)]), 1);
        state.remove(1);
        // The acknowledgement got lost, so the server sends it once more.
        let again = state.apply(remote(&[(1, REEL)]), 2);
        assert_eq!(
            again,
            Applied {
                added: 0,
                acknowledge: vec![1]
            }
        );
        assert!(state.items.is_empty());
    }

    #[test]
    fn the_same_link_sent_twice_is_one_item_and_links_are_cleaned_up() {
        let mut state = QueueState::default();
        let applied = state.apply(
            remote(&[
                (1, REEL),
                (2, "https://www.instagram.com/reel/C1a2B3c4D5e/?igsh=tracking"),
            ]),
            1,
        );
        assert_eq!(applied.added, 1);
        assert_eq!(applied.acknowledge, vec![1, 2]);
        assert_eq!(state.items[0].url, REEL);
    }

    #[test]
    fn links_the_app_cant_use_are_dropped_but_acknowledged() {
        let mut state = QueueState::default();
        let applied = state.apply(
            remote(&[(1, "https://youtube.com/watch?v=1"), (2, "https://x.com/jack")]),
            1,
        );
        assert_eq!(
            applied,
            Applied {
                added: 0,
                acknowledge: vec![1, 2]
            }
        );
        assert!(state.items.is_empty());
    }

    #[test]
    fn only_the_latest_server_ids_are_remembered() {
        let mut state = QueueState::default();
        let many: Vec<(i64, String)> = (1..=(MAX_SEEN as i64 + 20))
            .map(|i| (i, format!("https://x.com/u/status/{i}")))
            .collect();
        let refs: Vec<(i64, &str)> = many.iter().map(|(i, u)| (*i, u.as_str())).collect();
        state.apply(remote(&refs), 1);
        assert_eq!(state.seen.len(), MAX_SEEN);
        assert_eq!(state.seen[0], 21);
    }

    #[test]
    fn disconnecting_forgets_everything() {
        let mut state = QueueState::default();
        state.ensure_token();
        state.apply(remote(&[(1, REEL)]), 1);
        state.disconnect();
        assert_eq!(state, QueueState::default());
    }

    #[test]
    fn the_queue_survives_a_restart_and_a_damaged_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("queue.json");
        let store = QueueStore::new(path.clone());
        let token = store
            .update(|s| {
                s.apply(remote(&[(1, REEL)]), 7);
                s.ensure_token()
            })
            .unwrap();

        let reopened = QueueStore::new(path.clone()).snapshot();
        assert_eq!(reopened.token.as_deref(), Some(token.as_str()));
        assert_eq!(reopened.items.len(), 1);
        assert_eq!(reopened.last_refreshed, Some(7));
        // The ids already taken survive too, or a lost acknowledgement could bring links back.
        let mut again = reopened;
        assert_eq!(again.apply(remote(&[(1, REEL)]), 8).added, 0);

        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(QueueStore::new(path).snapshot(), QueueState::default());
    }
}
