//! System clipboard when running locally (1.30.0): OSC 52 goes to the terminal as before;
//! this also hands the text to wl-copy, xclip, xsel or pbcopy when one is installed.
//! Runs in a background thread so a slow tool never blocks the UI. Skipped over SSH.

pub fn system_copy(text: &str) {
    if cfg!(test) {
        return;
    }
    let text = text.to_string();
    std::thread::spawn(move || {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let x11 = std::env::var_os("DISPLAY").is_some();
        let tools: Vec<(&str, &[&str])> = vec![
            ("pbcopy", &[]),
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        for (cmd, args) in tools {
            let ok = match cmd {
                "pbcopy" => cfg!(target_os = "macos"),
                "wl-copy" => wayland,
                _ => x11,
            };
            if !ok {
                continue;
            }
            let Ok(mut child) = Command::new(cmd)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            if let Some(mut i) = child.stdin.take() {
                let _ = i.write_all(text.as_bytes());
            }
            if child.wait().is_ok_and(|s| s.success()) {
                return;
            }
        }
    });
}
