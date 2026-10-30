//! Read-only survey of interactive card shapes in this account's chats (ignored; by hand):
//! `GROKBOT_TUI_LIVE_SURVEY=1 cargo test live_cards_survey -- --ignored --nocapture`.
//! Prints entry kinds, message types and the KEY STRUCTURE of non-text messages only
//! (string values replaced by their length). Never resolves or answers anything.
use serde_json::Value;
use std::collections::BTreeMap;

fn shape(v: &Value, depth: usize) -> Value {
    match v {
        Value::Object(m) if depth < 6 => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    // Keep short enum-like strings for a few structural keys.
                    let keep = matches!(
                        k.as_str(),
                        "type" | "kind" | "status" | "role" | "inputType" | "fieldType" | "variant"
                    );
                    let v = match v {
                        Value::String(s) if keep && s.len() < 40 => Value::String(s.clone()),
                        _ => shape(v, depth + 1),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(a) if depth < 6 => {
            Value::Array(a.iter().take(2).map(|x| shape(x, depth + 1)).collect())
        }
        Value::String(s) => Value::String(format!("<str {}>", s.len())),
        Value::Object(_) | Value::Array(_) => Value::String("<…>".into()),
        other => other.clone(),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: read-only transcript survey"]
async fn live_cards_survey() {
    assert_eq!(std::env::var("GROKBOT_TUI_LIVE_SURVEY").as_deref(), Ok("1"));
    let (_s, client) = crate::live_attach_test::live_client().await;
    let mut seen: BTreeMap<String, (usize, Value)> = BTreeMap::new();
    for a in client.list_agents().await.unwrap() {
        let Ok(t) = client.transcript_history(&a.id, 200, 2000).await else {
            println!("skip {}", a.name);
            continue;
        };
        for e in &t.entries {
            let kind = e
                .raw
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string();
            let mt = e
                .raw
                .pointer("/message/type")
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_string();
            let key = format!("{kind} / {mt}");
            let ent = seen.entry(key).or_insert((0, shape(&e.raw, 0)));
            ent.0 += 1;
        }
    }
    for (k, (n, s)) in &seen {
        println!("== {k}  ×{n}");
        if !k.ends_with("/ text") && !k.starts_with("user-prompt") {
            println!("{}", serde_json::to_string(s).unwrap());
        }
    }
}

/// Live end-to-end (ignored; by hand): find the ONE pending test widget whose prompt
/// contains `GROKBOT_TUI_LIVE_CARD` in the bot named `GROKBOT_TUI_LIVE_BOT`, open it
/// through the TUI's card modal and press the option key `GROKBOT_TUI_LIVE_KEY`.
/// Touches no other card.
#[test]
#[ignore = "live: answers one named test card"]
fn live_card_answer() {
    use crate::api::ApiClient;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let want = std::env::var("GROKBOT_TUI_LIVE_CARD").expect("card prompt");
    let bot_name = std::env::var("GROKBOT_TUI_LIVE_BOT").expect("bot name");
    let key: char = std::env::var("GROKBOT_TUI_LIVE_KEY")
        .unwrap()
        .chars()
        .next()
        .unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (_s, client) = rt.block_on(crate::live_attach_test::live_client());
    let api = std::sync::Arc::new(crate::api::GatewayApi::new(client));
    let bots = rt.block_on(api.list_bots()).unwrap();
    let bot = bots
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(&bot_name))
        .expect("bot")
        .clone();
    rt.block_on(api.transcript(&bot.id, 60)).unwrap();
    let cards: Vec<_> = api
        .take_cards(&bot.id)
        .into_iter()
        .filter(|c| c.state.is_pending() && c.kind == "widget" && c.summary().contains(&want))
        .collect();
    assert_eq!(cards.len(), 1, "exactly one pending test card");
    if let crate::cards::Body::Widget { steps, .. } = &cards[0].body {
        let labels: Vec<_> = steps[0].options.iter().map(|o| o.label.clone()).collect();
        println!("options: {labels:?}");
        if let Ok(l) = std::env::var("GROKBOT_TUI_LIVE_LABEL") {
            assert_eq!(
                labels[(key as u8 - b'1') as usize],
                l,
                "key picks the wanted label"
            );
        }
    }
    let (mut app, _d) = crate::app::tests::test_app(&rt);
    app.api = Some(api.clone());
    app.bots = bots.clone();
    let k = crate::app::App::card_key(&cards[0]);
    app.absorb_cards(&bot.id, cards);
    assert!(app.open_card(Some(&k)));
    app.card_modal_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
    for _ in 0..20 {
        app.tick();
        app.wait_idle(std::time::Duration::from_secs(5));
        if app.card_modal.is_none() {
            break;
        }
    }
    println!(
        "TUI state: {:?}",
        app.cards.get(&k).map(|(_, c)| c.state.clone())
    );
    if let Some(m) = &app.card_modal {
        panic!("modal still open: {:?}", m.error);
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    let t = rt
        .block_on(api.client.transcript_tail(&bot.id, 30, None))
        .unwrap();
    for e in t.entries.iter().rev().take(8).rev() {
        let r = &e.raw;
        println!(
            "entry kind={} type={} respondedValue={:?} role={:?} text={:?}",
            r.get("kind").and_then(Value::as_str).unwrap_or("?"),
            r.pointer("/message/type")
                .and_then(Value::as_str)
                .unwrap_or("-"),
            r.get("respondedValue"),
            r.get("role"),
            e.text.chars().take(80).collect::<String>()
        );
    }
    rt.block_on(api.transcript(&bot.id, 60)).unwrap();
    let after = api.take_cards(&bot.id);
    let c = after
        .iter()
        .find(|c| crate::app::App::card_key(c) == k)
        .expect("card after");
    println!("SERVER state: {:?}", c.state);
}
