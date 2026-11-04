//! Website screenshots (ignored test; `scripts/screenshots.sh` runs it).
//!
//! Drives the TUI with the offline mock API in a temp data dir (never ~/.grokbot-tui,
//! never a real account) and writes each screen as HTML (one coloured cell per terminal
//! cell) to `target/screenshots/`, with `list.tsv` (name, caption). Headless Chromium
//! turns those into PNGs.
use super::tests::{mock_login, repo_themes, settle, test_app};
use super::*;
use crate::api::Attachment;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::fmt::Write as _;

const W: u16 = 150;
const H: u16 = 42;

fn hex(c: Color, fallback: &str) -> String {
    let named = |i: u8| -> String {
        const P: [&str; 16] = [
            "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
            "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
        ];
        P[i as usize].to_string()
    };
    match c {
        Color::Reset => fallback.to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Indexed(i) if i < 16 => named(i),
        Color::Indexed(i) if i >= 232 => {
            let v = 8 + (i - 232) * 10;
            format!("#{v:02x}{v:02x}{v:02x}")
        }
        Color::Indexed(i) => {
            let i = i - 16;
            let s = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            format!("#{:02x}{:02x}{:02x}", s(i / 36), s((i / 6) % 6), s(i % 6))
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The buffer as a standalone HTML page (cells on a fixed grid).
fn to_html(buf: &Buffer) -> String {
    let base: ratatui::style::Style = crate::theme::base().into();
    let bg0 = hex(base.bg.unwrap_or(Color::Reset), "#1a1b26");
    let fg0 = hex(base.fg.unwrap_or(Color::Reset), "#c0caf5");
    let mut out = format!(
        "<!doctype html><meta charset=utf-8><style>
html,body{{margin:0;background:{bg0}}}
.t{{font:16px/20px 'DejaVu Sans Mono','Noto Sans Mono',monospace;padding:12px;width:{}px}}
.r{{display:flex;height:20px}}
.r span{{display:inline-block;width:10px;height:20px;overflow:visible;white-space:pre;text-align:left}}
.r span.w{{width:20px}}
</style><div class=t>",
        W as u32 * 10
    );
    let area = buf.area;
    for y in 0..area.height {
        out.push_str("<div class=r>");
        let mut skip = 0;
        for x in 0..area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let c = &buf[(x, y)];
            let sym = c.symbol();
            let wide = unicode_width::UnicodeWidthStr::width(sym) > 1;
            if wide {
                skip = 1;
            }
            let mut fg = hex(c.fg, &fg0);
            let mut bg = hex(c.bg, &bg0);
            if c.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let mut st = format!("color:{fg};background:{bg}");
            if c.modifier.contains(Modifier::BOLD) {
                st.push_str(";font-weight:bold");
            }
            if c.modifier.contains(Modifier::ITALIC) {
                st.push_str(";font-style:italic");
            }
            if c.modifier.contains(Modifier::UNDERLINED) {
                st.push_str(";text-decoration:underline");
            }
            if c.modifier.contains(Modifier::DIM) {
                st.push_str(";opacity:.7");
            }
            let _ = write!(
                out,
                "<span{} style=\"{st}\">{}</span>",
                if wide { " class=w" } else { "" },
                esc(if sym.is_empty() { " " } else { sym })
            );
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    out
}

fn msg(role: Role, text: &str, attachments: Vec<Attachment>) -> Message {
    Message {
        role,
        text: text.into(),
        remote_id: None,
        attachments,
    }
}

fn att(name: &str, kind: &str, size: u64) -> Attachment {
    Attachment {
        path: format!("/workspace/{name}"),
        name: name.into(),
        size: Some(size),
        content_type: Some(kind.into()),
    }
}

/// Sample conversation (invented; no real account data).
fn seed(app: &mut App) -> String {
    let bot = app.active_bot().unwrap().id.clone();
    let rows = [
        msg(
            Role::User,
            "Can you summarize what changed in the deploy notes I sent?",
            vec![att("deploy-notes.md", "text/markdown", 4200)],
        ),
        msg(
            Role::Bot,
            "Here's the short version:\n\n- **Rollout** goes to the staging cluster first, then 10% of production.\n- The `healthcheck` timeout moved from 5s to 15s.\n- Rollback is one command:\n\n```sh\nkubectl rollout undo deploy/api\n```\n\nWant me to draft the announcement?",
            vec![],
        ),
        msg(
            Role::User,
            "Yes, and attach the dashboard screenshot.",
            vec![att("dashboard.png", "image/png", 182_000)],
        ),
        msg(
            Role::Bot,
            "Done. The draft is in `announcement.md` and the latency chart is attached.",
            vec![att("announcement.md", "text/markdown", 1800)],
        ),
    ];
    for m in &rows {
        app.store.append_message(&bot, m).unwrap();
    }
    app.reload_bot_log(&bot);
    let _ = app.react("👍", 2);
    let _ = app.react("🎉", 1);
    bot
}

struct Shots {
    dir: std::path::PathBuf,
    list: String,
}

impl Shots {
    fn shot(&mut self, app: &mut App, name: &str, caption: &str) {
        let mut term = ratatui::Terminal::new(TestBackend::new(W, H)).unwrap();
        // Twice: the first frame lays out hit areas some views read back.
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        let html = to_html(term.backend().buffer());
        std::fs::write(self.dir.join(format!("{name}.html")), html).unwrap();
        let _ = writeln!(self.list, "{name}\t{caption}");
    }
}

#[test]
#[ignore = "writes website screenshots; run by scripts/screenshots.sh"]
fn site_screenshots() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mut app, _d) = test_app(&rt);
    let dir = app.themes_dir();
    let mut themes: Vec<(String, String)> = vec![];
    for e in std::fs::read_dir(repo_themes()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("toml") {
            continue;
        }
        std::fs::copy(&p, dir.join(p.file_name().unwrap())).unwrap();
        let id = p.file_stem().unwrap().to_string_lossy().to_string();
        let name = std::fs::read_to_string(&p)
            .unwrap()
            .lines()
            .find_map(|l| {
                l.strip_prefix("name = \"")
                    .map(|n| n.trim_end_matches('"').to_string())
            })
            .unwrap_or_else(|| id.clone());
        themes.push((id, name));
    }
    themes.sort();
    app.init_themes();
    mock_login(&mut app, "tok");
    app.ssh = false;
    let bot = seed(&mut app);
    app.set_attachments_panel(true);
    app.attpanel.focused = false;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screenshots");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let mut s = Shots {
        dir: out.clone(),
        list: String::new(),
    };
    let reset = |app: &mut App| {
        app.reload_bot_log(&bot);
        app.notices.clear();
        app.notices_board.settings.dnd = true;
    };

    app.set_theme("default");
    app.set_theme_mode(crate::theme::Mode::Dark);
    reset(&mut app);
    s.shot(
        &mut app,
        "chat",
        "Chat: bot list on the left, message bubbles with reactions, and the attachments bar pulled out on the right",
    );
    app.set_attachments_panel(false);
    reset(&mut app);
    s.shot(
        &mut app,
        "chat-compact",
        "The same chat with the right-hand bar tucked away",
    );
    app.set_attachments_panel(true);
    app.attpanel.focused = false;
    reset(&mut app);

    app.open_msg_search("rollout");
    settle(&mut app);
    s.shot(
        &mut app,
        "search",
        "Ctrl-F searches every bot's messages and attachments locally",
    );
    app.msg_search = None;

    app.open_help("overview");
    s.shot(
        &mut app,
        "help",
        "Built-in help: the same guides as the docs",
    );
    app.help = None;
    reset(&mut app);

    app.open_theme_picker();
    s.shot(
        &mut app,
        "theme-picker",
        "Pick a theme with a live preview (F5)",
    );
    app.picker = None;
    app.theme_preview = None;
    reset(&mut app);

    for (id, name) in &themes {
        for (mode, label) in [
            (crate::theme::Mode::Dark, "dark"),
            (crate::theme::Mode::Light, "light"),
        ] {
            app.set_theme(id);
            app.set_theme_mode(mode);
            reset(&mut app);
            s.shot(
                &mut app,
                &format!("theme-{id}-{label}"),
                &format!("{name} ({label})"),
            );
        }
    }
    std::fs::write(out.join("list.tsv"), &s.list).unwrap();
}
