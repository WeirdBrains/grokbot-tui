//! ONE live presence round trip (ignored; run by hand only):
//! `GROKBOT_TUI_LIVE_PRESENCE=1 cargo test live_presence_round_trip -- --ignored --nocapture`.
//! Sends `active: true` then `active: false` for this account's install id. Prints no secrets.

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: one ReportGrokBotDesktopActivity claim and release"]
async fn live_presence_round_trip() {
    assert_eq!(
        std::env::var("GROKBOT_TUI_LIVE_PRESENCE").as_deref(),
        Ok("1")
    );
    let (store, client) = crate::live_attach_test::live_client().await;
    let id = store
        .get_pref(crate::config::PREF_INSTALL_ID)
        .unwrap()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let on = client.report_desktop_activity(&id, true).await;
    println!(
        "active:true → {:?}",
        on.as_ref()
            .map(|d| d.as_millis())
            .map_err(|e| e.to_string())
    );
    let off = client.report_desktop_activity(&id, false).await;
    println!(
        "active:false → {:?}",
        off.as_ref()
            .map(|d| d.as_millis())
            .map_err(|e| e.to_string())
    );
    on.unwrap();
    off.unwrap();
}
