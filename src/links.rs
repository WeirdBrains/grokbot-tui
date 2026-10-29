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
        if how == OpenWith::Builtin && is_web(url) && !url.contains('?') {
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
}
