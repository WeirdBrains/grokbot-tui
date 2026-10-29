//! ONE live attachment test against the real service (ignored; run by hand only):
//! `GROKBOT_TUI_LIVE_ATTACH=1 cargo test live_attachment_round_trip -- --ignored --nocapture`.
//! Uses the signed-in account in ~/.grokbot-tui the way the app does (stored login,
//! re-reading the desktop session for `/login desktop` accounts). Prints no secrets.
use crate::app::{apply_client_identity, real_desktop_reader};
use crate::storage::{LoginMethod, Store};
use grokbot_api as gw;
use serde_json::Value;

const AGENT: &str = "0818df1c-08cc-47eb-99d6-ec90eeb990ff";
const PROMPT: &str = "grokbot-tui attachment test: what colour is this image?";

fn shape(s: &str) -> String {
    // Alias format without the value: character classes and separators only.
    let mut out = String::new();
    let mut last = ' ';
    for c in s.chars() {
        let k = if c.is_ascii_digit() {
            '9'
        } else if c.is_ascii_lowercase() {
            'a'
        } else if c.is_ascii_uppercase() {
            'A'
        } else {
            c
        };
        if k != last || !matches!(k, '9' | 'a' | 'A') {
            out.push(k);
        }
        last = k;
    }
    format!("{out} (len {})", s.len())
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// The signed-in account's client, built the way the app does. Prints no secrets.
pub(crate) async fn live_client() -> (Store, gw::Client) {
    let (dir, _) = Store::default_dir().unwrap();
    let store = Store::open(dir, None).unwrap();
    let config = store.load_config().unwrap();
    let login = store.load_login().unwrap().expect("no stored login");
    println!(
        "account: {} · method: {:?}",
        store.account().alias,
        login.method
    );
    let mut opts = gw::ClientOptions::default();
    apply_client_identity(&mut opts, &config, true);
    let client = match (login.method, login.cursor_token) {
        (_, Some(tok)) => gw::Client::with_cursor_token(login.session, tok, opts).unwrap(),
        (LoginMethod::Desktop, None) => {
            let (cred, _) = real_desktop_reader()(login.desktop_entry.as_deref(), &opts.policy)
                .expect("desktop session read failed");
            match cred {
                gw::desktop::DesktopCredential::CursorToken(t) => {
                    gw::Client::connect_cursor(&t, opts).await.unwrap()
                }
                gw::desktop::DesktopCredential::Session(_) => {
                    panic!("STOP: desktop gave a gateway-only session (no Cursor token)")
                }
            }
        }
        _ => panic!("STOP: login has no Cursor token; attachments need /login desktop or cursor"),
    };
    (store, client)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: sends one real prompt with an attachment"]
async fn live_attachment_round_trip() {
    assert_eq!(std::env::var("GROKBOT_TUI_LIVE_ATTACH").as_deref(), Ok("1"));
    let (_store, client) = live_client().await;
    assert!(client.has_cursor_session());
    let team = client.list_team_agents().await.unwrap().unwrap_or_default();
    println!(
        "Grokbot-cli in team list: {}",
        team.iter().any(|a| a.agent_id == AGENT)
    );

    // 64x64 solid teal PNG.
    let img = image::RgbImage::from_pixel(64, 64, image::Rgb([0x00, 0x80, 0x80]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();
    println!("png bytes: {}", png.len());

    let begin = client
        .begin_attachment_upload(
            AGENT,
            None,
            "teal-64.png",
            png.len() as u64,
            Some(&gw::attachments::sha256_hex(&png)),
        )
        .await;
    let begin = match begin {
        Ok(b) => b,
        Err(e) => panic!("Begin failed: {e}"),
    };
    println!("Begin: {begin:?}");
    match begin.mode {
        gw::UploadMode::SinglePut => {}
        m => {
            if !begin.upload_token.is_empty() {
                let _ = client.abort_attachment_upload(&begin.upload_token).await;
            }
            panic!("STOP: mode {m} (use_box_reason {:?})", begin.use_box_reason);
        }
    }
    let put = begin.put.clone().expect("no put target");
    println!(
        "upload host: {} · header names: {:?} · expires_at_ms set: {}",
        gw::attachments::url_host(&put.url),
        put.headers.keys().collect::<Vec<_>>(),
        put.expires_at_ms.is_some()
    );
    // One PUT, exactly as returned, no auth header.
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut rb = http.put(&put.url).body(png.clone());
    for (k, v) in &put.headers {
        rb = rb.header(k, v);
    }
    let status = rb
        .send()
        .await
        .map(|r| r.status().as_u16())
        .map_err(|e| e.without_url().to_string());
    println!("PUT status: {status:?}");
    if !matches!(status, Ok(200..=299)) {
        let _ = client.abort_attachment_upload(&begin.upload_token).await;
        panic!("STOP: PUT failed");
    }
    let done = client
        .complete_attachment_upload(&begin.upload_token)
        .await
        .unwrap();
    println!(
        "Complete: refusal {} · alias shape {} · file_name {:?} · size {:?} · content_type {:?}",
        done.refusal,
        shape(&done.alias),
        done.file_name,
        done.size_bytes,
        done.content_type
    );
    assert!(!done.alias.is_empty(), "no alias");

    let mut req = gw::SendPrompt::new(AGENT, PROMPT);
    req.attachment_paths = vec![done.alias.clone()];
    req.attachment_names = vec!["teal-64.png".into()];
    match client.send_prompt(&req).await {
        Ok(r) => println!(
            "sendPrompt: delivery {} · message id set {} · response keys {:?}",
            r.delivery,
            r.message_id.is_some(),
            keys(&r.raw)
        ),
        Err(e) => println!("sendPrompt error: {e}"),
    }

    let reads = client
        .resolve_attachment_reads(AGENT, None, std::slice::from_ref(&done.alias))
        .await;
    match &reads {
        Ok(r) => {
            for x in r {
                println!(
                    "read: outcome {} · host {} · content_type {:?} · size {:?} · etag set {}",
                    x.outcome,
                    gw::attachments::url_host(&x.url),
                    x.content_type,
                    x.size_bytes,
                    !x.etag.is_empty()
                );
            }
        }
        Err(e) => println!("ResolveGrokBotAttachmentReads error: {e}"),
    }
    // GET the resolved URL once to confirm the bytes round-trip.
    if let Ok(r) = &reads
        && let Some(x) = r.first().filter(|x| x.url.starts_with("https://"))
    {
        let got = http.get(&x.url).send().await;
        match got {
            Ok(resp) => {
                let st = resp.status().as_u16();
                let b = resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
                println!("GET status {st} · bytes match {}", b == png);
            }
            Err(e) => println!("GET error: {}", e.without_url()),
        }
    }

    // How the transcript records it (keys only).
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    if let Ok(t) = client.transcript_tail(AGENT, 10, None).await {
        if let Some(e) = t
            .entries
            .iter()
            .rev()
            .find(|e| e.text.contains("attachment test"))
        {
            println!("transcript entry keys: {:?}", keys(&e.raw));
            for k in keys(&e.raw) {
                let v = &e.raw[&k];
                if k.to_lowercase().contains("attach") {
                    let desc = match v {
                        Value::Array(a) => format!(
                            "array[{}] of {}",
                            a.len(),
                            a.first()
                                .map(|x| if x.is_object() {
                                    format!("object keys {:?}", keys(x))
                                } else {
                                    "strings".into()
                                })
                                .unwrap_or_default()
                        ),
                        other => (if other.is_object() {
                            "object"
                        } else {
                            "scalar"
                        })
                        .to_string(),
                    };
                    println!("  {k}: {desc}");
                }
            }
            let parsed = crate::api::attachments_from_raw(&e.raw);
            println!(
                "attachments_from_raw: {} item(s), names {:?}, path==alias {}",
                parsed.len(),
                parsed.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                parsed.iter().any(|a| a.path == done.alias)
            );
        } else {
            println!("transcript: test entry not found in the last 10");
        }
    }
}
