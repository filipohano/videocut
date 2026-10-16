//! The Telegram link queue: links sent to the bot wait on a small server until the
//! app picks them up. See `fillerncut_core::queue` and `queue-worker/`.

use fillerncut_core::queue::{
    resolve_server, Applied, LinkedChat, PairInfo, QueueItem, QueueState, RemoteQueue,
};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use std::time::Duration;
use tauri::State;

use crate::state::AppState;

const JOB: &str = "queue";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueView {
    /// A queue server is known (built in or typed in Settings).
    configured: bool,
    /// This installation has made its secret, i.e. the user has started connecting Telegram.
    connected: bool,
    bot: Option<String>,
    linked: Vec<LinkedChat>,
    items: Vec<QueueItem>,
    last_refreshed: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResult {
    view: QueueView,
    /// Links that were not in the queue before this refresh.
    added: usize,
}

fn view(state: &AppState) -> QueueView {
    let settings = state.settings();
    let q = state.queue.snapshot();
    QueueView {
        configured: resolve_server(settings.queue_server.as_deref()).is_ok(),
        connected: q.token.is_some(),
        bot: q.bot,
        linked: q.linked,
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

/// One call to the queue server as this installation.
async fn call(
    state: &AppState,
    method: Method,
    path: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> Result<String, String> {
    let server = resolve_server(state.settings().queue_server.as_deref()).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut request = client
        .request(method, format!("{server}{path}"))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request.send().await.map_err(|_| {
        "Couldn't reach the queue server. Check your internet connection and try again.".to_string()
    })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "The queue server cut the connection.".to_string())?;
    match status {
        s if s.is_success() => Ok(text),
        StatusCode::UNAUTHORIZED => Err(
            "The queue server didn't accept this installation. Disconnect Telegram and connect again.".into(),
        ),
        s => Err(format!(
            "The queue server answered with an error ({}).",
            s.as_u16()
        )),
    }
}

fn unexpected() -> String {
    "The queue server gave an answer FillernCut doesn't understand. Is the address right?".into()
}

/// What is in the queue now, from disk. No network.
#[tauri::command]
pub fn queue_get(state: State<'_, AppState>) -> QueueView {
    view(&state)
}

/// Pick up new links from the server. Without a connection (the user hasn't connected
/// Telegram yet) this does nothing.
#[tauri::command]
pub async fn queue_refresh(state: State<'_, AppState>) -> Result<RefreshResult, String> {
    let Some(token) = state.queue.snapshot().token else {
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

    let text = call(&state, Method::GET, "/v1/queue", &token, None).await?;
    let remote: RemoteQueue = serde_json::from_str(&text).map_err(|_| unexpected())?;
    // Saved before the server is told, so a crash in between can't lose a link.
    let Applied { added, acknowledge } = state
        .queue
        .update(|q| q.apply(remote, now_unix()))
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    if !acknowledge.is_empty() {
        // If this fails the links simply come again, and are recognised.
        let _ = call(
            &state,
            Method::POST,
            "/v1/ack",
            &token,
            Some(serde_json::json!({ "ids": acknowledge })),
        )
        .await;
    }
    Ok(RefreshResult {
        view: view(&state),
        added,
    })
}

/// Start connecting a Telegram chat: a one-time code and the link that opens the bot with it.
#[tauri::command]
pub async fn queue_pair(state: State<'_, AppState>) -> Result<PairInfo, String> {
    resolve_server(state.settings().queue_server.as_deref()).map_err(|e| e.to_string())?;
    let token = state
        .queue
        .update(QueueState::ensure_token)
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    let text = call(&state, Method::POST, "/v1/pair", &token, None).await?;
    serde_json::from_str(&text).map_err(|_| unexpected())
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

/// Disconnect Telegram: the server forgets the chats and links, and this installation gets
/// a new secret next time. Works offline too; then the server's copy simply expires.
#[tauri::command]
pub async fn queue_disconnect(state: State<'_, AppState>) -> Result<QueueView, String> {
    if let Some(token) = state.queue.snapshot().token {
        let _ = call(&state, Method::POST, "/v1/unlink", &token, None).await;
    }
    state
        .queue
        .update(QueueState::disconnect)
        .map_err(|e| format!("Couldn't save the queue: {e}"))?;
    Ok(view(&state))
}
