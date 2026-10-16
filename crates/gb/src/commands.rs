//! Extensible slash-command registry. Add a [`Command`] to [`registry`].
use crate::app::{App, EntryKind, InputMode, LoginSource};
use crate::storage::Scope;
use anyhow::{Context, Result, bail};

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
                usage: "/login gateway [url] | cursor | desktop [entry]",
                help: "Log in the active account: gateway URL + token, or a Cursor access token",
                run: login,
            },
            Command {
                name: "logout",
                aliases: &[],
                usage: "/logout",
                help: "Log out of the active account (keeps chats and memory)",
                run: logout,
            },
            Command {
                name: "whoami",
                aliases: &[],
                usage: "/whoami",
                help: "Show the active account's login details",
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
                usage: "/bot [name|id] | /bot new <name> | /bot delete [name]",
                help: "Switch to, create, or delete a bot",
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
                name: "account",
                aliases: &["acct"],
                usage: "/account [add|use|rename|remove]",
                help: "List accounts; add <alias>, use <alias>, rename <old> <new>, remove <alias>",
                run: account,
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
                name: "quit",
                aliases: &["q", "exit"],
                usage: "/quit",
                help: "Exit grokbot-tui",
                run: quit,
            },
        ],
    }
}

fn help(app: &mut App, args: &str) -> Result<()> {
    if app.open_help(args) {
        return Ok(());
    }
    let id = crate::help::normalize(args);
    let names = crate::help::all_names(&app.registry);
    match crate::help::suggest(&id, names.iter().map(String::as_str)) {
        Some(s) => bail!(
            "No help for '{args}'. Did you mean /help {}?",
            crate::help::link_label(&s).trim_start_matches("/help ")
        ),
        None => bail!("No help for '{args}'. Try /help."),
    }
}

fn login(app: &mut App, args: &str) -> Result<()> {
    let mut it = args.split_whitespace();
    match (it.next(), it.next(), it.next()) {
        (None, _, _) => {
            app.open_help("login");
        }
        (Some("gateway"), url, None) => {
            app.mode = InputMode::GatewayUrl;
            match url {
                Some(u) => set_gateway_url(app, u),
                None => app.info(
                    "Enter the gateway URL (e.g. https://<box>.cursorvm.com). Empty cancels."
                        .into(),
                ),
            }
        }
        (Some("cursor"), None, None) => {
            app.mode = InputMode::CursorToken;
            app.info(
                "Paste your Cursor access token (masked; not a dashboard API key). It is exchanged via EnsureSandBox and stored so grokbot-tui can re-exchange when the gateway token expires. Empty cancels."
                    .into(),
            );
        }
        (Some("desktop"), entry, None) => {
            app.info("Reading the Grok Bot desktop app's saved session…".into());
            app.start_desktop_login(entry.map(str::to_string), LoginSource::Interactive);
        }
        _ => bail!("Usage: /login gateway [url]  |  /login cursor  |  /login desktop [entry]"),
    }
    Ok(())
}

/// Validate a gateway URL against the host allowlist, then ask for the token.
pub fn set_gateway_url(app: &mut App, url: &str) {
    let checked = if app.mock {
        Ok(url.trim_end_matches('/').to_string())
    } else {
        app.opts.policy.check_gateway(url)
    };
    match checked {
        Ok(u) => {
            app.login_url_draft = Some(u);
            app.mode = InputMode::GatewayToken;
            app.info("Paste the gateway token (masked). Empty cancels.".into());
        }
        Err(e) => {
            app.mode = InputMode::Normal;
            app.login_url_draft = None;
            app.error(e);
        }
    }
}

fn logout(app: &mut App, _: &str) -> Result<()> {
    app.logout();
    Ok(())
}

