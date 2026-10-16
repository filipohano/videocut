//! The Telegram queue commands end to end against a local stand-in for the queue server,
//! which (like the real one) keeps a separate queue for every secret token it is given.

mod common;

use common::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Server {
    /// token → links waiting for that installation.
    queues: HashMap<String, Vec<(i64, String)>>,
    next_id: i64,
    /// Every call as "METHOD /path".
    calls: Vec<String>,
    tokens_seen: Vec<String>,
    acknowledged: Vec<i64>,
    /// Keep sending acknowledged items again, as if the acknowledgement never arrived.
    forget_acks: bool,
    /// Answer everything with this status.
    status: Option<u16>,
    garbage: bool,
}

type Shared = Arc<Mutex<Server>>;

impl Server {
    fn send(&mut self, token: &str, url: &str) {
        self.next_id += 1;
        let id = self.next_id;
        self.queues
            .entry(token.to_string())
            .or_default()
            .push((id, url.to_string()));
    }
}

fn spawn_server() -> (String, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Server::default()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let state = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            std::thread::spawn(move || serve(stream, state));
        }
    });
    (address, shared)
}

fn serve(mut stream: std::net::TcpStream, state: Shared) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let (head, mut body) = loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            return;
        }
        raw.extend_from_slice(&buf[..n]);
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break (
                String::from_utf8_lossy(&raw[..end]).into_owned(),
                raw[end + 4..].to_vec(),
            );
        }
    };
    let length = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    while body.len() < length {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    let mut first = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let token = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("authorization: Bearer ")
                .or_else(|| l.strip_prefix("Authorization: Bearer "))
        })
        .unwrap_or("")
        .trim()
        .to_string();

    let mut s = state.lock().unwrap();
    s.calls.push(format!("{method} {path}"));
    s.tokens_seen.push(token.clone());
    let (status, text) = if let Some(code) = s.status {
        (code, json!({ "ok": false }).to_string())
    } else if s.garbage {
        (200, "<html>not the queue</html>".to_string())
    } else if token.len() < 32 {
        (
            401,
            json!({ "ok": false, "error": "missing or invalid token" }).to_string(),
        )
    } else {
        match (method, path) {
            ("GET", "/v1/queue") => {
                let items: Vec<Value> = s
                    .queues
                    .get(&token)
                    .map(|q| {
                        q.iter()
                            .map(|(id, url)| json!({ "id": id, "url": url, "at": 1_800_000_000 + id }))
                            .collect()
                    })
                    .unwrap_or_default();
                let linked = if s.queues.contains_key(&token) {
                    json!([{ "label": "Filip", "linkedAt": 1_800_000_000 }])
                } else {
                    json!([])
                };
                (
                    200,
                    json!({ "ok": true, "bot": "FillernBot", "linked": linked, "items": items }).to_string(),
                )
            }
            ("POST", "/v1/ack") => {
                let ids: Vec<i64> = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|v| v["ids"].as_array().cloned())
                    .map(|a| a.iter().filter_map(Value::as_i64).collect())
                    .unwrap_or_default();
                s.acknowledged.extend(&ids);
                if !s.forget_acks {
                    if let Some(q) = s.queues.get_mut(&token) {
                        q.retain(|(id, _)| !ids.contains(id));
                    }
                }
                (200, json!({ "ok": true, "removed": ids.len() }).to_string())
            }
            ("POST", "/v1/pair") => {
                s.queues.entry(token.clone()).or_default();
                (
                    200,
                    json!({ "ok": true, "bot": "FillernBot", "code": "ABCD2345",
                        "link": "https://t.me/FillernBot?start=ABCD2345", "expiresAt": 1_800_000_600 })
                    .to_string(),
                )
            }
            ("POST", "/v1/unlink") => {
                s.queues.remove(&token);
                (200, json!({ "ok": true }).to_string())
            }
            _ => (404, json!({ "ok": false }).to_string()),
        }
    };
    drop(s);
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}

fn use_server(h: &Harness, address: &str) {
    let mut s = call(&h.win, "get_settings", json!({})).unwrap();
    s["queueServer"] = json!(address);
    call(&h.win, "save_settings", json!({ "settings": s })).unwrap();
}

const REEL: &str = "https://www.instagram.com/reel/C1a2B3c4D5e/";
const TIKTOK: &str = "https://www.tiktok.com/@someone/video/7123456789012345678";

