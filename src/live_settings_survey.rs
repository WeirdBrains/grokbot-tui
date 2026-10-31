//! ONE live settings survey (ignored; run by hand only):
//! `GROKBOT_TUI_LIVE_SETTINGS=1 cargo test live_settings_survey -- --ignored --nocapture`.
//! Read-only: one raw `listAgents` call. Prints the record KEYS the gateway returns for
//! each agent (never values), so the Settings page knows which per-agent rows it can read.

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: one listAgents call"]
async fn live_settings_survey() {
    assert_eq!(
        std::env::var("GROKBOT_TUI_LIVE_SETTINGS").as_deref(),
        Ok("1")
    );
    let (_store, client) = crate::live_attach_test::live_client().await;
    let raw = client
        .call_raw("listAgents", serde_json::json!({}))
        .await
        .expect("listAgents");
    let mut keys = std::collections::BTreeSet::new();
    let mut top = std::collections::BTreeSet::new();
    if let Some(o) = raw.as_object() {
        top.extend(o.keys().cloned());
    }
    let list = raw
        .get("agents")
        .or_else(|| raw.get("data").and_then(|d| d.get("agents")))
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| raw.as_array().cloned())
        .unwrap_or_default();
    for a in &list {
        if let Some(o) = a.as_object() {
            keys.extend(o.keys().cloned());
        }
    }
    println!("top-level keys: {top:?}");
    println!("agents: {} · agent record keys: {keys:?}", list.len());
}

/// ONE live notification-source survey (ignored; by hand only):
/// `GROKBOT_TUI_LIVE_SETTINGS=1 cargo test live_notification_survey -- --ignored --nocapture`.
/// Read-only: `listAgents` + one `getAgentTranscriptTail` (200) per own bot. Prints entry
/// roles/types, their key sets, and how many entries mention usage/credits (counts only,
/// never message text).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: listAgents + transcript tails"]
async fn live_notification_survey() {
    assert_eq!(
        std::env::var("GROKBOT_TUI_LIVE_SETTINGS").as_deref(),
        Ok("1")
    );
    let (_store, client) = crate::live_attach_test::live_client().await;
    let agents = client.list_agents().await.expect("listAgents");
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    let mut keys = std::collections::BTreeSet::new();
    let mut usage_hits = std::collections::BTreeMap::<String, usize>::new();
    for a in &agents {
        let Ok(t) = client.transcript_tail(&a.id, 200, None).await else {
            continue;
        };
        for e in &t.entries {
            let ty = ["type", "kind", "role"]
                .iter()
                .filter_map(|k| {
                    e.raw
                        .get(*k)
                        .and_then(|v| v.as_str())
                        .map(|v| format!("{k}={v}"))
                })
                .collect::<Vec<_>>()
                .join(",");
            *kinds.entry(ty.clone()).or_default() += 1;
            if e.raw.get("kind").and_then(|v| v.as_str()) == Some("notice") {
                fn shape(v: &serde_json::Value) -> serde_json::Value {
                    match v {
                        serde_json::Value::Object(o) => serde_json::Value::Object(
                            o.iter()
                                .map(|(k, v)| {
                                    let keep = matches!(
                                        k.as_str(),
                                        "kind"
                                            | "type"
                                            | "noticeKind"
                                            | "level"
                                            | "severity"
                                            | "code"
                                            | "variant"
                                            | "category"
                                            | "action"
                                            | "href"
                                            | "url"
                                    );
                                    (
                                        k.clone(),
                                        if keep && v.is_string() {
                                            v.clone()
                                        } else {
                                            shape(v)
                                        },
                                    )
                                })
                                .collect(),
                        ),
                        serde_json::Value::Array(a) => {
                            serde_json::Value::Array(a.iter().take(2).map(shape).collect())
                        }
                        serde_json::Value::String(_) => "<str>".into(),
                        x => x.clone(),
                    }
                }
                println!("notice shape: {}", shape(&e.raw));
                println!(
                    "notice text: {}",
                    e.text.chars().take(90).collect::<String>()
                );
            }
            if let Some(o) = e.raw.as_object() {
                keys.extend(o.keys().cloned());
            }
            let low = e.text.to_ascii_lowercase();
            if [
                "usage ran out",
                "usage limit",
                "credits",
                "top up",
                "out of usage",
            ]
            .iter()
            .any(|w| low.contains(w))
            {
                *usage_hits.entry(ty).or_default() += 1;
            }
        }
    }
    println!("entry kinds: {kinds:?}");
    println!("entry keys: {keys:?}");
    println!("usage/credit mentions by kind: {usage_hits:?}");
}
