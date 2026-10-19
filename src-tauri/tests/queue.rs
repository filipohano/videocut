//! The Telegram queue commands end to end against a local stand-in for `t.me/s/<channel>`
//! (`FILLERNCUT_TELEGRAM_HOST`). Every test uses a channel of its own on the one shared server.

mod common;

use common::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};

/// How many posts a page holds. Telegram's is about 20; a small number shows the paging.
const PAGE_SIZE: usize = 5;

const REEL: &str = "https://www.instagram.com/reel/C1a2B3c4D5e/";
const TIKTOK: &str = "https://www.tiktok.com/@someone/video/7123456789012345678";
const TWEET: &str = "https://x.com/jack/status/20";

#[derive(Default)]
struct Channel {
    /// (post number, message text as HTML)
    posts: Vec<(i64, String)>,
    requests: usize,
    /// Answer with this status instead of the page.
    status: Option<u16>,
    /// Hang up without answering.
    hang_up: bool,
}

struct Server {
    channels: Mutex<HashMap<String, Channel>>,
}

fn server() -> &'static Server {
    static S: OnceLock<&'static Server> = OnceLock::new();
    S.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server: &'static Server = Box::leak(Box::new(Server {
            channels: Mutex::new(HashMap::new()),
        }));
        std::env::set_var("FILLERNCUT_TELEGRAM_HOST", &host);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || serve(stream, server));
            }
        });
        server
    })
}

fn message(id: i64, name: &str, body: &str) -> String {
    format!(
        r#"<div class="tgme_widget_message_wrap js-widget_message_wrap"><div class="tgme_widget_message text_not_supported_wrap js-widget_message" data-post="{name}/{id}" data-view="abc">
  <div class="tgme_widget_message_bubble"><div class="tgme_widget_message_text js-message_text" dir="auto">{body}</div>
  <div class="tgme_widget_message_footer"><a class="tgme_widget_message_date" href="https://t.me/{name}/{id}"><time datetime="2026-01-02T03:{:02}:00+00:00" class="time">08:00</time></a></div></div></div></div>"#,
        id % 60
    )
}

fn link(url: &str) -> String {
    format!(
        r#"<a href="{url}" target="_blank" rel="noopener" onclick="return confirm('Open this link?\n\n'+this.href);">{url}</a>"#
    )
}

