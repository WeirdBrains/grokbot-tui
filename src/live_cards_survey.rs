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
