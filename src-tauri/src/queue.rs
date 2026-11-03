//! The Telegram queue: the app reads up to two public Telegram channels of the user's own
//! (`https://t.me/s/<name>`), one per column, and queues the TikTok/Instagram/X/YouTube links
//! posted there. See `fillerncut_core::queue`.

use fillerncut_core::queue::{parse_channel, parse_channel_page, ChannelPost, QueueItem, SLOTS};
use serde::Serialize;
use std::time::Duration;
use tauri::State;

use crate::download::USER_AGENT;
use crate::state::AppState;

/// Each column has its own job name, so two columns can be refreshed at the same time.
fn job(slot: usize) -> String {
    format!("queue-{slot}")
}

/// A column number from the page; anything but a real column is an error, not a panic.
fn check_slot(slot: usize) -> Result<usize, String> {
    if slot < SLOTS {
        Ok(slot)
    } else {
        Err(format!("There are only {SLOTS} columns"))
    }
}
/// A page holds the newest ~20 posts. A refresh reads back until it reaches posts it has
/// already seen, but never more than this many pages, however long the app was closed.
const MAX_PAGES: usize = 5;
/// The first time a channel is read, this many of its newest posts are picked up (so a
/// channel with a long history doesn't fill the queue with old links)...
const FIRST_LOOK_POSTS: usize = 50;
/// ...which takes this many pages.
const FIRST_LOOK_PAGES: usize = 3;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    /// Which column this is.
    slot: usize,
    channel: Option<String>,
    items: Vec<QueueItem>,
    last_refreshed: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    view: QueueView,
    /// Links that were not in the queue before.
    added: usize,
}

