//! The Telegram queue commands end to end against a local stand-in for `t.me/s/<channel>`
//! (`FILLERNCUT_TELEGRAM_HOST`). Every test uses a channel of its own on the one shared server.

mod common;

use common::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};

/// How many posts a page holds unless a test says otherwise. Telegram's is about 20; a small
/// number shows the paging.
const PAGE_SIZE: usize = 5;

const REEL: &str = "https://www.instagram.com/reel/C1a2B3c4D5e/";
const TIKTOK: &str = "https://www.tiktok.com/@someone/video/7123456789012345678";
const TWEET: &str = "https://x.com/jack/status/20";

#[derive(Default)]
struct Channel {
    /// (post number, message text as HTML)
    posts: Vec<(i64, String)>,
    /// Posts per page; 0 means `PAGE_SIZE`.
    page_size: usize,
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
                let size = if c.page_size == 0 { PAGE_SIZE } else { c.page_size };
                let start = shown.len().saturating_sub(size);
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

/// Add a queue for a channel.
fn add(h: &Harness, channel: &str) -> Result<Value, Value> {
    call(&h.win, "queue_add", json!({ "input": channel }))
}

/// Add a queue and give back its id.
fn add_id(h: &Harness, channel: &str) -> u64 {
    add(h, channel).unwrap()["view"]["id"].as_u64().unwrap()
}

fn refresh(h: &Harness, id: u64) -> Result<Value, Value> {
    call(&h.win, "queue_refresh", json!({ "id": id }))
}

fn all(h: &Harness) -> Vec<Value> {
    call(&h.win, "queue_get", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

/// One queue, as `queue_get` reports it.
fn queue(h: &Harness, id: u64) -> Value {
    all(h)
        .into_iter()
        .find(|q| q["id"] == id)
        .expect("queue is there")
}

fn remove_item(h: &Harness, id: u64, item: i64) -> Value {
    call(&h.win, "queue_remove_item", json!({ "queue": id, "item": item })).unwrap()
}

#[test]
fn a_new_app_has_no_queues() {
    let Some(h) = harness() else { return };
    assert!(all(&h).is_empty());
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

    let result = add(&h, "https://t.me/inorder_links").unwrap();
    assert_eq!(result["added"], 3);
    let view = &result["view"];
    assert!(view["id"].as_u64().unwrap() > 0);
    assert_eq!(view["channel"], "inorder_links");
    assert_eq!(urls(view), vec![REEL, TIKTOK, TWEET]);
    assert_eq!(view["items"][0]["platform"], "instagram");
    assert_eq!(view["items"][1]["platform"], "tiktok");
    assert_eq!(view["items"][2]["platform"], "twitter");
    // Post 1 was posted 2026-01-02 03:01 UTC: the time of the post, not of the refresh.
    assert_eq!(view["items"][0]["addedAt"], 1_767_322_860i64);
    assert!(view["lastRefreshed"].as_i64().unwrap() > 1_700_000_000);
    assert_eq!(all(&h).len(), 1);
}

#[test]
fn only_new_posts_are_added_later_and_paging_follows_back_to_where_it_stopped() {
    let Some(h) = harness() else { return };
    server();
    let name = "paging_links";
    open_channel(name, (1..=3).map(|n| (n, link(&tweet(n)))).collect());
    let result = add(&h, name).unwrap();
    assert_eq!(result["added"], 3);
    let id = result["view"]["id"].as_u64().unwrap();
    assert_eq!(requests(name), 1);

    // Nothing new: one request, nothing added.
    assert_eq!(refresh(&h, id).unwrap()["added"], 0);
    assert_eq!(requests(name), 2);

    // 12 new posts: three pages of five are needed to get back to post 3.
    for n in 4..=15 {
        post_link(name, n, &tweet(n));
    }
    let refreshed = refresh(&h, id).unwrap();
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
    let id = add_id(&h, name);
    let before = requests(name);
    for n in 2..=100 {
        post_link(name, n, &tweet(n));
    }
    let refreshed = refresh(&h, id).unwrap();
    assert_eq!(requests(name) - before, 5);
    // The newest 25 were read; the app doesn't go digging through all of history.
    assert_eq!(refreshed["added"], 25);
    assert_eq!(urls(&refreshed["view"]).last().unwrap(), &tweet(100));
}

#[test]
fn adding_an_old_channel_picks_up_its_newest_posts_only() {
    let Some(h) = harness() else { return };
    server();
    let name = "old_channel";
    // 40 posts, five to a page: three pages are read, so the newest 15.
    open_channel(name, (1..=40).map(|n| (n, link(&tweet(n)))).collect());
    let result = add(&h, name).unwrap();
    let id = result["view"]["id"].as_u64().unwrap();
    assert_eq!(requests(name), 3);
    assert_eq!(urls(&result["view"]), (26..=40).map(tweet).collect::<Vec<_>>());
    // What is older than that is skipped for good, not trickled in later.
    post_link(name, 41, &tweet(41));
    let later = refresh(&h, id).unwrap();
    assert_eq!(later["added"], 1);
    assert_eq!(urls(&later["view"]).len(), 16);
}

#[test]
fn the_first_look_keeps_the_newest_fifty_of_what_it_read() {
    let Some(h) = harness() else { return };
    server();
    let name = "long_history";
    // Pages of 20 like Telegram's: three pages are 60 posts, and the newest 50 are kept.
    open_channel(name, (1..=200).map(|n| (n, link(&tweet(n)))).collect());
    server().channels.lock().unwrap().get_mut(name).unwrap().page_size = 20;
    let result = add(&h, name).unwrap();
    assert_eq!(requests(name), 3);
    assert_eq!(result["added"], 50);
    assert_eq!(urls(&result["view"]), (151..=200).map(tweet).collect::<Vec<_>>());
}

#[test]
fn a_short_channel_is_read_to_its_first_post() {
    let Some(h) = harness() else { return };
    server();
    let name = "short_channel";
    open_channel(name, (1..=12).map(|n| (n, link(&tweet(n)))).collect());
    let result = add(&h, name).unwrap();
    assert_eq!(result["added"], 12);
    assert_eq!(requests(name), 3, "5 + 5 + 2 posts, and it stops at post 1");
}

#[test]
fn a_link_that_was_opened_or_removed_does_not_come_back() {
    let Some(h) = harness() else { return };
    server();
    let name = "removed_links";
    open_channel(name, vec![(1, link(REEL)), (2, link(TIKTOK))]);
    let view = add(&h, name).unwrap()["view"].clone();
    let id = view["id"].as_u64().unwrap();
    let first = view["items"][0]["id"].as_i64().unwrap();
    let after = remove_item(&h, id, first);
    assert_eq!(after["id"], id);
    assert_eq!(urls(&after), vec![TIKTOK]);

    let refreshed = refresh(&h, id).unwrap();
    assert_eq!(refreshed["added"], 0);
    assert_eq!(urls(&refreshed["view"]), vec![TIKTOK]);

    // Posting the same link again is a new post, and queues it again.
    post_link(name, 3, REEL);
    let again = refresh(&h, id).unwrap();
    assert_eq!(again["added"], 1);
    assert_eq!(urls(&again["view"]), vec![TIKTOK, REEL]);
}

#[test]
fn a_channel_that_cannot_be_read_is_explained_and_not_added() {
    let Some(h) = harness() else { return };
    server();
    // Doesn't exist, or isn't public: Telegram shows no posts.
    let err = add(&h, "@no_such_channel").unwrap_err();
    assert!(err.as_str().unwrap().contains("public"), "{err}");
    open_channel("empty_channel", vec![]);
    assert!(
        add(&h, "empty_channel").is_err(),
        "a channel without posts can't be checked"
    );
    assert!(all(&h).is_empty());

    // Names that aren't channels never reach the network.
    for (input, word) in [
        ("my channel", "isn't a Telegram channel"),
        ("https://t.me/+AbCdEfGhIjK", "public"),
        ("", "name of your Telegram channel"),
    ] {
        let err = add(&h, input).unwrap_err();
        assert!(err.as_str().unwrap().contains(word), "{input:?}: {err}");
    }
}

#[test]
fn trouble_reaching_telegram_is_explained_and_loses_nothing() {
    let Some(h) = harness() else { return };
    server();
    let name = "flaky_links";
    open_channel(name, vec![(1, link(REEL))]);
    let id = add_id(&h, name);
    post_link(name, 2, TIKTOK);

    server().channels.lock().unwrap().get_mut(name).unwrap().status = Some(503);
    let err = refresh(&h, id).unwrap_err();
    assert!(err.as_str().unwrap().contains("503"), "{err}");
    {
        let mut channels = server().channels.lock().unwrap();
        let c = channels.get_mut(name).unwrap();
        c.status = None;
        c.hang_up = true;
    }
    let err = refresh(&h, id).unwrap_err();
    assert!(err.as_str().unwrap().contains("Couldn't reach Telegram"), "{err}");
    assert_eq!(urls(&queue(&h, id)), vec![REEL], "what was already there stays");

    // Once it works again the missed post arrives.
    server().channels.lock().unwrap().get_mut(name).unwrap().hang_up = false;
    let refreshed = refresh(&h, id).unwrap();
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
    let from_a = add(&a, "@filips_links").unwrap();
    let from_b = add(&b, "@maris_links").unwrap();
    assert_eq!(urls(&from_a["view"]), vec![REEL]);
    assert_eq!(urls(&from_b["view"]), vec![TIKTOK]);
    let (ida, idb) = (
        from_a["view"]["id"].as_u64().unwrap(),
        from_b["view"]["id"].as_u64().unwrap(),
    );

    post_link("filips_links", 2, TWEET);
    assert_eq!(urls(&refresh(&a, ida).unwrap()["view"]), vec![REEL, TWEET]);
    assert_eq!(urls(&refresh(&b, idb).unwrap()["view"]), vec![TIKTOK]);
    assert_eq!(all(&a).len(), 1);
    assert_eq!(all(&b).len(), 1);
}

#[test]
fn deleting_a_queue_forgets_it_and_adding_the_channel_again_starts_over() {
    let Some(h) = harness() else { return };
    server();
    open_channel("gone_channel", vec![(1, link(REEL))]);
    let id = add_id(&h, "gone_channel");
    call(&h.win, "queue_delete", json!({ "id": id })).unwrap();
    assert!(all(&h).is_empty());
    // Deleting it again, or using it, is explained rather than crashing.
    call(&h.win, "queue_delete", json!({ "id": id })).unwrap();
    for (cmd, body) in [
        ("queue_refresh", json!({ "id": id })),
        ("queue_remove_item", json!({ "queue": id, "item": 1 })),
    ] {
        let err = call(&h.win, cmd, body).unwrap_err();
        assert!(
            err.as_str().unwrap().contains("isn't there any more"),
            "{cmd}: {err}"
        );
    }
    // Adding the same channel again is a new queue that finds its latest posts.
    let again = add(&h, "gone_channel").unwrap();
    assert_eq!(again["added"], 1);
    assert_ne!(again["view"]["id"], id);
}

#[test]
fn the_queues_are_kept_on_disk_in_order_with_nothing_secret_in_them() {
    let Some(h) = harness() else { return };
    server();
    open_channel("disk_one", vec![(1, link(REEL))]);
    open_channel("disk_two", vec![(1, link(TIKTOK))]);
    add(&h, "disk_one").unwrap();
    add(&h, "disk_two").unwrap();
    let text = std::fs::read_to_string(h.dir.path().join("data/queue.json")).unwrap();
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["queues"].as_array().unwrap().len(), 2);
    assert_eq!(saved["queues"][0]["channel"], "disk_one");
    assert_eq!(saved["queues"][0]["items"][0]["url"], REEL);
    assert_eq!(saved["queues"][1]["items"][0]["url"], TIKTOK);
    assert!(!text.contains("token"), "{text}");
    let names: Vec<_> = all(&h).iter().map(|q| q["channel"].clone()).collect();
    assert_eq!(names, vec!["disk_one", "disk_two"]);
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

#[test]
fn many_queues_read_their_own_channels_independently() {
    let Some(h) = harness() else { return };
    server();
    let names = ["queue_one", "queue_two", "queue_three"];
    open_channel(names[0], vec![(1, link(REEL))]);
    open_channel(names[1], vec![(1, link(TIKTOK)), (2, link(TWEET))]);
    open_channel(names[2], vec![(1, link(&tweet(7)))]);
    let ids: Vec<u64> = names.iter().map(|n| add_id(&h, &format!("@{n}"))).collect();
    assert_eq!(all(&h).len(), 3);
    assert_eq!(urls(&queue(&h, ids[1])), vec![TIKTOK, TWEET]);

    // New posts in one channel only reach that queue, and each refresh reads only its own.
    let reads: Vec<usize> = names.iter().map(|n| requests(n)).collect();
    post_link(names[0], 2, &tweet(2));
    assert_eq!(refresh(&h, ids[0]).unwrap()["added"], 1);
    assert_eq!(requests(names[0]), reads[0] + 1);
    assert_eq!(requests(names[1]), reads[1], "the other queues weren't touched");
    assert_eq!(requests(names[2]), reads[2]);
    assert_eq!(urls(&queue(&h, ids[0])), vec![REEL, tweet(2).as_str()]);

    // Removing a link, and deleting a queue, only affect that one.
    let first = queue(&h, ids[1])["items"][0]["id"].as_i64().unwrap();
    assert_eq!(urls(&remove_item(&h, ids[1], first)), vec![TWEET]);
    assert_eq!(urls(&queue(&h, ids[0])).len(), 2);
    call(&h.win, "queue_delete", json!({ "id": ids[0] })).unwrap();
    let left: Vec<_> = all(&h).iter().map(|q| q["id"].as_u64().unwrap()).collect();
    assert_eq!(left, vec![ids[1], ids[2]]);
    assert_eq!(urls(&queue(&h, ids[1])), vec![TWEET]);
}

#[test]
fn several_queues_can_be_refreshed_at_the_same_time() {
    let Some(h) = harness() else { return };
    server();
    open_channel("parallel_one", vec![(1, link(REEL))]);
    open_channel("parallel_two", vec![(1, link(TIKTOK))]);
    let a = add_id(&h, "parallel_one");
    let b = add_id(&h, "parallel_two");
    post_link("parallel_one", 2, &tweet(2));
    post_link("parallel_two", 2, &tweet(3));
    // Not the same "job": one refresh must not turn the other into a no-op.
    let (win_a, win_b) = (h.win.clone(), h.win.clone());
    let a = std::thread::spawn(move || call(&win_a, "queue_refresh", json!({ "id": a })).unwrap());
    let b = std::thread::spawn(move || call(&win_b, "queue_refresh", json!({ "id": b })).unwrap());
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert_eq!(a["added"], 1);
    assert_eq!(b["added"], 1);
}

#[test]
fn a_channel_can_only_be_one_queue() {
    let Some(h) = harness() else { return };
    server();
    open_channel("Only_One_Chan", vec![(1, link(REEL))]);
    add(&h, "Only_One_Chan").unwrap();
    let reads = requests("Only_One_Chan");
    // The same name, however it is written (and Telegram ignores capitals).
    for input in ["Only_One_Chan", "@only_one_chan", "https://t.me/s/ONLY_ONE_CHAN"] {
        let err = add(&h, input).unwrap_err();
        assert!(
            err.as_str()
                .unwrap()
                .contains("t.me/Only_One_Chan is already one of your queues"),
            "{input}: {err}"
        );
    }
    assert_eq!(
        requests("Only_One_Chan"),
        reads,
        "refused before reading Telegram"
    );
    assert_eq!(all(&h).len(), 1);
}

#[test]
fn there_is_a_limit_on_how_many_queues_there_are() {
    let Some(h) = harness() else { return };
    server();
    for i in 0..20 {
        let name = format!("limit_chan_{i}");
        open_channel(&name, vec![(1, link(REEL))]);
        add(&h, &name).unwrap();
    }
    open_channel("limit_chan_extra", vec![(1, link(REEL))]);
    let err = add(&h, "limit_chan_extra").unwrap_err();
    assert!(err.as_str().unwrap().contains("at most 20 queues"), "{err}");
    assert_eq!(requests("limit_chan_extra"), 0);
    assert_eq!(all(&h).len(), 20);
}

#[test]
fn the_two_columns_of_the_last_version_are_kept_as_two_queues() {
    let Some(h) = harness_with_data(|data| {
        std::fs::write(
            data.join("queue.json"),
            r#"{"slots":[{"channel":"old_left","seenUpTo":5,"nextId":1,"items":[{"id":1,"url":"https://x.com/jack/status/20","platform":"twitter","addedAt":1}]},{"channel":"old_right"}]}"#,
        )
        .unwrap();
    }) else {
        return;
    };
    let queues = all(&h);
    assert_eq!(queues.len(), 2);
    assert_eq!(queues[0]["channel"], "old_left");
    assert_eq!(urls(&queues[0]), vec![TWEET]);
    assert_eq!(queues[1]["channel"], "old_right");
}