fn token_of(h: &Harness) -> String {
    let file = std::fs::read_to_string(h.dir.path().join("data/queue.json")).unwrap();
    serde_json::from_str::<Value>(&file).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn without_a_server_the_queue_explains_what_is_missing() {
    let Some(h) = harness() else { return };
    if fillerncut_core::queue::built_in_server().is_some() {
        return; // this build has one built in
    }
    let view = call(&h.win, "queue_get", json!({})).unwrap();
    assert_eq!(view["configured"], false);
    assert_eq!(view["connected"], false);
    let err = call(&h.win, "queue_pair", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("Settings"), "{err}");
    // Nothing to fetch yet, and no complaint about it.
    let refreshed = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(refreshed["added"], 0);
}

#[test]
fn the_server_address_is_a_setting_and_must_be_sound() {
    let Some(h) = harness() else { return };
    let (address, _) = spawn_server();
    use_server(&h, &format!("  {address}/ "));
    let saved = call(&h.win, "get_settings", json!({})).unwrap();
    assert_eq!(saved["queueServer"], address.as_str(), "trimmed");
    assert_eq!(call(&h.win, "queue_get", json!({})).unwrap()["configured"], true);

    use_server(&h, "http://not-local.example.dev");
    assert_eq!(call(&h.win, "queue_get", json!({})).unwrap()["configured"], false);
    let err = call(&h.win, "queue_pair", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("address"), "{err}");
    use_server(&h, "   ");
    assert!(call(&h.win, "get_settings", json!({})).unwrap()["queueServer"].is_null());
}

#[test]
fn connecting_makes_a_secret_once_and_returns_the_link_for_the_bot() {
    let Some(h) = harness() else { return };
    let (address, server) = spawn_server();
    use_server(&h, &address);

    let pair = call(&h.win, "queue_pair", json!({})).unwrap();
    assert_eq!(pair["code"], "ABCD2345");
    assert_eq!(pair["link"], "https://t.me/FillernBot?start=ABCD2345");
    assert_eq!(pair["bot"], "FillernBot");
    assert_eq!(call(&h.win, "queue_get", json!({})).unwrap()["connected"], true);

    let token = token_of(&h);
    assert_eq!(token.len(), 64);
    call(&h.win, "queue_pair", json!({})).unwrap();
    assert_eq!(token_of(&h), token, "the same secret is kept");
    let s = server.lock().unwrap();
    assert_eq!(s.calls, vec!["POST /v1/pair", "POST /v1/pair"]);
    assert!(s.tokens_seen.iter().all(|t| *t == token));
}

#[test]
fn refresh_picks_up_links_acknowledges_them_and_keeps_them() {
    let Some(h) = harness() else { return };
    let (address, server) = spawn_server();
    use_server(&h, &address);
    call(&h.win, "queue_pair", json!({})).unwrap();
    let token = token_of(&h);
    {
        let mut s = server.lock().unwrap();
        s.send(&token, REEL);
        s.send(&token, TIKTOK);
    }

    let first = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(first["added"], 2);
    let items = first["view"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["url"], REEL);
    assert_eq!(items[0]["platform"], "instagram");
    assert_eq!(items[1]["platform"], "tiktok");
    assert_eq!(first["view"]["linked"][0]["label"], "Filip");
    assert!(first["view"]["lastRefreshed"].as_i64().unwrap() > 1_700_000_000);
    assert_eq!(server.lock().unwrap().acknowledged, vec![1, 2]);

    // Nothing new: nothing added, and the links are still there.
    let second = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(second["added"], 0);
    assert_eq!(second["view"]["items"].as_array().unwrap().len(), 2);

    // They're on disk, so a restart still has them.
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(h.dir.path().join("data/queue.json")).unwrap())
            .unwrap();
    assert_eq!(on_disk["items"].as_array().unwrap().len(), 2);
}

