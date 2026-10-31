//! Where a link goes: the built-in browser or the system opener (`links.open_with`), with
//! the SSH rule (links stay in the built-in browser; anything that would launch an app on
//! the remote machine asks first).
use crate::app::App;
use crate::config::OpenWith;
use std::time::{Duration, Instant};

/// How long a "repeat to confirm" for a remote system open stays armed.
pub const CONFIRM_WINDOW: Duration = Duration::from_secs(5);

/// Which opener a link uses.
pub fn choose(pref: OpenWith, other: bool, ssh: bool, ssh_builtin: bool) -> OpenWith {
    let base = if ssh && ssh_builtin {
        OpenWith::Builtin
    } else {
        pref
    };
    if other { base.other() } else { base }
}

/// What to show for a URL: presigned URLs (with a query) show the host only.
pub fn shown(url: &str) -> String {
    if url.contains('?') {
        grokbot_api::attachments::url_host(url)
    } else {
        url.to_string()
    }
}

fn is_web(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// A Grok Bot app deep link (`grokbot://app/...`), e.g. a settings row.
pub fn is_app_link(url: &str) -> bool {
    url.starts_with("grokbot://app/")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The built-in browser's page for a Grok Bot deep link. Grok Bot has no public web
/// settings URL, so the page explains where to open it instead of guessing one.
pub fn app_link_page(url: &str) -> String {
    let u = esc(url);
    let what = url::Url::parse(url)
        .ok()
        .and_then(|p| {
            p.query_pairs()
                .find(|(k, _)| k == "id")
                .map(|(_, v)| v.into_owned())
        })
        .map(|id| format!("the <b>{}</b> setting", esc(&id)))
        .unwrap_or_else(|| "this page".into());
    format!(
        "<html><head><title>Open in Grok Bot</title></head><body>\
         <h1>Open in the Grok Bot app</h1>\
         <p>This link opens {what} in the Grok Bot app. Grok Bot has no web page for it, \
         and this is an SSH session, so it can't be opened from here.</p>\
         <p>Link (copied to your clipboard over OSC 52 if your terminal allows it):</p>\
         <pre>{u}</pre>\
         <p>Paste it into a browser or run <code>open</code>/<code>xdg-open</code> on the \
         computer where Grok Bot is installed, or open Grok Bot → Settings there.</p>\
         </body></html>"
    )
}

impl App {
    /// Open a link from chat, `/open` or a key: `other` flips `links.open_with`
    /// (modifier-click, vim `x`).
    pub fn open_link(&mut self, url: &str, other: bool) {
        let how = choose(
            self.config.open_with,
            other,
            self.ssh,
            self.config.ssh_builtin,
        );
        if how == OpenWith::Builtin && self.ssh && is_app_link(url) {
            // SSH: launching the app on the remote machine is useless; show the link in
            // the built-in browser and copy it so it can be opened locally.
            self.open_browser(Some(url));
            self.copy_text(url.to_string());
        } else if how == OpenWith::Builtin && is_web(url) && !url.contains('?') {
            self.open_browser(Some(url));
        } else {
            self.open_system(url);
        }
    }

    /// Open a file/attachment URL (1.33.0): follows `links.open_with` like any link, but
    /// non-text files (`textual` false) go to the system opener even with the built-in
    /// browser selected, since it can only show text.
    pub fn open_resource(&mut self, url: &str, textual: bool) {
        let how = choose(
            self.config.open_with,
            false,
            self.ssh,
            self.config.ssh_builtin,
        );
        if how == OpenWith::Builtin && textual && is_web(url) {
            self.open_browser(Some(url));
        } else {
            self.open_system(url);
        }
    }

    /// Launch the system opener. In SSH it asks first: repeat within 5 s to confirm.
    pub fn open_system(&mut self, url: &str) {
        let label = shown(url);
        if self.ssh {
            let armed = self
                .ssh_confirm
                .as_ref()
                .is_some_and(|(u, t)| u == url && t.elapsed() < CONFIRM_WINDOW);
            if !armed {
                self.ssh_confirm = Some((url.to_string(), Instant::now()));
                self.raise_notice(
                    crate::notify::NoticeKind::Warning,
                    "SSH session",
                    format!(
                        "This would open {label} on the remote machine, not your computer. \
                         Do the same again within 5 s to confirm."
                    ),
                );
                return;
            }
            self.ssh_confirm = None;
        }
        match crate::markdown::open_external(url) {
            Ok(()) => self.info(format!("Opened {label}")),
            Err(e) => self.error(format!("Couldn't open {label}: {e}")),
        }
    }

    /// Set `links.open_with` (menu, `/links`) and save it.
    pub fn set_open_with(&mut self, w: OpenWith) {
        if let Err(e) = self
            .store
            .set_pref(crate::config::PREF_LINKS_OPEN_WITH, w.as_str())
        {
            self.error(format!("{e:#}"));
            return;
        }
        self.config.open_with = w;
        let ssh = if self.ssh && self.config.ssh_builtin {
            " (this is an SSH session: links still use the built-in browser; links.ssh_behavior)"
        } else {
            ""
        };
        self.info(format!(
            "Links open with: {}{ssh}",
            match w {
                OpenWith::Builtin => "the built-in browser",
                OpenWith::System => "the system browser",
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_follows_pref_modifier_and_ssh() {
        use OpenWith::*;
        assert_eq!(choose(Builtin, false, false, true), Builtin);
        assert_eq!(choose(Builtin, true, false, true), System);
        assert_eq!(choose(System, false, false, true), System);
        assert_eq!(choose(System, true, false, true), Builtin);
        // SSH: builtin regardless of the pref; the modifier still flips it.
        assert_eq!(choose(System, false, true, true), Builtin);
        assert_eq!(choose(System, true, true, true), System);
        // respect_pref
        assert_eq!(choose(System, false, true, false), System);
    }

    #[test]
    fn presigned_urls_show_host_only() {
        assert_eq!(shown("https://a.b/c"), "https://a.b/c");
        assert!(!shown("https://s3.x/k?X-Amz-Signature=abc").contains("Signature"));
    }

    #[test]
    fn ssh_system_open_needs_confirmation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        app.ssh = true;
        app.open_system("https://example.invalid/");
        assert!(app.ssh_confirm.is_some(), "first try only arms");
        // The second would launch the opener; just check the arming logic.
        app.ssh_confirm = Some(("https://other/".into(), Instant::now()));
        app.open_system("https://example.invalid/");
        assert_eq!(
            app.ssh_confirm.as_ref().unwrap().0,
            "https://example.invalid/"
        );
        // In SSH, a plain link opens the built-in browser even with pref=system.
        app.config.open_with = OpenWith::System;
        app.open_link("https://example.invalid/x", false);
        assert!(app.browser.is_some());
    }

    #[test]
    fn ssh_settings_deep_link_opens_builtin_page() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        app.ssh = true;
        app.config.open_with = OpenWith::System;
        let link = crate::settings::app_link("usage");
        app.open_link(&link, false);
        assert!(app.ssh_confirm.is_none(), "no remote launch prompt");
        let b = app.browser.as_ref().expect("built-in browser opened");
        let Some(crate::browser::Fetched::Text { body, url, .. }) = &b.current else {
            panic!("local page shown");
        };
        assert_eq!(url, &link);
        assert!(body.contains("grokbot://app/v1/settings?id=usage"));
        assert!(body.contains("<b>usage</b>"));
        assert_eq!(app.pending_clipboard.as_deref(), Some(link.as_str()));
    }

    #[test]
    fn local_deep_link_still_uses_system_opener() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        app.ssh = false;
        assert!(is_app_link("grokbot://app/v1/settings?id=x"));
        assert!(!is_app_link("https://x/"));
        // respect_pref over SSH with pref=system: arms the remote confirm instead.
        app.ssh = true;
        app.config.ssh_builtin = false;
        app.config.open_with = OpenWith::System;
        app.open_link("grokbot://app/v1/settings?id=x", false);
        assert!(app.browser.is_none());
        assert!(app.ssh_confirm.is_some());
    }

    #[test]
    fn app_link_page_escapes() {
        let p = app_link_page("grokbot://app/v1/settings?id=<x>");
        assert!(!p.contains("<x>"));
    }
}
