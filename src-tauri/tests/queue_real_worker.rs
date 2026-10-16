//! The app's queue commands against the *real* worker (`queue-worker/`) running under
//! `wrangler dev`, with Telegram played by plain HTTP calls to the webhook. Skipped unless
//! `FILLERNCUT_REAL_QUEUE_URL` points at it; see queue-worker/README.md ("Trying it locally").

mod common;

use common::*;
use serde_json::{json, Value};

fn say(
    runtime: &tokio::runtime::Runtime,
    base: &str,
    secret: &str,
    chat: i64,
    name: &str,
    text: &str,
) -> Value {
    let entities: Vec<Value> = text
        .match_indices("http")
        .map(|(i, _)| {
            let rest = &text[i..];
            let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
            json!({ "type": "url", "offset": text[..i].encode_utf16().count(), "length": rest[..len].encode_utf16().count() })
        })
        .collect();
    let update = json!({ "update_id": 1, "message": {
        "chat": { "id": chat, "type": "private" }, "from": { "first_name": name },
        "text": text, "entities": entities } });
    runtime.block_on(async {
        let response = reqwest::Client::new()
            .post(format!("{base}/telegram/webhook"))
            .header("x-telegram-bot-api-secret-token", secret)
            .body(update.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        serde_json::from_str(&response.text().await.unwrap()).unwrap()
    })
}

fn code_from(link: &str) -> &str {
    link.rsplit("start=").next().unwrap()
}

#[test]
fn two_people_with_the_real_worker_each_get_only_their_own_links() {
    let Ok(base) = std::env::var("FILLERNCUT_REAL_QUEUE_URL") else {
        eprintln!("skipping: FILLERNCUT_REAL_QUEUE_URL is not set");
        return;
    };
    let secret = std::env::var("FILLERNCUT_REAL_QUEUE_SECRET").unwrap_or_else(|_| "local-secret".into());
    let (Some(a), Some(b)) = (harness(), harness()) else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // Chat ids that no other run used, so the test can be repeated against the same database.
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (chat_a, chat_b, stranger) = (salt * 10 + 1, salt * 10 + 2, salt * 10 + 3);

    for app in [&a, &b] {
        let mut s = call(&app.win, "get_settings", json!({})).unwrap();
        s["queueServer"] = json!(base);
        call(&app.win, "save_settings", json!({ "settings": s })).unwrap();
    }
    let pair_a = call(&a.win, "queue_pair", json!({})).unwrap();
    let pair_b = call(&b.win, "queue_pair", json!({})).unwrap();
    assert_eq!(pair_a["bot"], pair_b["bot"]);
    assert!(pair_a["link"].as_str().unwrap().starts_with("https://t.me/"));

    let hello_a = say(
        &runtime,
        &base,
        &secret,
        chat_a,
        "Filip",
        &format!("/start {}", code_from(pair_a["link"].as_str().unwrap())),
    );
    let hello_b = say(
        &runtime,
        &base,
        &secret,
        chat_b,
        "Mari",
        &format!("/start {}", code_from(pair_b["link"].as_str().unwrap())),
    );
    assert!(
        hello_a["text"].as_str().unwrap().contains("Connected"),
        "{hello_a}"
    );
    assert!(
        hello_b["text"].as_str().unwrap().contains("Connected"),
        "{hello_b}"
    );

    let reel = "https://www.instagram.com/reel/C1a2B3c4D5e/";
    let tiktok = "https://www.tiktok.com/@someone/video/7123456789012345678";
    say(&runtime, &base, &secret, chat_a, "Filip", &format!("see {reel}"));
    say(&runtime, &base, &secret, chat_b, "Mari", tiktok);
    let nobody = say(
        &runtime,
        &base,
        &secret,
        stranger,
        "Eve",
        "https://x.com/jack/status/20",
    );
    assert!(
        nobody["text"].as_str().unwrap().contains("isn't connected"),
        "{nobody}"
    );

    let from_a = call(&a.win, "queue_refresh", json!({})).unwrap();
    let from_b = call(&b.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(from_a["added"], 1);
    assert_eq!(from_a["view"]["items"][0]["url"], reel);
    assert_eq!(from_a["view"]["linked"][0]["label"], "Filip");
    assert_eq!(from_b["added"], 1);
    assert_eq!(from_b["view"]["items"][0]["url"], tiktok);
    assert_eq!(from_b["view"]["linked"][0]["label"], "Mari");

    // Handed over and acknowledged: the server has nothing left, the app still has them.
    let again = call(&a.win, "queue_refresh", json!({})).unwrap();
    assert_eq!(again["added"], 0);
    assert_eq!(again["view"]["items"].as_array().unwrap().len(), 1);

    // Disconnecting stops the chat from reaching the app.
    call(&a.win, "queue_disconnect", json!({})).unwrap();
    let after = say(&runtime, &base, &secret, chat_a, "Filip", tiktok);
    assert!(
        after["text"].as_str().unwrap().contains("isn't connected"),
        "{after}"
    );
}