fn serve(mut stream: std::net::TcpStream, server: &Server) {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]).into_owned();
    let target = request.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let before: Option<i64> = query.strip_prefix("before=").and_then(|v| v.parse().ok());
    let name = path.strip_prefix("/s/").unwrap_or("").to_string();

    let mut channels = server.channels.lock().unwrap();
    let (status, body) = match channels.get_mut(&name) {
        Some(c) if c.hang_up => {
            c.requests += 1;
            return;
        }
        Some(c) => {
            c.requests += 1;
            if let Some(status) = c.status {
                (status, "error".to_string())
            } else {
                let mut shown: Vec<&(i64, String)> = c
                    .posts
                    .iter()
                    .filter(|(id, _)| before.is_none_or(|b| *id < b))
                    .collect();
                shown.sort_by_key(|(id, _)| *id);
                let start = shown.len().saturating_sub(PAGE_SIZE);
                let page = &shown[start..];
                let more = page
                    .first()
                    .filter(|(id, _)| *id > 1)
                    .map(|(id, _)| format!(r#"<a href="/s/{name}?before={id}" class="tme_messages_more js-messages_more" data-before="{id}"></a>"#))
                    .unwrap_or_default();
                let html: String = page.iter().map(|(id, body)| message(*id, &name, body)).collect();
                (200, format!("<html><body><section class=\"tgme_channel_history\">{more}{html}</section></body></html>"))
            }
        }
        // A channel that doesn't exist, or isn't public: Telegram shows a plain page without posts.
        None => (
            200,
            r#"<html><body><div class="tgme_page_title">Some name</div></body></html>"#.to_string(),
        ),
    };
    drop(channels);
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// Make (or reset) a channel and give it posts 1..=n, each with one link from `links`.
fn open_channel(name: &str, posts: Vec<(i64, String)>) {
    server().channels.lock().unwrap().insert(
        name.to_string(),
        Channel {
            posts,
            ..Channel::default()
        },
    );
}

fn post_link(name: &str, id: i64, url: &str) {
    server()
        .channels
        .lock()
        .unwrap()
        .get_mut(name)
        .unwrap()
        .posts
        .push((id, link(url)));
}

fn requests(name: &str) -> usize {
    server().channels.lock().unwrap()[name].requests
}

/// A distinct link per number, so every post can be told apart.
fn tweet(n: i64) -> String {
    format!("https://x.com/user/status/{n}")
}

fn urls(view: &Value) -> Vec<String> {
    view["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["url"].as_str().unwrap().to_string())
        .collect()
}

fn set(h: &Harness, channel: &str) -> Result<Value, Value> {
    call(&h.win, "queue_set_channel", json!({ "input": channel }))
}

#[test]
fn a_new_queue_is_empty_and_not_connected() {
    let Some(h) = harness() else { return };
    let view = call(&h.win, "queue_get", json!({})).unwrap();
    assert!(view["channel"].is_null());
    assert_eq!(view["items"].as_array().unwrap().len(), 0);
    // Refreshing without a channel is not an error: there is just nothing to read.
    let refreshed = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(refreshed["added"], 0);
}

#[test]
fn a_channel_is_read_and_its_links_queued_in_the_order_they_were_posted() {
    let Some(h) = harness() else { return };
    server();
    open_channel(
        "inorder_links",
        vec![
            (1, link(REEL)),
            (2, "just a note".into()),
            (3, format!("see {} and {}", link(TIKTOK), link(TWEET))),
        ],
    );

    let result = set(&h, "https://t.me/inorder_links").unwrap();
    assert_eq!(result["added"], 3);
    let view = &result["view"];
    assert_eq!(view["channel"], "inorder_links");
    assert_eq!(urls(view), vec![REEL, TIKTOK, TWEET]);
    assert_eq!(view["items"][0]["platform"], "instagram");
    assert_eq!(view["items"][1]["platform"], "tiktok");
    assert_eq!(view["items"][2]["platform"], "twitter");
    // Post 1 was posted 2026-01-02 03:01 UTC: the time of the post, not of the refresh.
    assert_eq!(view["items"][0]["addedAt"], 1_767_322_860i64);
    assert!(view["lastRefreshed"].as_i64().unwrap() > 1_700_000_000);
}

#[test]
fn only_new_posts_are_added_later_and_paging_follows_back_to_where_it_stopped() {
    let Some(h) = harness() else { return };
    server();
    let name = "paging_links";
    open_channel(name, (1..=3).map(|n| (n, link(&tweet(n)))).collect());
    assert_eq!(set(&h, name).unwrap()["added"], 3);
    assert_eq!(requests(name), 1);

    // Nothing new: one request, nothing added.
    assert_eq!(call(&h.win, "queue_refresh", json!({})).unwrap()["added"], 0);
    assert_eq!(requests(name), 2);

    // 12 new posts: three pages of five are needed to get back to post 3.
    for n in 4..=15 {
        post_link(name, n, &tweet(n));
    }
    let refreshed = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(refreshed["added"], 12);
    assert_eq!(requests(name), 2 + 3);
    let all = urls(&refreshed["view"]);
    assert_eq!(all.len(), 15);
    assert_eq!(
        all,
        (1..=15).map(tweet).collect::<Vec<_>>(),
        "in order and none twice"
    );
}

#[test]
fn a_refresh_never_reads_more_than_five_pages() {
    let Some(h) = harness() else { return };
    server();
    let name = "many_links";
    open_channel(name, vec![(1, link(&tweet(1)))]);
    set(&h, name).unwrap();
    let before = requests(name);
    for n in 2..=100 {
        post_link(name, n, &tweet(n));
    }
    let refreshed = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(requests(name) - before, 5);
    // The newest 25 were read; the app doesn't go digging through all of history.
    assert_eq!(refreshed["added"], 25);
    assert_eq!(urls(&refreshed["view"]).last().unwrap(), &tweet(100));
}

#[test]
fn connecting_to_an_old_channel_reads_its_latest_page_only() {
    let Some(h) = harness() else { return };
    server();
    let name = "old_channel";
    open_channel(name, (1..=40).map(|n| (n, link(&tweet(n)))).collect());
    let result = set(&h, name).unwrap();
    assert_eq!(result["added"], PAGE_SIZE as i64);
    assert_eq!(requests(name), 1);
    assert_eq!(urls(&result["view"]), (36..=40).map(tweet).collect::<Vec<_>>());
}

#[test]
fn a_link_that_was_opened_or_removed_does_not_come_back() {
    let Some(h) = harness() else { return };
    server();
    let name = "removed_links";
    open_channel(name, vec![(1, link(REEL)), (2, link(TIKTOK))]);
    let view = set(&h, name).unwrap()["view"].clone();
    let first = view["items"][0]["id"].as_i64().unwrap();
    let after = call(&h.win, "queue_remove", json!({ "id": first })).unwrap();
    assert_eq!(urls(&after), vec![TIKTOK]);

    let refreshed = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(refreshed["added"], 0);
    assert_eq!(urls(&refreshed["view"]), vec![TIKTOK]);

    // Posting the same link again is a new post, and queues it again.
    post_link(name, 3, REEL);
    let again = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(again["added"], 1);
    assert_eq!(urls(&again["view"]), vec![TIKTOK, REEL]);
}

#[test]
fn a_channel_that_cannot_be_read_is_explained_and_not_saved() {
    let Some(h) = harness() else { return };
    server();
    // Doesn't exist, or isn't public: Telegram shows no posts.
    let err = set(&h, "@no_such_channel").unwrap_err();
    assert!(err.as_str().unwrap().contains("public"), "{err}");
    open_channel("empty_channel", vec![]);
    assert!(
        set(&h, "empty_channel").is_err(),
        "a channel without posts can't be checked"
    );
    assert!(call(&h.win, "queue_get", json!({})).unwrap()["channel"].is_null());

    // Names that aren't channels never reach the network.
    for (input, word) in [
        ("my channel", "isn't a Telegram channel"),
        ("https://t.me/+AbCdEfGhIjK", "public"),
        ("", "name of your Telegram channel"),
    ] {
        let err = set(&h, input).unwrap_err();
        assert!(err.as_str().unwrap().contains(word), "{input:?}: {err}");
    }
}

#[test]
fn trouble_reaching_telegram_is_explained_and_loses_nothing() {
    let Some(h) = harness() else { return };
    server();
    let name = "flaky_links";
    open_channel(name, vec![(1, link(REEL))]);
    set(&h, name).unwrap();
    post_link(name, 2, TIKTOK);

    server().channels.lock().unwrap().get_mut(name).unwrap().status = Some(503);
    let err = call(&h.win, "queue_refresh", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("503"), "{err}");
    {
        let mut channels = server().channels.lock().unwrap();
        let c = channels.get_mut(name).unwrap();
        c.status = None;
        c.hang_up = true;
    }
    let err = call(&h.win, "queue_refresh", json!({})).unwrap_err();
    assert!(err.as_str().unwrap().contains("Couldn't reach Telegram"), "{err}");
    assert_eq!(
        urls(&call(&h.win, "queue_get", json!({})).unwrap()),
        vec![REEL],
        "what was already there stays"
    );

    // Once it works again the missed post arrives.
    server().channels.lock().unwrap().get_mut(name).unwrap().hang_up = false;
    let refreshed = call(&h.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(refreshed["added"], 1);
    assert_eq!(urls(&refreshed["view"]), vec![REEL, TIKTOK]);
}

#[test]
fn two_people_only_ever_see_their_own_channel() {
    let (Some(a), Some(b)) = (harness(), harness()) else {
        return;
    };
    server();
    open_channel("filips_links", vec![(1, link(REEL))]);
    open_channel("maris_links", vec![(1, link(TIKTOK))]);
    let from_a = set(&a, "@filips_links").unwrap();
    let from_b = set(&b, "@maris_links").unwrap();
    assert_eq!(urls(&from_a["view"]), vec![REEL]);
    assert_eq!(urls(&from_b["view"]), vec![TIKTOK]);

    post_link("filips_links", 2, TWEET);
    assert_eq!(
        urls(&call(&a.win, "queue_refresh", json!({})).unwrap()["view"]),
        vec![REEL, TWEET]
    );
    assert_eq!(
        urls(&call(&b.win, "queue_refresh", json!({})).unwrap()["view"]),
        vec![TIKTOK]
    );
}

#[test]
fn changing_channel_starts_a_fresh_queue_and_disconnecting_forgets_everything() {
    let Some(h) = harness() else { return };
    server();
    open_channel("first_channel", vec![(1, link(REEL))]);
    open_channel("second_channel", vec![(1, link(TIKTOK))]);
    set(&h, "first_channel").unwrap();
    let changed = set(&h, "second_channel").unwrap();
    assert_eq!(urls(&changed["view"]), vec![TIKTOK]);

    let view = call(&h.win, "queue_disconnect", json!({})).unwrap();
    assert!(view["channel"].is_null());
    assert_eq!(view["items"].as_array().unwrap().len(), 0);
    assert_eq!(call(&h.win, "queue_refresh", json!({})).unwrap()["added"], 0);
    // Connecting to the same channel again starts over and finds its latest posts.
    assert_eq!(set(&h, "second_channel").unwrap()["added"], 1);
}

#[test]
fn the_queue_is_kept_on_disk_with_nothing_secret_in_it() {
    let Some(h) = harness() else { return };
    server();
    open_channel("saved_links", vec![(1, link(REEL))]);
    set(&h, "saved_links").unwrap();
    let text = std::fs::read_to_string(h.dir.path().join("data/queue.json")).unwrap();
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["channel"], "saved_links");
    assert_eq!(saved["items"][0]["url"], REEL);
    assert!(saved.get("token").is_none(), "{text}");
}

#[test]
fn only_a_channel_address_can_be_opened_besides_the_projects_pages() {
    let Some(h) = harness() else { return };
    for bad in [
        "https://evil.example/",
        "https://t.me/",
        "https://t.me/my_links?start=a&x=1",
        "file:///etc/passwd",
        "https://t.me.evil.example/my_links",
    ] {
        let err = call(&h.win, "open_url", json!({ "url": bad })).unwrap_err();
        assert!(err.as_str().unwrap().contains("isn't allowed"), "{bad}: {err}");
    }
}
