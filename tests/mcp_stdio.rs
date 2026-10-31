//! Smoke tests for `grokbot-tui --mock mcp serve`: stdio (initialize, then tools/list)
//! and HTTP (bearer token required).
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_grokbot-tui"))
}

#[test]
fn stdio_initialize_then_tools_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = bin()
        .args(["--mock", "--data-dir"])
        .arg(dir.path())
        .args(["mcp", "serve", "--stdio"])
        .env("GROKBOT_TUI_NO_THEME_DOWNLOAD", "1")
        .env("XDG_CACHE_HOME", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: serde_json::Value| {
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    };
    let mut read = || -> serde_json::Value {
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    };
    send(
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "smoke", "version": "0"}}}),
    );
    let init = read();
    assert_eq!(init["id"], 1, "{init}");
    assert_eq!(
        init["result"]["serverInfo"]["name"], "grokbot-tui",
        "{init}"
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    send(serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let list = read();
    assert_eq!(list["id"], 2, "{list}");
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in [
        "list_bots",
        "get_bot",
        "send_message",
        "read_messages",
        "wait_for_reply",
        "list_pending_cards",
        "answer_card",
        "list_attachments",
        "download_attachment",
    ] {
        assert!(names.contains(&want), "{want} missing: {names:?}");
    }
    send(
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "list_bots", "arguments": {}}}),
    );
    let call = read();
    assert!(
        call["result"]["structuredContent"]["bots"]
            .as_array()
            .unwrap()
            .len()
            >= 3,
        "{call}"
    );
    drop(stdin);
    let _ = child.wait();
}

fn post(port: u16, auth: Option<&str>) -> String {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}"#;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let auth = auth
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    write!(s, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut r = String::new();
    let mut buf = [0u8; 4096];
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                r.push_str(&String::from_utf8_lossy(&buf[..n]));
                if r.contains("serverInfo") || r.starts_with("HTTP/1.1 401") && r.contains("token")
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    r
}

#[test]
fn http_requires_the_bearer_token() {
    let dir = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let token = String::from_utf8(
        bin()
            .arg("--data-dir")
            .arg(dir.path())
            .args(["mcp", "token"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert_eq!(token.len(), 64);
    let mut child = bin()
        .args(["--mock", "--data-dir"])
        .arg(dir.path())
        .args(["mcp", "serve", "--http", &format!("127.0.0.1:{port}")])
        .env("GROKBOT_TUI_NO_THEME_DOWNLOAD", "1")
        .env("XDG_CACHE_HOME", dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait for the listener.
    let start = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "server did not start"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let denied = post(port, None);
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
    let wrong = post(port, Some("nope"));
    assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");
    let ok = post(port, Some(&token));
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(ok.contains("grokbot-tui"), "{ok}");
    let _ = child.kill();
    let mut err = String::new();
    let _ = child.wait();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    assert!(!err.contains(&token), "token leaked to stderr");
}

#[test]
fn http_refuses_non_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let o = bin()
        .args(["--mock", "--data-dir"])
        .arg(dir.path())
        .args(["mcp", "serve", "--http", "0.0.0.0:8765"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("loopback"));
}
