//! ONE live bot-list refresh (ignored; run by hand only):
//! `GROKBOT_TUI_LIVE_BOTS=1 cargo test live_bot_list_refresh -- --ignored --nocapture`.
//! Lists bots with the stored login (read-only) and diffs them against the cached list,
//! as `/refresh` does. Prints bot names only, no secrets.

use crate::api::ApiClient;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: one listAgents (+ team agents) call"]
async fn live_bot_list_refresh() {
    assert_eq!(std::env::var("GROKBOT_TUI_LIVE_BOTS").as_deref(), Ok("1"));
    let (store, client) = crate::live_attach_test::live_client().await;
    let cached = store.bots().unwrap_or_default();
    let api = crate::api::GatewayApi::new(client);
    let fresh = api.list_bots().await.expect("list_bots");
    if let Some(n) = api.take_list_notice() {
        println!("list notice: {n}");
    }
    let d = crate::api::diff_bots(&cached, &fresh);
    let team = fresh.iter().filter(|b| b.team).count();
    let cached_team: Vec<&str> = cached
        .iter()
        .filter(|b| b.team)
        .map(|b| b.name.as_str())
        .collect();
    println!(
        "team bots fresh: {team}; cached team: {}",
        cached_team.join(", ")
    );
    println!(
        "cached: {} bots · fresh: {} bots",
        cached.len(),
        fresh.len()
    );
    println!("{}", d.notice());
    let names: Vec<&str> = fresh.iter().map(|b| b.name.as_str()).collect();
    println!("names: {}", names.join(", "));
    if let Ok(want) = std::env::var("GROKBOT_TUI_LIVE_EXPECT") {
        assert!(
            fresh.iter().any(|b| b.name.eq_ignore_ascii_case(&want)),
            "{want} not in the refreshed list"
        );
        println!("found: {want}");
    }
}