fn view(state: &AppState, slot: usize) -> QueueView {
    let q = state.queue.snapshot(slot);
    QueueView {
        slot,
        channel: q.channel,
        items: q.items,
        last_refreshed: q.last_refreshed,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Telegram's web address. Overridable so the integration tests can point it at a local server.
fn telegram_host() -> String {
    std::env::var("FILLERNCUT_TELEGRAM_HOST").unwrap_or_else(|_| "https://t.me".to_string())
}

/// The newest posts of a channel, reading back until `already_read` (or `pages` pages).
async fn fetch_posts(channel: &str, already_read: i64, pages: usize) -> Result<Vec<ChannelPost>, String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut posts = Vec::new();
    let mut before: Option<i64> = None;
    for _ in 0..pages {
        let mut url = format!("{}/s/{channel}", telegram_host());
        if let Some(before) = before {
            url.push_str(&format!("?before={before}"));
        }
        let response = client.get(&url).send().await.map_err(|_| {
            "Couldn't reach Telegram. Check your internet connection and try again.".to_string()
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Telegram answered with an error ({}).", status.as_u16()));
        }
        let html = response
            .text()
            .await
            .map_err(|_| "Telegram cut the connection. Try again.".to_string())?;
        let page = parse_channel_page(&html);
        let Some(oldest) = page.iter().map(|p| p.id).min() else {
            if before.is_none() {
                return Err(format!(
                    "Couldn't read any posts from t.me/{channel}. The channel has to be public (it needs a username: Channel settings → Channel type → Public) and have at least one post."
                ));
            }
            break;
        };
        posts.extend(page);
        if oldest <= already_read + 1 || oldest <= 1 {
            break;
        }
        before = Some(oldest);
    }
    Ok(posts)
}

/// The `count` newest posts.
fn newest(mut posts: Vec<ChannelPost>, count: usize) -> Vec<ChannelPost> {
    posts.sort_by_key(|p| p.id);
    posts.dedup_by_key(|p| p.id);
    let skip = posts.len().saturating_sub(count);
    posts.split_off(skip)
}

/// What is in the queues now, from disk. No network.
#[tauri::command]
pub fn queue_get(state: State<'_, AppState>) -> Vec<QueueView> {
    (0..SLOTS).map(|slot| view(&state, slot)).collect()
}

/// Read a channel and start using it in a column. A channel that can't be read isn't saved.
#[tauri::command]
pub async fn queue_set_channel(
    state: State<'_, AppState>,
    slot: usize,
    input: String,
) -> Result<RefreshResult, String> {
    let slot = check_slot(slot)?;
    let mut channel = parse_channel(&input).map_err(|e| e.to_string())?;
    // Telegram doesn't care about capitals, so neither do we: the same name is the same channel.
    let current = state.queue.snapshot(slot).channel;
    if let Some(existing) = current.as_deref().filter(|c| c.eq_ignore_ascii_case(&channel)) {
        channel = existing.to_string();
    }
    for other in (0..SLOTS).filter(|o| *o != slot) {
        if state
            .queue
            .snapshot(other)
            .channel
            .is_some_and(|c| c.eq_ignore_ascii_case(&channel))
        {
            return Err(format!(
                "t.me/{channel} is already in the {} column. Each column needs its own channel.",
                column_name(other)
            ));
        }
    }
    let Ok(_guard) = state.begin_job(&job(slot)) else {
        return Err("This column is being updated. Try again in a moment.".into());
    };
    // Looking at a channel for the first time picks up its newest posts only.
    let first_look = current.as_deref() != Some(channel.as_str());
    let already_read = if first_look {
        0
    } else {
        state.queue.snapshot(slot).seen_up_to()
    };
    let pages = if first_look { FIRST_LOOK_PAGES } else { MAX_PAGES };
    let mut posts = fetch_posts(&channel, already_read, pages).await?;
    if first_look {
        posts = newest(posts, FIRST_LOOK_POSTS);
    }
    let added = state
        .queue
        .update(slot, |q| {
            q.set_channel(&channel);
            q.apply_posts(posts, now_unix())
        })
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(RefreshResult {
        view: view(&state, slot),
        added,
    })
}

/// Pick up the links posted since last time in one column. Does nothing until it has a channel.
#[tauri::command]
pub async fn queue_refresh(state: State<'_, AppState>, slot: usize) -> Result<RefreshResult, String> {
    let slot = check_slot(slot)?;
    let snapshot = state.queue.snapshot(slot);
    let Some(channel) = snapshot.channel.clone() else {
        return Ok(RefreshResult {
            view: view(&state, slot),
            added: 0,
        });
    };
    // Two refreshes of one column at once (the timer and a click) would only repeat each other.
    let Ok(_guard) = state.begin_job(&job(slot)) else {
        return Ok(RefreshResult {
            view: view(&state, slot),
            added: 0,
        });
    };
    let first_look = snapshot.first_look();
    let pages = if first_look { FIRST_LOOK_PAGES } else { MAX_PAGES };
    let mut posts = fetch_posts(&channel, snapshot.seen_up_to(), pages).await?;
    if first_look {
        posts = newest(posts, FIRST_LOOK_POSTS);
    }
    let added = state
        .queue
        .update(slot, |q| {
            // The channel may have been changed while this was fetching.
            if q.channel.as_deref() == Some(channel.as_str()) {
                q.apply_posts(posts, now_unix())
            } else {
                0
            }
        })
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(RefreshResult {
        view: view(&state, slot),
        added,
    })
}

/// Take a link out of a queue (it was opened, or dismissed).
#[tauri::command]
pub fn queue_remove(state: State<'_, AppState>, slot: usize, id: i64) -> Result<QueueView, String> {
    let slot = check_slot(slot)?;
    state
        .queue
        .update(slot, |q| q.remove(id))
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(view(&state, slot))
}

/// Stop reading a column's channel and forget its links. The channel itself isn't touched.
#[tauri::command]
pub fn queue_disconnect(state: State<'_, AppState>, slot: usize) -> Result<QueueView, String> {
    let slot = check_slot(slot)?;
    state
        .queue
        .update(slot, |q| q.disconnect())
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(view(&state, slot))
}

/// How a column is called in messages.
fn column_name(slot: usize) -> &'static str {
    if slot == 0 {
        "left"
    } else {
        "right"
    }
}
