//! The Telegram queue: the app reads a public Telegram channel of the user's own
//! (`https://t.me/s/<name>`) and queues the TikTok/Instagram/X links posted there.
//! See `fillerncut_core::queue`.

use fillerncut_core::queue::{parse_channel, parse_channel_page, ChannelPost, QueueItem};
use serde::Serialize;
use std::time::Duration;
use tauri::State;

use crate::download::USER_AGENT;
use crate::state::AppState;

const JOB: &str = "queue";
/// A page holds the newest ~20 posts. A refresh reads back until it reaches posts it has
/// already seen, but never more than this many pages, however long the app was closed.
const MAX_PAGES: usize = 5;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
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

fn view(state: &AppState) -> QueueView {
    let q = state.queue.snapshot();
    QueueView {
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

/// What is in the queue now, from disk. No network.
#[tauri::command]
pub fn queue_get(state: State<'_, AppState>) -> QueueView {
    view(&state)
}

/// Read a channel and start using it. A channel that can't be read isn't saved.
#[tauri::command]
pub async fn queue_set_channel(state: State<'_, AppState>, input: String) -> Result<RefreshResult, String> {
    let channel = parse_channel(&input).map_err(|e| e.to_string())?;
    let Ok(_guard) = state.begin_job(JOB) else {
        return Err("The queue is being updated. Try again in a moment.".into());
    };
    // Looking at a channel for the first time reads its latest page only.
    let first_look = state.queue.snapshot().channel.as_deref() != Some(channel.as_str());
    let already_read = if first_look {
        0
    } else {
        state.queue.snapshot().seen_up_to()
    };
    let posts = fetch_posts(&channel, already_read, if first_look { 1 } else { MAX_PAGES }).await?;
    let added = state
        .queue
        .update(|q| {
            q.set_channel(&channel);
            q.apply_posts(posts, now_unix())
        })
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(RefreshResult {
        view: view(&state),
        added,
    })
}

/// Pick up the links posted since last time. Does nothing until a channel is set.
#[tauri::command]
pub async fn queue_refresh(state: State<'_, AppState>) -> Result<RefreshResult, String> {
    let snapshot = state.queue.snapshot();
    let Some(channel) = snapshot.channel.clone() else {
        return Ok(RefreshResult {
            view: view(&state),
            added: 0,
        });
    };
    // Two refreshes at once (the timer and a click) would only repeat each other.
    let Ok(_guard) = state.begin_job(JOB) else {
        return Ok(RefreshResult {
            view: view(&state),
            added: 0,
        });
    };
    let pages = if snapshot.first_look() { 1 } else { MAX_PAGES };
    let posts = fetch_posts(&channel, snapshot.seen_up_to(), pages).await?;
    let added = state
        .queue
        .update(|q| {
            // The channel may have been changed while this was fetching.
            if q.channel.as_deref() == Some(channel.as_str()) {
                q.apply_posts(posts, now_unix())
            } else {
                0
            }
        })
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(RefreshResult {
        view: view(&state),
        added,
    })
}

/// Take a link out of the queue (it was opened, or dismissed).
#[tauri::command]
pub fn queue_remove(state: State<'_, AppState>, id: i64) -> Result<QueueView, String> {
    state
        .queue
        .update(|q| q.remove(id))
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(view(&state))
}

/// Stop reading the channel and forget its links. The channel itself isn't touched.
#[tauri::command]
pub fn queue_disconnect(state: State<'_, AppState>) -> Result<QueueView, String> {
    state
        .queue
        .update(|q| q.disconnect())
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(view(&state))
}
