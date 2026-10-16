//! In-app help: topics, per-command details, error hints, and "did you mean".
//!
//! Help is shown in a modal (see [`HelpModal`]) and is never written to chat history.
//! Help text is static and never includes credentials.
use crate::commands::Registry;

/// A help topic. `id` is what `/help <id>` takes (spaces map to `-`).
pub struct Topic {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub related: &'static [&'static str],
}

pub const TOPICS: &[Topic] = &[
    Topic {
        id: "overview",
        title: "grokbot-tui help",
        body: "\
grokbot-tui is a terminal client for Grok Bot (unofficial, uses an undocumented gateway API).

Getting started
  1. Sign in to the Grok Bot desktop app once.
  2. Run /login desktop  (recommended — no token copying).
  3. Pick a bot in the left sidebar and type a message.

Plain text is sent to the active bot. Lines starting with / are commands.

Help topics (Enter on a link below opens it):
  login · login desktop · login cursor · login gateway
  keys · accounts · prefs · env · sync · delivery · troubleshooting

Search: /help <keyword> (e.g. /help keyring), or press / in this window.

Commands: /help <command> shows usage, examples and related topics.
  help login logout whoami bots bot history account prefs memory quit",
        related: &[
            "login",
            "login-desktop",
            "keys",
            "accounts",
            "sync",
            "troubleshooting",
        ],
    },
    Topic {
        id: "login",
        title: "Logging in",
        body: "\
Each gb account logs in once; the login is stored in ~/.gb/context.db for that account only.

Which method?
  ★ /login desktop  RECOMMENDED. Uses the Grok Bot desktop app's saved session.
                    Requires the app installed and signed in on this computer.
    /login cursor   Paste a Cursor access token; gb exchanges it for a gateway session.
                    Getting this token by hand is not documented — use /login desktop.
    /login gateway  Paste a gateway URL + gateway token directly.
                    Getting these by hand is not documented — use /login desktop.

Credentials are only sent to allowed hosts (see /help env, GB_ALLOW_LOCAL_GATEWAY).
/logout removes this account's stored login and keeps chats and memory.
/whoami shows how the account is logged in (never the token).",
        related: &[
            "login-desktop",
            "login-cursor",
            "login-gateway",
            "accounts",
            "troubleshooting",
        ],
    },
    Topic {
        id: "login-desktop",
        title: "/login desktop (recommended)",
        body: "\
Usage: /login desktop [entry]

Steps
  1. Install the Grok Bot desktop app and sign in once.
  2. Linux only: install secret-tool (package libsecret-tools) and keep your keyring unlocked.
  3. Run /login desktop. gb reads the app's saved session, checks it by listing bots,
     and saves it to the active gb account.

What gb reads (only when you run this command, or on a 401 for a desktop-login account)
  App data folder:
    Linux    $XDG_CONFIG_HOME/Grok Bot  or  ~/.config/Grok Bot
    macOS    ~/Library/Application Support/Grok Bot
    Windows  %APPDATA%\\Grok Bot
  1st choice: gateway-descriptor.json (a ready gateway session).
  Fallback:   sand-secrets.json → cursor-accounts → Cursor access token, exchanged via
              EnsureSandBox like /login cursor. That token is not stored by gb.
  Decryption uses the OS keychain: secret-tool on Linux, the Keychain item
  'Grok Bot Safe Storage' on macOS, DPAPI on Windows. Nothing is printed or logged.

Several saved sessions/accounts
  gb opens a pick list (e.g. '1) 3f2a9c1e… (active in app)'); choose with ↑↓/Enter or
  the mouse. Or name it directly: /login desktop <number>.
  Tip: /account add <alias> first so each desktop account maps to its own gb account.

Client version
  gb detects the installed app version for x-cursor-client-version (see /help env).

Errors
  APP_NOT_FOUND        App data folder missing → install the app / check the path above.
  NOT_SIGNED_IN        No saved session → open the app and sign in.
  SECRET_TOOL_MISSING  Install libsecret-tools (provides secret-tool).
  KEYCHAIN_LOCKED      Unlock your keyring / log in to your desktop session.
  AMBIGUOUS_ENTRIES    Several sessions → /login desktop <number>.
  NO_SUCH_ENTRY        That number/name doesn't exist → run /login desktop to list.
  DECRYPT_FAILED       Wrong keychain password or corrupt data → sign out/in in the app.
  'version no longer supported' → update the app, or set GB_CURSOR_CLIENT_VERSION.",
        related: &[
            "login",
            "login-cursor",
            "accounts",
            "env",
            "troubleshooting",
        ],
    },
    Topic {
        id: "login-cursor",
        title: "/login cursor",
        body: "\
Usage: /login cursor   (then paste the token; input is masked, Esc cancels)

What it does
  gb sends the token to https://api2.cursor.sh/aiserver.v1.GrokBotService/EnsureSandBox
  and gets back a gateway URL, gateway token and routing headers. The Cursor token is
  stored for this account so gb can re-exchange it when the gateway answers 401.

Getting a Cursor access token
  Not documented — use /login desktop. The desktop app keeps this token encrypted in
  sand-secrets.json; /login desktop reads and exchanges it for you automatically.
  A Cursor dashboard API key does NOT work here.

Headers sent: x-cursor-client-type (sand), x-cursor-client-version (detected app version),
x-sand-box-namespace (prod). Override with GB_CURSOR_CLIENT_* / prefs (see /help env).

Errors
  401 'version no longer supported' → update the desktop app or set GB_CURSOR_CLIENT_VERSION.
  417 (empty body) → the gateway session expired; gb re-authenticates like a 401
    (desktop re-read / Cursor re-exchange). If it persists, run /login desktop.
  'no gatewayUrl/gatewayToken' → the token was rejected or is not an access token.",
        related: &["login", "login-desktop", "env", "troubleshooting"],
    },
    Topic {
        id: "login-gateway",
        title: "/login gateway",
        body: "\
Usage: /login gateway [url]   (then paste the gateway token; masked, Esc cancels)

What it does
  Uses a gateway session directly: a URL like https://<box>.cursorvm.com plus its token.
  gb checks the URL against the host allowlist, validates by listing bots, then saves it.

Getting a gateway URL + token
  Not documented — use /login desktop. These are issued by EnsureSandBox (what
  /login cursor and /login desktop do for you) and are not shown in the desktop app.

Non-interactive use (startup account only, never saved)
  GB_GROKBOT_TOKEN=<token> GB_GROKBOT_GATEWAY_URL=<url> grokbot-tui
  Optional GB_GROKBOT_GATEWAY_HEADERS='{\"x-anyrun-network-token\":\"…\"}'

Allowed hosts: https on cursor.sh, cursor.com, *.cursor.sh, *.cursor.com, *.cursorvm.com.
Loopback only with GB_ALLOW_LOCAL_GATEWAY=1. URLs with user:pass@ are rejected.",
        related: &["login", "login-desktop", "env", "troubleshooting"],
    },
    Topic {
        id: "keys",
        title: "Keys",
        body: "\
Anywhere
  F1, or ? on an empty input   Open this help
  F2                           Pick a bot (list popup; same as /bot)
  F3                           Pick an account (same as /account use)
  Ctrl-C / Ctrl-D              Quit
  Tab                          Switch focus between input and sidebar
  PgUp / PgDn, mouse wheel     Scroll the chat
  Esc                          Cancel a prompt (login, confirm) / return focus to input

Input focused (multi-line)
  Enter                     Send message or run /command
  Shift-Enter               New line (terminals with the kitty keyboard protocol:
                            kitty, WezTerm, foot, Ghostty, recent Alacritty/iTerm2)
  Alt-Enter or Ctrl-J       New line (fallback that works in any terminal)
  ← → / Home End            Move the cursor / to the start or end of the line
  ↑ / ↓                     Previous / next line; on the first / last line, scroll the chat
  Backspace / Delete        Delete before / at the cursor
  Long lines wrap; the box grows up to 10 rows, then scrolls to keep the cursor
  visible (row/total shown bottom-right). Pasted text keeps its line breaks.

Sidebar focused
  ↑ ↓ / k j  Move      Enter  Open bot

Mouse
  Click a bot to open it. Click the @account header to pick an account.

In a pick list (F2, F3, /bot, /account use)
  ↑ ↓ / j k, PgUp PgDn, Home End  Move      Enter  Pick      Esc / q  Cancel
  Mouse: hover highlights, click picks, click outside cancels, wheel moves

In this help window
  ↑ ↓ / j k, PgUp PgDn  Scroll      Tab / ← →  Select a related link
  Enter  Open link      Backspace  Back      Esc / q  Close
  /      Search all help by keyword (type, Enter to run, Esc to cancel)",
        related: &["overview", "accounts"],
    },
    Topic {
        id: "accounts",
        title: "Accounts",
        body: "\
gb keeps several accounts fully separate: logins, bots, chats, memory and prefs.

  /account                  List accounts (* = active)
  /account add <alias>      Create an account
  /account use [alias]      Switch (no alias, F3 or clicking @account: pick list)
  /account rename <a> <b>   Rename
  /account remove <alias>   Delete (confirm in a pick list; Cancel is the default)

Start in an account: grokbot-tui --account work  or  GB_ACCOUNT=work grokbot-tui
GB_GROKBOT_TOKEN only applies to the account selected at startup.
Several desktop Cursor accounts? Add one gb account each, then /login desktop <number>.",
        related: &["login", "login-desktop", "prefs", "env"],
    },
    Topic {
        id: "prefs",
        title: "Preferences",
        body: "\
/prefs                 List preferences
/prefs get <key>       Show one
/prefs set <key> <v>   Set (empty value resets to default)

Per-account keys
  default_bot            Bot id/name to open when nothing else is selected. Default: unset.
  active_bot             Last opened bot (set automatically).
  show_hidden_bots       true/false — show bots hidden from the sidebar. Default: false.
  cursor.client_version  Override x-cursor-client-version for EnsureSandBox.
                         Default: detected desktop app version, else 0.70.2.
  cursor.client_type     Override x-cursor-client-type. Default: sand.
  sand.box_namespace     Override x-sand-box-namespace. Default: prod.

Global keys
  ui.sidebar_width       Sidebar width, 10–80. Default: 24.
  active_account         Account opened at startup (set automatically).

Environment variables override the cursor.* / sand.* prefs (see /help env).",
        related: &["env", "accounts", "login-cursor"],
    },
    Topic {
        id: "env",
        title: "Environment variables",
        body: "\
Login (startup account only, never saved)
  GB_GROKBOT_TOKEN            Gateway token override.
  GB_GROKBOT_GATEWAY_URL      Gateway URL for that token (else the stored URL).
  GB_GROKBOT_GATEWAY_HEADERS  JSON object of extra gateway headers.

EnsureSandBox (Cursor token exchange)
  GB_CURSOR_API_BASE_URL      Backend base (must be *.cursor.sh / *.cursor.com).
  GB_CURSOR_CLIENT_VERSION    x-cursor-client-version. Priority:
                              env > pref cursor.client_version > detected app > 0.70.2
  GB_CURSOR_CLIENT_TYPE       x-cursor-client-type (default sand).
  GB_SAND_BOX_NAMESPACE       x-sand-box-namespace (default prod).

Other
  GB_ACCOUNT                  Starting account alias (same as --account).
  GB_ALLOW_LOCAL_GATEWAY=1    Allow http(s) loopback gateways (development).
  GB_GROKBOT_APP_DIR          Grok Bot install dir for version detection.

Data lives in ~/.gb/context.db (override with --data-dir). /whoami shows the
client version in use and where it came from.",
        related: &["prefs", "login-cursor", "login-gateway", "troubleshooting"],
    },
    Topic {
        id: "sync",
        title: "Chat sync",
        body: "\
Bots come from the gateway's listAgents. Messages come from getAgentTranscriptTail
(there is no separate chat list).

  • Opening a bot (or /history) pages the whole transcript (up to 1000 entries),
    merges it into ~/.gb/context.db and redraws the chat from there — so messages
    from the desktop app appear too.
  • While a bot is open gb polls for new messages every ~1 s, backing off to 15 s
    when idle, and every 1 s right after you send.
  • The chat you see is always rebuilt from that database. Your sent message is saved
    there first (as pending) and shows immediately; notices and errors appear in a
    separate section below the chat and are not saved.
  • /bots refreshes the bot list.",
        related: &["delivery", "troubleshooting"],
    },
    Topic {
        id: "delivery",
        title: "Sending and delivery",
        body: "\
Every message you send shows as pending until the transcript confirms it.

  Accepted  The gateway returned a receipt. The message is confirmed when it appears.
  Rejected  The gateway refused it (4xx, policy). It was NOT delivered; fix and resend.
  Unknown   5xx, network error, timeout or no receipt. gb then asks the gateway
            (promptAcceptanceStatus, by the message's nonce) whether it landed:
              accepted  → nothing to do; it will appear in the chat.
              rejected  → 'Not delivered (rejected by the gateway: <code>)'.
              not found → 'Not delivered … Safe to resend' (it never arrived).
              pending   → asked again a few times.
            Only if the gateway can't tell does gb show 'Delivery unknown'; then wait
            for the message to appear (or not) before resending. gb never resends itself.",
        related: &["sync", "troubleshooting"],
    },
    Topic {
        id: "troubleshooting",
        title: "Troubleshooting",
        body: "\
'Gateway rejected the credential (401)'
  desktop login: gb re-reads the app session once automatically; if it still fails,
  open the app, make sure you're signed in, then /login desktop.
  cursor login: gb re-exchanges once; otherwise /login cursor with a fresh token.
  gateway login: /login gateway with a new token.

'This version of Grok Bot is no longer supported'
  Update the desktop app, or set GB_CURSOR_CLIENT_VERSION / pref cursor.client_version.
  /whoami shows the version in use.

Desktop errors (APP_NOT_FOUND, NOT_SIGNED_IN, SECRET_TOOL_MISSING, KEYCHAIN_LOCKED, …)
  See /help login desktop for each fix.

'host not allowed' — the URL isn't on the allowlist (see /help login gateway).

Chat looks out of date — /history reloads the full transcript.
No bots — create one in the desktop app, then /bots.
Running gb prints git errors — your shell may alias gb; the binary is grokbot-tui.",
        related: &[
            "login-desktop",
            "login-cursor",
            "login-gateway",
            "sync",
            "env",
        ],
    },
];

/// Normalize `/help` arguments to a topic id: "login desktop" → "login-desktop".
pub fn normalize(arg: &str) -> String {
    arg.trim()
        .trim_start_matches('/')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

pub fn topic(id: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.id == id)
}

/// Extra details for `/help <command>`: (description, examples, related topics).
fn command_extra(
    name: &str,
) -> (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
) {
    match name {
        "help" => (
            "Open help. With a topic or command name, open that page; any other words search all help pages.",
            &[
                "/help",
                "/help login desktop",
                "/help keys",
                "/help bot",
                "/help keyring",
                "/help search 401",
            ],
            &["overview", "keys"],
        ),
        "login" => (
            "Log in the active account. /login desktop is recommended. Bare /login opens login help.",
            &[
                "/login desktop",
                "/login desktop 2",
                "/login cursor",
                "/login gateway https://box.cursorvm.com",
            ],
            &["login", "login-desktop", "login-cursor", "login-gateway"],
        ),
        "logout" => (
            "Delete the active account's stored login. Chats, memory and prefs are kept.",
            &["/logout"],
            &["login", "accounts"],
        ),
        "whoami" => (
            "Show the account, login method, gateway host and the client version in use (never tokens).",
            &["/whoami"],
            &["login", "env"],
        ),
        "bots" => (
            "Refresh the bot list from the gateway and print it.",
            &["/bots"],
            &["sync", "prefs"],
        ),
        "bot" => (
            "Open a bot by name or id (also: click it, F2, or bare /bot for the pick list).\n\
             /bot new <name> creates a bot on the gateway (createAgent) and opens it.\n\
             /bot delete [name] deletes the named (or current) bot (deleteAgent) after a\n\
             confirmation pick list where Cancel is the default. Deleting is permanent.",
            &[
                "/bot Helper",
                "/use Helper",
                "/bot new Researcher",
                "/bot delete Researcher",
            ],
            &["sync", "keys"],
        ),
        "shell" => (
            "Run a command on THIS machine with $SHELL -c (sh if unset). Purely local:\n\
             nothing is sent to the bot/gateway and nothing is saved to chat history.\n\
             Output opens in a popup: the command line, stdout then stderr, and the exit\n\
             code. It shows 'running…' until the command finishes (the UI stays usable).\n\
             Scroll with ↑/↓, PgUp/PgDn, Home/End or the mouse wheel; Esc or q closes.\n\
             No stdin (interactive programs won't work); killed after 120s; output capped\n\
             at 500 lines / 64 KB. Red border = nonzero exit; exit 127 usually means\n\
             command not found.",
            &["/shell ls -la", "/shell git status", "/sh uptime"],
            &["keys"],
        ),
        "history" => (
            "Reload the active bot's full transcript from the gateway.",
            &["/history"],
            &["sync"],
        ),
        "account" => (
            "Manage separate accounts.",
            &[
                "/account",
                "/account add work",
                "/account use work",
                "/account rename work job",
                "/account remove job",
            ],
            &["accounts", "login"],
        ),
        "prefs" => (
            "List, read or change preferences.",
            &[
                "/prefs",
                "/prefs set show_hidden_bots true",
                "/prefs set cursor.client_version 0.70.2",
            ],
            &["prefs", "env"],
        ),
        "memory" => (
            "Local notes per bot (stored only in ~/.gb, never sent).",
            &["/memory", "/memory add lang rust", "/memory clear lang"],
            &["accounts"],
        ),
        "quit" => ("Exit grokbot-tui.", &["/quit"], &["keys"]),
        _ => ("", &[], &["overview"]),
    }
}

/// A rendered help page: title, body, related topic ids.
pub struct Page {
    pub id: String,
    pub title: String,
    pub body: String,
    pub related: Vec<String>,
}

/// Resolve `/help <arg>` to a page: a topic, a command, `search <words>`, or —
/// when nothing matches exactly — keyword search results (if any).
pub fn page(registry: &Registry, arg: &str) -> Option<Page> {
    let raw = arg.trim();
    if let Some(q) = raw
        .strip_prefix("search:")
        .or_else(|| raw.strip_prefix("search "))
    {
        return Some(search_page(registry, q.trim()));
    }
    if let Some(p) = exact_page(registry, raw) {
        return Some(p);
    }
    (!search(registry, raw).is_empty()).then(|| search_page(registry, raw))
}

/// One keyword-search hit.
pub struct Hit {
    pub id: String,
    pub title: String,
    pub snippet: String,
}

/// Search all topics and command pages. Every query word must appear (case-
/// insensitive) in the id, title or body. Title/id matches rank first.
pub fn search(registry: &Registry, query: &str) -> Vec<Hit> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.trim_start_matches('/').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return vec![];
    }
    let mut hits: Vec<(usize, Hit)> = vec![];
    for name in all_names(registry) {
        let Some(p) = exact_page(registry, &name) else {
            continue;
        };
        let head = format!("{} {}", p.id, p.title).to_lowercase();
        let body = p.body.to_lowercase();
        if !words.iter().all(|w| head.contains(w) || body.contains(w)) {
            continue;
        }
        let score = words.iter().filter(|w| head.contains(*w)).count();
        let snippet = p
            .body
            .lines()
            .find(|l| words.iter().any(|w| l.to_lowercase().contains(w)))
            .unwrap_or_else(|| p.body.lines().next().unwrap_or(""))
            .trim();
        let snippet: String = if snippet.chars().count() > 90 {
            format!("{}…", snippet.chars().take(89).collect::<String>())
        } else {
            snippet.to_string()
        };
        hits.push((
            score,
            Hit {
                id: p.id,
                title: p.title,
                snippet,
            },
        ));
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    hits.into_iter().map(|(_, h)| h).collect()
}