fn whoami(app: &mut App, _: &str) -> Result<()> {
    let alias = app.account().alias.clone();
    let mut lines = vec![format!("account: {alias}")];
    match &app.user_label {
        Some(l) => lines.push(format!("session: {l}")),
        None => lines.push(if app.connecting {
            "session: connecting…".into()
        } else {
            "session: not logged in".into()
        }),
    }
    if let Some(l) = app.store.load_login()? {
        lines.push(format!("stored login: {:?}", l.method).to_lowercase());
        lines.push(format!("gateway: {}", l.session.gateway_url));
        if !l.session.headers.is_empty() {
            lines.push(format!(
                "extra headers: {}",
                l.session
                    .headers
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines.push(format!(
            "cursor token stored: {}",
            if l.cursor_token.is_some() {
                "yes"
            } else {
                "no"
            }
        ));
    } else {
        lines.push("stored login: none (recommended: /login desktop)".into());
    }
    lines.push(format!(
        "client version: {} ({})",
        app.opts.client_version, app.client_version_source
    ));
    if app.store.is_startup_account()
        && std::env::var(crate::config::TOKEN_ENV).is_ok_and(|t| !t.trim().is_empty())
    {
        lines.push(format!(
            "{} override: active for this account",
            crate::config::TOKEN_ENV
        ));
    }
    lines.push(format!("database: {}", app.store.db_path().display()));
    lines.push("(The gateway has no identity call, so no user name is available.)".into());
    app.info(lines.join("\n"));
    Ok(())
}

fn bots(app: &mut App, _: &str) -> Result<()> {
    app.refresh_bots(true);
    Ok(())
}

fn bot(app: &mut App, args: &str) -> Result<()> {
    if args.is_empty() {
        app.open_bot_picker();
        return Ok(());
    }
    let (sub, rest) = args.split_once(' ').unwrap_or((args, ""));
    let rest = rest.trim();
    match sub {
        "new" => {
            if rest.is_empty() {
                bail!("Usage: /bot new <name>");
            }
            app.create_bot(rest);
            return Ok(());
        }
        "delete" => {
            let i = if rest.is_empty() {
                app.active
                    .ok_or_else(|| anyhow::anyhow!("No active bot. Usage: /bot delete <name>"))?
            } else {
                app.find_bot(rest)
                    .ok_or_else(|| anyhow::anyhow!("No bot named {rest}. Try /bots."))?
            };
            app.confirm_delete_bot(i);
            return Ok(());
        }
        _ => {}
    }
    match app.find_bot(args) {
        Some(i) => {
            app.select_bot(i);
            Ok(())
        }
        None => bail!("No bot named {args}. Try /bots, or /bot new {args} to create it."),
    }
}

fn history(app: &mut App, _: &str) -> Result<()> {
    if app.active_bot().is_none() {
        bail!("No active bot")
    }
    app.request_history();
    app.push(
        EntryKind::System,
        "Fetching chat history from the gateway…".into(),
    );
    Ok(())
}

fn account(app: &mut App, args: &str) -> Result<()> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    match parts.as_slice() {
        [] | ["list"] => {
            let cur = app.account().id;
            let list = app
                .store
                .accounts()?
                .into_iter()
                .map(|a| format!("{} {}", if a.id == cur { "*" } else { " " }, a.alias))
                .collect::<Vec<_>>()
                .join("\n");
            app.info(format!("Accounts (* = active):\n{list}"));
        }
        ["add", alias] => {
            let a = app.store.add_account(alias)?;
            app.info(format!(
                "Added account '{}'. Switch with /account use {}.",
                a.alias, a.alias
            ));
        }
        ["use"] => app.open_account_picker(),
        ["use", alias] => app.switch_account(alias)?,
        ["rename", old, new] => {
            app.store.rename_account(old, new)?;
            app.info(format!("Renamed account '{old}' to '{new}'."));
        }
        ["remove", alias] => {
            let a = app
                .store
                .find_account(alias)?
                .with_context(|| format!("no account named '{alias}'"))?;
            if a.id == app.account().id {
                bail!("cannot remove the active account; switch with /account use first");
            }
            app.pending_remove = Some(a.alias.clone());
            use crate::picker::{PickItem, Picker};
            app.open_picker(
                Picker::new(
                    format!("Delete account '{}' and ALL its data?", a.alias),
                    vec![
                        PickItem::new("Cancel", "cancel"),
                        PickItem::new(
                            format!("Delete '{}' (credentials, chats, memory, prefs)", a.alias),
                            "delete",
                        ),
                    ],
                    |app, it| {
                        let alias = app.pending_remove.clone().unwrap_or_default();
                        let typed = if it.value == "delete" {
                            alias
                        } else {
                            String::new()
                        };
                        finish_remove(app, &typed);
                    },
                )
                .on_cancel(|app| finish_remove(app, "")),
            );
        }
        _ => bail!(
            "Usage: /account [list | add <alias> | use <alias> | rename <old> <new> | remove <alias>]"
        ),
    }
    Ok(())
}

pub fn finish_remove(app: &mut App, typed: &str) {
    let Some(alias) = app.pending_remove.take() else {
        return;
    };
    if typed != alias {
        app.info("Account removal cancelled.".into());
        return;
    }
    match app.store.remove_account(&alias) {
        Ok(()) => app.info(format!("Removed account '{alias}' and all its data.")),
        Err(e) => app.error(format!("{e:#}")),
    }
}

fn prefs(app: &mut App, args: &str) -> Result<()> {
    let mut it = args.splitn(3, char::is_whitespace);
    match (it.next().unwrap_or(""), it.next(), it.next()) {
        ("" | "list", None, None) => {
            let list = app
                .store
                .list_prefs_scoped()?
                .into_iter()
                .map(|(s, k, v)| {
                    let scope = if s == Scope::Global {
                        "global"
                    } else {
                        "account"
                    };
                    format!("  {scope:<8} {k:<22} {v}")
                })
                .collect::<Vec<_>>()
                .join("\n");
            app.info(format!(
                "Preferences (account '{}'):\n{list}",
                app.account().alias
            ));
        }
        ("get", Some(k), None) => {
            let v = app.store.get_pref(k)?;
            app.info(format!("{k} = {}", v.as_deref().unwrap_or("(unset)")));
        }
        ("set", Some(k), v) => {
            let v = v.unwrap_or("").trim();
            app.store.set_pref(k, v)?;
            app.reload_config()?;
            if k == "show_hidden_bots" && !app.all_bots.is_empty() {
                let all = app.all_bots.clone();
                app.apply_bots(all);
            }
            let scope = if crate::config::is_global(k) {
                "global"
            } else {
                "account"
            };
            app.info(format!("{k} = {v} ({scope})"));
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