#[test]
fn a_link_the_user_dealt_with_does_not_come_back_when_an_acknowledgement_is_lost() {
    let Some(h) = harness() else { return };
    let (address, server) = spawn_server();
    use_server(&h, &address);
    call(&h.win, "queue_pair", json!({})).unwrap();
    let token = token_of(&h);
    {
        let mut s = server.lock().unwrap();
        s.forget_acks = true;
        s.send(&token, REEL);
    }
    let first = call(&h.win, "queue_refresh", json!({})).unwrap();
    let id = first["view"]["items"][0]["id"].as_i64().unwrap();
    let removed = call(&h.win, "queue_remove", json!({ "id": id })).unwrap();
    assert_eq!(removed["items"].as_array().unwrap().len(), 0);

    // The server still holds it and sends it again.
    let again = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(again["added"], 0);
    assert_eq!(again["view"]["items"].as_array().unwrap().len(), 0);
}

#[test]
fn two_installations_never_see_each_others_links() {
    let (Some(a), Some(b)) = (harness(), harness()) else {
        return;
    };
    let (address, server) = spawn_server();
    for app in [&a, &b] {
        use_server(app, &address);
        call(&app.win, "queue_pair", json!({})).unwrap();
    }
    let (ta, tb) = (token_of(&a), token_of(&b));
    assert_ne!(ta, tb, "every installation has its own secret");
    {
        let mut s = server.lock().unwrap();
        s.send(&ta, REEL);
        s.send(&tb, TIKTOK);
    }
    let from_a = call(&a.win, "queue_refresh", json!({})).unwrap();
    let from_b = call(&b.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(from_a["view"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(from_a["view"]["items"][0]["url"], REEL);
    assert_eq!(from_b["view"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(from_b["view"]["items"][0]["url"], TIKTOK);
}

#[test]
fn trouble_with_the_server_is_explained_and_never_loses_links() {
    let Some(h) = harness() else { return };
    let (address, server) = spawn_server();
    use_server(&h, &address);
    call(&h.win, "queue_pair", json!({})).unwrap();
    let token = token_of(&h);
    server.lock().unwrap().send(&token, REEL);
    call(&h.win, "queue_refresh", json!({})).unwrap();

    server.lock().unwrap().status = Some(401);
    let err = call(&h.win, "queue_refresh", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("Disconnect"), "{err}");
    server.lock().unwrap().status = Some(500);
    let err = call(&h.win, "queue_refresh", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("500"), "{err}");
    {
        let mut s = server.lock().unwrap();
        s.status = None;
        s.garbage = true;
    }
    let err = call(&h.win, "queue_refresh", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("address"), "{err}");

    // A server that isn't there at all.
    use_server(&h, "http://127.0.0.1:9");
    let err = call(&h.win, "queue_refresh", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("Couldn't reach"), "{err}");
    let view = call(&h.win, "queue_get", json!({})).unwrap();
    assert_eq!(
        view["items"].as_array().unwrap().len(),
        1,
        "what was already picked up stays"
    );
}

#[test]
fn disconnecting_tells_the_server_and_starts_over_with_a_new_secret() {
    let Some(h) = harness() else { return };
    let (address, server) = spawn_server();
    use_server(&h, &address);
    call(&h.win, "queue_pair", json!({})).unwrap();
    let old = token_of(&h);
    server.lock().unwrap().send(&old, REEL);
    call(&h.win, "queue_refresh", json!({})).unwrap();

    let view = call(&h.win, "queue_disconnect", json!({})).unwrap();
    assert_eq!(view["connected"], false);
    assert_eq!(view["items"].as_array().unwrap().len(), 0);
    assert!(server
        .lock()
        .unwrap()
        .calls
        .contains(&"POST /v1/unlink".to_string()));
    assert!(!server.lock().unwrap().queues.contains_key(&old));

    call(&h.win, "queue_pair", json!({})).unwrap();
    assert_ne!(token_of(&h), old);

    // Offline, disconnecting still works on this computer.
    use_server(&h, "http://127.0.0.1:9");
    let view = call(&h.win, "queue_disconnect", json!({})).unwrap();
    assert_eq!(view["connected"], false);
}

#[test]
fn only_the_telegram_bot_link_can_be_opened_besides_the_projects_pages() {
    let Some(h) = harness() else { return };
    for bad in [
        "https://evil.example/",
        "https://t.me/",
        "https://t.me/FillernBot?start=a&x=1",
        "file:///etc/passwd",
        "https://t.me.evil.example/FillernBot",
    ] {
        let err = call(&h.win, "open_url", json!({ "url": bad })).unwrap_err();
        assert!(err.as_str().unwrap().contains("isn't allowed"), "{bad}: {err}");
    }
}
