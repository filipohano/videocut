//! The Telegram queue: the app reads public Telegram channels of the user's own
//! (`https://t.me/s/<name>`), one queue per channel, and queues the TikTok/Instagram/X/YouTube
//! links posted there. See `fillerncut_core::queue`.

use fillerncut_core::queue::{parse_channel, parse_channel_page, ChannelPost, Queue, QueueItem};
use serde::Serialize;
use std::time::Duration;
use tauri::State;

use crate::download::USER_AGENT;
use crate::state::AppState;

/// Each queue has its own job name, so several queues can be refreshed at the same time.
fn job(id: u64) -> String {
    format!("queue-{id}")
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
    id: u64,
    channel: String,
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

fn view_of(q: Queue) -> QueueView {
    QueueView {
        id: q.id,
        channel: q.state.channel.unwrap_or_default(),
        items: q.state.items,
        last_refreshed: q.state.last_refreshed,
    }
}

fn view(state: &AppState, id: u64) -> Result<QueueView, String> {
    state
        .queue
        .get(id)
        .map(view_of)
        .ok_or_else(|| "That queue isn't there any more.".to_string())
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

/// What is in the queues now, from disk, in the order they were added. No network.
#[tauri::command]
pub fn queue_get(state: State<'_, AppState>) -> Vec<QueueView> {
    state.queue.list().into_iter().map(view_of).collect()
}

/// Read a channel and add a queue for it. A channel that can't be read isn't added.
#[tauri::command]
pub async fn queue_add(state: State<'_, AppState>, input: String) -> Result<RefreshResult, String> {
    let channel = parse_channel(&input).map_err(|e| e.to_string())?;
    state.queue.can_add(&channel).map_err(|e| e.to_string())?;
    // Adding the same channel twice at once (a double click) would only race.
    let Ok(_guard) = state.begin_job(&format!("queue-add-{}", channel.to_ascii_lowercase())) else {
        return Err(format!("t.me/{channel} is being added. Wait a moment."));
    };
    // A new channel picks up its newest posts only.
    let posts = newest(
        fetch_posts(&channel, 0, FIRST_LOOK_PAGES).await?,
        FIRST_LOOK_POSTS,
    );
    let (queue, added) = state
        .queue
        .add(&channel, |q| q.apply_posts(posts, now_unix()))
        .map_err(|e| e.to_string())?;
    let id = queue.id;
    Ok(RefreshResult {
        view: view(&state, id)?,
        added,
    })
}

/// Pick up the links posted in one queue's channel since last time.
#[tauri::command]
pub async fn queue_refresh(state: State<'_, AppState>, id: u64) -> Result<RefreshResult, String> {
    let snapshot = state.queue.get(id).ok_or("That queue isn't there any more.")?;
    let Some(channel) = snapshot.state.channel.clone() else {
        return Err("That queue isn't there any more.".into());
    };
    // Two refreshes of one queue at once (the timer and a click) would only repeat each other.
    let Ok(_guard) = state.begin_job(&job(id)) else {
        return Ok(RefreshResult {
            view: view(&state, id)?,
            added: 0,
        });
    };
    let first_look = snapshot.state.first_look();
    let pages = if first_look { FIRST_LOOK_PAGES } else { MAX_PAGES };
    let mut posts = fetch_posts(&channel, snapshot.state.seen_up_to(), pages).await?;
    if first_look {
        posts = newest(posts, FIRST_LOOK_POSTS);
    }
    let added = state
        .queue
        .update(id, |q| q.apply_posts(posts, now_unix()))
        .map_err(|e| e.to_string())?;
    Ok(RefreshResult {
        view: view(&state, id)?,
        added,
    })
}

/// Take a link out of a queue (it was opened, or dismissed).
#[tauri::command]
pub fn queue_remove_item(state: State<'_, AppState>, queue: u64, item: i64) -> Result<QueueView, String> {
    state
        .queue
        .update(queue, |q| q.remove(item))
        .map_err(|e| e.to_string())?;
    view(&state, queue)
}

/// Stop reading a channel and forget its links. The channel itself isn't touched.
#[tauri::command]
pub fn queue_delete(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    state.queue.remove(id).map_err(|e| e.to_string())
}
