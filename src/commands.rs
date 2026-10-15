//! Extensible slash-command registry. Add a [`Command`] to [`registry`].
use crate::app::{App, EntryKind, InputMode};
use anyhow::{Result, bail};

pub type Handler = fn(&mut App, &str) -> Result<()>;

pub struct Command {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub help: &'static str,
    pub run: Handler,
}

pub struct Registry {
    pub commands: Vec<Command>,
}

impl Registry {
    pub fn find(&self, name: &str) -> Option<&Command> {
        let n = name.to_lowercase();
        self.commands
            .iter()
            .find(|c| c.name == n || c.aliases.contains(&n.as_str()))
    }
}

pub fn registry() -> Registry {
    Registry {
        commands: vec![
            Command {
                name: "help",
                aliases: &["h", "?"],
                usage: "/help [command]",
                help: "Show commands",
                run: help,
            },
            Command {
                name: "login",
                aliases: &[],
                usage: "/login [token]",
                help: "Log in with an API token (prompted, masked, if omitted)",
                run: login,
            },
            Command {
                name: "logout",
                aliases: &[],
                usage: "/logout",
                help: "Delete stored credential and clear state",
                run: logout,
            },
            Command {
                name: "whoami",
                aliases: &[],
                usage: "/whoami",
                help: "Show the logged-in user",
                run: whoami,
            },
            Command {
                name: "bots",
                aliases: &[],
                usage: "/bots",
                help: "Refresh and list bots",
                run: bots,
            },
            Command {
                name: "bot",
                aliases: &["use"],
                usage: "/bot <name|id>",
                help: "Switch to a bot's chat",
                run: bot,
            },
            Command {
                name: "history",
                aliases: &[],
                usage: "/history",
                help: "Reload the active bot's chat history",
                run: history,
            },
            Command {
                name: "clear",
                aliases: &[],
                usage: "/clear",
                help: "Clear the body (local only)",
                run: clear,
            },
            Command {
                name: "quit",
                aliases: &["q", "exit"],
                usage: "/quit",
                help: "Exit gb",
                run: quit,
            },
        ],
    }
}

fn help(app: &mut App, args: &str) -> Result<()> {
    let text = if args.is_empty() {
        let mut s = String::from("Commands:\n");
        for c in &app.registry.commands {
            s.push_str(&format!("  {:<18} {}\n", c.usage, c.help));
        }
        s.push_str("Plain text is sent to the active bot.\nKeys: Tab focus sidebar, ↑/↓ + Enter pick bot, PgUp/PgDn or wheel scroll, click a bot, Ctrl-C quit.");
        s
    } else {
        match app.registry.find(args.trim_start_matches('/')) {
            Some(c) => format!("{} — {}", c.usage, c.help),
            None => bail!("No such command: {args}"),
        }
    };
    app.info(text);
    Ok(())
}

fn login(app: &mut App, args: &str) -> Result<()> {
    if args.is_empty() {
        app.mode = InputMode::Token;
        app.info("Paste your API token and press Enter (input is masked; empty cancels).".into());
    } else {
        finish_login(app, args);
    }
    Ok(())
}

/// Validate via whoami; persist only on success.
// Future: an OAuth device-code flow would obtain `token` here instead of a paste.
pub fn finish_login(app: &mut App, token: &str) {
    if token.is_empty() {
        app.info("Login cancelled.".into());
        return;
    }
    match app.authenticate(token.to_string()) {
        Ok(()) => match app.store.save_token(token) {
            Ok(()) => app.info(format!(
                "Token saved to {}",
                app.store.credentials_path().display()
            )),
            Err(e) => app.error(format!("Logged in, but could not save token: {e:#}")),
        },
        Err(e) => app.error(format!("Login failed: {e:#}")),
    }
}

fn logout(app: &mut App, _: &str) -> Result<()> {
    app.logout();
    Ok(())
}

fn whoami(app: &mut App, _: &str) -> Result<()> {
    let u = app.block(app.api.whoami())?;
    app.info(format!("{} (id {})", u.name, u.id));
    Ok(())
}

fn bots(app: &mut App, _: &str) -> Result<()> {
    let b = app.block(app.api.list_bots())?;
    app.bots = b;
    let list = app
        .bots
        .iter()
        .map(|b| format!("  {:<12} {:<12} {}", b.id, b.name, b.description))
        .collect::<Vec<_>>()
        .join("\n");
    app.info(format!("Bots:\n{list}"));
    if app.active.is_none_or(|i| i >= app.bots.len()) && !app.bots.is_empty() {
        app.select_bot(0);
    }
    Ok(())
}

fn bot(app: &mut App, args: &str) -> Result<()> {
    if args.is_empty() {
        bail!("Usage: /bot <name|id>");
    }
    match app.find_bot(args) {
        Some(i) => {
            app.select_bot(i);
            Ok(())
        }
        None => bail!("No bot named {args}. Try /bots."),
    }
}

fn history(app: &mut App, _: &str) -> Result<()> {
    let Some(id) = app.active_bot().map(|b| b.id.clone()) else {
        bail!("No active bot")
    };
    app.load_history(&id);
    app.push(EntryKind::System, "History reloaded.".into());
    Ok(())
}

fn clear(app: &mut App, _: &str) -> Result<()> {
    app.clear_current();
    Ok(())
}

fn quit(app: &mut App, _: &str) -> Result<()> {
    app.should_quit = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_lookup_and_unique_names() {
        let r = registry();
        assert_eq!(r.find("q").unwrap().name, "quit");
        assert_eq!(r.find("HELP").unwrap().name, "help");
        let mut names: Vec<_> = r
            .commands
            .iter()
            .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
            .collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(n, names.len());
    }
}