fn search_page(registry: &Registry, q: &str) -> Page {
    let hits = search(registry, q);
    let mut body = if hits.is_empty() {
        format!("No help pages match '{q}'.\n\nTry fewer or different words, or browse /help.\n")
    } else {
        format!(
            "{} result(s) for '{q}'. Tab/←→ to pick one below, Enter to open.\n\n",
            hits.len()
        )
    };
    for h in &hits {
        body.push_str(&format!(
            "{}  ({})\n    {}\n\n",
            h.title,
            link_label(&h.id),
            h.snippet
        ));
    }
    body.push_str("Press / to search again.");
    let mut related: Vec<String> = hits.into_iter().map(|h| h.id).collect();
    if related.is_empty() {
        related.push("overview".into());
    }
    Page {
        id: format!("search:{q}"),
        title: format!("search \"{q}\""),
        body,
        related,
    }
}

fn exact_page(registry: &Registry, arg: &str) -> Option<Page> {
    let id = normalize(arg);
    let id = if id.is_empty() {
        "overview".to_string()
    } else {
        id
    };
    if let Some(t) = topic(&id) {
        return Some(Page {
            id: t.id.into(),
            title: t.title.into(),
            body: t.body.into(),
            related: t.related.iter().map(|s| s.to_string()).collect(),
        });
    }
    let c = registry.find(id.strip_prefix("cmd-").unwrap_or(&id))?;
    let (desc, examples, related) = command_extra(c.name);
    let mut body = format!("Usage: {}\n\n{}\n", c.usage, c.help);
    if !desc.is_empty() {
        body.push_str(&format!("\n{desc}\n"));
    }
    if !c.aliases.is_empty() {
        body.push_str(&format!(
            "\nAliases: {}\n",
            c.aliases
                .iter()
                .map(|a| format!("/{a}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !examples.is_empty() {
        body.push_str("\nExamples:\n");
        for e in examples {
            body.push_str(&format!("  {e}\n"));
        }
    }
    Some(Page {
        id: format!("cmd-{}", c.name),
        title: format!("/{}", c.name),
        body,
        related: related.iter().map(|s| s.to_string()).collect(),
    })
}

/// How a related id reads in the modal ("/help login desktop").
pub fn link_label(id: &str) -> String {
    format!(
        "/help {}",
        id.strip_prefix("cmd-").unwrap_or(id).replace('-', " ")
    )
}

/// Every name `/help` accepts (topics + commands), for "did you mean".
pub fn all_names(registry: &Registry) -> Vec<String> {
    let mut v: Vec<String> = TOPICS.iter().map(|t| t.id.to_string()).collect();
    v.extend(registry.commands.iter().map(|c| c.name.to_string()));
    v
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest candidate within a small edit distance (or a unique prefix match).
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let input = input.to_lowercase();
    let cands: Vec<&str> = candidates.into_iter().collect();
    let prefixed: Vec<&&str> = cands.iter().filter(|c| c.starts_with(&input)).collect();
    if !input.is_empty() && prefixed.len() == 1 {
        return Some(prefixed[0].to_string());
    }
    cands
        .iter()
        .map(|c| (levenshtein(&input, c), *c))
        .filter(|(d, c)| *d <= 2.max(c.len() / 3) && *d < c.len())
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.to_string())
}

/// A fix / pointer for an error message, if we recognise it.
pub fn hint_for_error(msg: &str) -> Option<&'static str> {
    let m = msg.to_lowercase();
    let rules: &[(&str, &str)] = &[
        (
            "no longer supported",
            "Update the Grok Bot app or set GB_CURSOR_CLIENT_VERSION. See /help troubleshooting.",
        ),
        (
            "app data was not found",
            "Install the Grok Bot desktop app. See /help login desktop.",
        ),
        (
            "no saved session",
            "Open the Grok Bot app and sign in, then /login desktop.",
        ),
        (
            "no saved cursor accounts",
            "Open the Grok Bot app and sign in, then /login desktop.",
        ),
        (
            "no cursor-accounts",
            "Open the Grok Bot app and sign in, then /login desktop.",
        ),
        (
            "secret-tool",
            "Install libsecret-tools (provides secret-tool). See /help login desktop.",
        ),
        (
            "keychain did not return",
            "Unlock your keyring, then /login desktop. See /help login desktop.",
        ),
        (
            "several cursor accounts",
            "Pick one with /login desktop <number>. See /help login desktop.",
        ),
        (
            "several saved sessions",
            "Pick one with /login desktop <name>. See /help login desktop.",
        ),
        (
            "could not decrypt",
            "Sign out and back in to the Grok Bot app. See /help login desktop.",
        ),
        (
            "not allowed",
            "Only Cursor hosts are allowed. See /help login gateway.",
        ),
        ("401", "See /help troubleshooting (401)."),
        (
            "dashboard api key",
            "Use /login desktop instead. See /help login cursor.",
        ),
    ];
    rules.iter().find(|(k, _)| m.contains(k)).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry;

    #[test]
    fn every_topic_and_command_renders_with_resolvable_related() {
        let r = registry();
        for name in all_names(&r) {
            let p = page(&r, &name).unwrap_or_else(|| panic!("no page for {name}"));
            assert!(!p.body.trim().is_empty(), "{name}");
            assert!(!p.related.is_empty(), "{name} has no related links");
            for rel in &p.related {
                assert!(page(&r, rel).is_some(), "{name}: dangling related {rel}");
            }
        }
    }

    #[test]
    fn normalize_and_lookup() {
        let r = registry();
        assert_eq!(page(&r, "login desktop").unwrap().id, "login-desktop");
        assert_eq!(page(&r, "/bot").unwrap().id, "cmd-bot");
        assert_eq!(page(&r, "").unwrap().id, "overview");
        assert!(page(&r, "nope").is_none());
        assert_eq!(link_label("login-desktop"), "/help login desktop");
        assert_eq!(link_label("cmd-bot"), "/help bot");
    }

    #[test]
    fn suggestions_and_hints() {
        assert_eq!(
            suggest("lgoin", ["login", "logout", "help"]).as_deref(),
            Some("login")
        );
        assert_eq!(
            suggest("hist", ["history", "help"]).as_deref(),
            Some("history")
        );
        assert_eq!(suggest("zzzzzz", ["login", "help"]), None);
        assert!(
            hint_for_error(
                "EnsureSandBox failed: 401 This version of Grok Bot is no longer supported"
            )
            .unwrap()
            .contains("GB_CURSOR_CLIENT_VERSION")
        );
        assert!(
            hint_for_error("Could not run secret-tool")
                .unwrap()
                .contains("libsecret")
        );
        assert!(hint_for_error("all good").is_none());
    }

    #[test]
    fn keyword_search_matches_topics_and_commands() {
        let r = registry();
        let ids = |q: &str| search(&r, q).into_iter().map(|h| h.id).collect::<Vec<_>>();
        assert!(ids("keyring").contains(&"login-desktop".to_string()));
        assert!(ids("GB_CURSOR_CLIENT_VERSION").contains(&"env".to_string()));
        assert!(ids("examples rename").contains(&"cmd-account".to_string()));
        assert_eq!(ids("prefs")[0], "prefs", "title match ranks first");
        assert!(ids("secret-tool libsecret").iter().all(|i| i != "keys"));
        assert!(ids("zzqqxx").is_empty());
        assert!(ids("").is_empty());
        let hit = &search(&r, "keyring")[0];
        assert!(hit.snippet.to_lowercase().contains("keyring"));
        // page(): exact first, else search with results, explicit search always.
        assert_eq!(page(&r, "keys").unwrap().id, "keys");
        let p = page(&r, "keyring").unwrap();
        assert_eq!(p.id, "search:keyring");
        assert!(p.related.iter().all(|id| page(&r, id).is_some()));
        assert!(page(&r, "zzqqxx").is_none());
        assert!(
            page(&r, "search zzqqxx")
                .unwrap()
                .body
                .contains("No help pages")
        );
        assert_eq!(page(&r, "search:keyring").unwrap().id, "search:keyring");
    }

    #[test]
    fn desktop_is_recommended() {
        let p = topic("login").unwrap();
        assert!(p.body.contains("RECOMMENDED"));
        assert!(topic("overview").unwrap().body.contains("/login desktop"));
    }
}
