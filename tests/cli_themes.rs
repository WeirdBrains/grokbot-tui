//! `--list-themes` / `--theme` on a first run download the default theme from
//! `GROKBOT_TUI_THEME_URL` (a local mock server here; never the real network or the real
//! home folder).

use std::process::Command;

fn bin(data: &std::path::Path, base: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_grokbot-tui"));
    c.arg("--data-dir")
        .arg(data)
        .env("HOME", data)
        .env("GROKBOT_TUI_THEME_URL", base)
        .env_remove("GROKBOT_TUI_NO_THEME_DOWNLOAD")
        .env_remove("GROKBOT_TUI_ACCOUNT");
    c
}

fn shipped() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/themes/default.toml")).unwrap()
}

fn server(rt: &tokio::runtime::Runtime) -> wiremock::MockServer {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    rt.block_on(async {
        let srv = MockServer::start().await;
        Mock::given(path("/themes/default.toml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(shipped()))
            .mount(&srv)
            .await;
        srv
    })
}

#[test]
fn list_themes_downloads_the_default_on_a_first_run() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let srv = server(&rt);
    let d = tempfile::tempdir().unwrap();
    let out = bin(d.path(), &format!("{}/themes/", srv.uri()))
        .arg("--list-themes")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(stderr.contains("downloaded the default theme"), "{stderr}");
    assert!(stdout.contains("● default"), "{stdout}");
    let file = d.path().join("themes/default.toml");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), shipped());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // Second run: nothing to download.
    let out = bin(d.path(), "http://127.0.0.1:9/")
        .arg("--list-themes")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("download"));
}

#[test]
fn theme_flag_downloads_first_then_checks() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let srv = server(&rt);
    let d = tempfile::tempdir().unwrap();
    // The download happens, then the missing `solar` stops before the UI with the list.
    let out = bin(d.path(), &format!("{}/themes", srv.uri()))
        .args(["--theme", "solar"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("downloaded the default theme"), "{stderr}");
    assert!(stderr.contains("themes: default"), "{stderr}");
}

#[test]
fn offline_or_opted_out_lists_without_themes() {
    let d = tempfile::tempdir().unwrap();
    let out = bin(d.path(), "http://127.0.0.1:9/")
        .arg("--list-themes")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: couldn't download"));
    assert!(String::from_utf8_lossy(&out.stdout).contains("(none)"));
    let out = bin(d.path(), "http://127.0.0.1:9/")
        .env("GROKBOT_TUI_NO_THEME_DOWNLOAD", "1")
        .arg("--list-themes")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("downloads are off"));
    assert!(!d.path().join("themes/default.toml").exists());
}
