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
