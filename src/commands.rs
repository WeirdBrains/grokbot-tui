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
                name: "prefs",
                aliases: &["pref"],
                usage: "/prefs [get <k>|set <k> <v>]",
                help: "List, get, or set preferences",
                run: prefs,
            },
            Command {
                name: "memory",
                aliases: &["mem"],
                usage: "/memory [add <k> <v>|clear [k]]",
                help: "Per-bot memory notes for the active bot",
                run: memory,
            },
            Command {
                name: "clear",
                aliases: &[],
                usage: "/clear [saved]",
                help: "Clear the body; `saved` also deletes the bot's stored transcript",
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
        Ok(()) => {
            match app
                .store
                .save_credentials(token, app.user.as_ref(), &app.config.api_base_url)
            {
                Ok(()) => app.info(format!("Token saved to {}", app.store.db_path().display())),
                Err(e) => app.error(format!("Logged in, but could not save token: {e:#}")),
            }
        }
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

fn prefs(app: &mut App, args: &str) -> Result<()> {
    let mut it = args.splitn(3, char::is_whitespace);
    match (it.next().unwrap_or(""), it.next(), it.next()) {
        ("" | "list", None, None) => {
            let list = app
                .store
                .list_prefs()?
                .into_iter()
                .map(|(k, v)| format!("  {k:<22} {v}"))
                .collect::<Vec<_>>()
                .join("\n");
            app.info(format!("Preferences:\n{list}"));
        }
        ("get", Some(k), None) => {
            let v = app.store.get_pref(k)?;
            app.info(format!("{k} = {}", v.as_deref().unwrap_or("(unset)")));
        }
        ("set", Some(k), v) => {
            let v = v.unwrap_or("").trim();
            app.store.set_pref(k, v)?;
            app.config = app.store.load_config()?;
            let note = if k.starts_with("api_")
                || k.starts_with("paths.")
                || k == "request_timeout_secs"
            {
                " (restart gb for API changes to take effect)"
            } else {
                ""
            };
            app.info(format!("{k} = {v}{note}"));
        }
        _ => bail!("Usage: /prefs [list | get <key> | set <key> <value>]"),
    }
    Ok(())
}

fn memory(app: &mut App, args: &str) -> Result<()> {
    let Some(bot) = app.active_bot().map(|b| b.id.clone()) else {
        bail!("No active bot")
    };
    let mut it = args.splitn(3, char::is_whitespace);
    match (it.next().unwrap_or(""), it.next(), it.next()) {
        ("" | "list", None, None) => {
            let items = app.store.list_memory(&bot)?;
            let text = if items.is_empty() {
                format!("No memory notes for {bot}.")
            } else {
                let l = items
                    .iter()
                    .map(|(k, v)| format!("  {k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Memory for {bot}:\n{l}")
            };
            app.info(text);
        }
        ("add" | "set", Some(k), Some(v)) => {
            app.store.set_memory(&bot, k, v.trim())?;
            app.info(format!("Remembered {k} for {bot}."));
        }
        ("clear", k, None) => {
            let n = app.store.clear_memory(&bot, k)?;
            app.info(format!("Cleared {n} memory note(s) for {bot}."));
        }
        _ => bail!("Usage: /memory [list | add <key> <value> | clear [key]]"),
    }
    Ok(())
}

fn clear(app: &mut App, args: &str) -> Result<()> {
    match args {
        "" => {}
        "saved" => {
            let Some(id) = app.active_bot().map(|b| b.id.clone()) else {
                bail!("No active bot")
            };
            app.store.clear_messages(&id)?;
        }
        _ => bail!("Usage: /clear [saved]"),
    }
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
