//! The MCP tools and resources, independent of the transport: [`Ctx::call`] takes a tool
//! name and JSON arguments and returns JSON, on top of the TUI's [`ApiClient`].
//!
//! Rules: tokens, presigned URLs and secret values never appear in a result or the audit
//! log. `allowed_bots` limits every bot argument; `read_only` refuses writes.
use super::config::{Audit, ServerConfig};
use crate::api::{ApiClient, Attachment, Bot, Message, Role};
use crate::cards::{self, Body, Card};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A tool failure the client sees (`isError: true`). Never put secrets in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError(pub String);

impl<T: std::fmt::Display> From<T> for ToolError {
    fn from(e: T) -> Self {
        Self(e.to_string())
    }
}

type R = Result<Value, ToolError>;

pub struct Ctx {
    pub api: Arc<dyn ApiClient>,
    pub cfg: ServerConfig,
    pub audit: Audit,
    /// Last bot list (for names, team sessions and the allowlist).
    bots: Mutex<Vec<Bot>>,
    pub download_dir: PathBuf,
    pub download_cap: u64,
    /// Polling interval for `wait_for_reply` (shorter in tests).
    pub poll: Duration,
    /// Called after each tool call so a refreshed gateway session can be saved.
    pub on_session: Option<Box<dyn Fn(crate::api::Session) + Send + Sync>>,
}

/// One tool: name, description and JSON Schema for its arguments.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub writes: bool,
}

fn bot_prop() -> Value {
    json!({"type": "string", "description": "Bot id, or its exact name (case-insensitive)"})
}

pub fn tool_defs() -> Vec<ToolDef> {
    let obj = |props: Value, req: &[&str]| json!({"type": "object", "properties": props, "required": req});
    vec![
        ToolDef {
            name: "list_bots",
            description: "List the Grok Bots this server may use (id, name, description, team/group flags).",
            schema: obj(
                json!({"include_hidden": {"type": "boolean", "description": "Include bots hidden from the sidebar (default false)"}}),
                &[],
            ),
            writes: false,
        },
        ToolDef {
            name: "get_bot",
            description: "Details for one bot.",
            schema: obj(json!({"bot": bot_prop()}), &["bot"]),
            writes: false,
        },
        ToolDef {
            name: "send_message",
            description: "Send a message to a bot as the signed-in user. Returns message_id for wait_for_reply. Optional attachments are local file paths to upload.",
            schema: obj(
                json!({"bot": bot_prop(), "text": {"type": "string"},
                "attachments": {"type": "array", "items": {"type": "string"}, "description": "Local file paths"}}),
                &["bot", "text"],
            ),
            writes: true,
        },
        ToolDef {
            name: "read_messages",
            description: "Recent messages in a bot's chat, oldest first. `since` = a message id: only messages after it.",
            schema: obj(
                json!({"bot": bot_prop(), "since": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Default 20"}}),
                &["bot"],
            ),
            writes: false,
        },
        ToolDef {
            name: "wait_for_reply",
            description: "Wait until the bot answers the message `message_id` (from send_message; default: your latest message), then return the reply messages and any new pending cards.",
            schema: obj(
                json!({"bot": bot_prop(), "message_id": {"type": "string"},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600, "description": "Default 120"}}),
                &["bot"],
            ),
            writes: false,
        },
        ToolDef {
            name: "list_pending_cards",
            description: "Interactive cards in a bot's chat waiting for an answer: questions (widget), approvals, secret requests, forms, drafts. Use answer_card with the card_id.",
            schema: obj(json!({"bot": bot_prop()}), &["bot"]),
            writes: false,
        },
        ToolDef {
            name: "answer_card",
            description: "Answer a pending card. widget: value (option value or custom text; several steps/options joined by newlines) or action=dismiss. approval: action=approve|deny. secret: value (never echoed) or action=dismiss. form: values {field_id: text} or action=dismiss. draft: action=send (optional body) or action=discard.",
            schema: obj(
                json!({"bot": bot_prop(), "card_id": {"type": "string"},
                "action": {"type": "string", "enum": ["answer", "dismiss", "approve", "deny", "send", "discard"]},
                "value": {"type": "string"},
                "values": {"type": "object", "additionalProperties": {"type": "string"}},
                "body": {"type": "string", "description": "Edited draft body for action=send"}}),
                &["bot", "card_id"],
            ),
            writes: true,
        },
        ToolDef {
            name: "list_attachments",
            description: "Files attached to recent messages in a bot's chat (path, name, size, type, message id).",
            schema: obj(
                json!({"bot": bot_prop(), "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Messages to scan (default 50)"}}),
                &["bot"],
            ),
            writes: false,
        },
        ToolDef {
            name: "download_attachment",
            description: "Download an attachment (its `path` from list_attachments) to a local folder. Returns the saved file path, never the download URL.",
            schema: obj(
                json!({"bot": bot_prop(), "path": {"type": "string"},
                "dest_dir": {"type": "string", "description": "Folder to save into (default: the server's download folder)"}}),
                &["bot", "path"],
            ),
            writes: false,
        },
    ]
}

fn arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}
fn req<'a>(a: &'a Value, k: &str) -> Result<&'a str, ToolError> {
    arg(a, k)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolError(format!("missing argument `{k}`")))
}
fn num(a: &Value, k: &str, def: u64, max: u64) -> u64 {
    a.get(k)
        .and_then(Value::as_u64)
        .unwrap_or(def)
        .clamp(1, max)
}

fn bot_json(b: &Bot) -> Value {
    json!({"id": b.id, "name": b.name, "description": b.description,
        "team": b.team, "group": b.is_group, "hidden": b.hidden})
}

fn att_json(a: &Attachment) -> Value {
    json!({"path": a.path, "name": a.name, "size": a.size, "content_type": a.content_type})
}

fn msg_json(m: &Message) -> Value {
    let mut v = json!({"id": m.remote_id, "role": match m.role { Role::User => "user", Role::Bot => "bot" }, "text": m.text});
    if !m.attachments.is_empty() {
        v["attachments"] = m.attachments.iter().map(att_json).collect();
    }
    v
}

/// What an MCP client needs to answer a card. Secret requests show only label/description.
pub fn card_json(c: &Card) -> Value {
    let (kind, details) = match &c.body {
        Body::Widget {
            help,
            steps,
            allow_custom,
        } => (
            "widget",
            json!({"help": help, "allow_custom": allow_custom, "steps": steps.iter().map(|s| json!({
                "prompt": s.prompt, "multi": s.multi,
                "options": s.options.iter().map(|o| json!({"label": o.label, "value": o.value, "description": o.description})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()}),
        ),
        Body::Approval {
            local,
            summary,
            command,
            reason,
            ..
        } => (
            "approval",
            json!({"local_tool": local, "summary": summary, "command": command, "reason": reason}),
        ),
        Body::Secret { label, description } => (
            "secret",
            json!({"label": label, "description": description}),
        ),
        Body::Form { title, fields } => (
            "form",
            json!({"title": title, "fields": fields.iter().map(|f| json!({
                "id": f.id, "label": f.label, "type": f.kind, "required": f.required,
                "secret": f.secret, "placeholder": f.placeholder})).collect::<Vec<_>>()}),
        ),
        Body::Draft {
            kind,
            from,
            to,
            subject,
            body,
            ..
        } => (
            "draft",
            json!({"draft_kind": kind, "from": from, "to": to, "subject": subject, "body": body}),
        ),
        Body::Info { kind, title, lines } => (
            "info",
            json!({"info_kind": kind, "title": title, "lines": lines, "note": "Not answerable here; use the Grok Bot app."}),
        ),
    };
    json!({"card_id": c.entry_id, "type": kind, "message_type": c.kind,
        "state": c.state.label(), "summary": c.summary(), "details": details})
}

fn kind_of(c: &Card) -> &'static str {
    match c.body {
        Body::Widget { .. } => "widget",
        Body::Approval { .. } => "approval",
        Body::Secret { .. } => "secret",
        Body::Form { .. } => "form",
        Body::Draft { .. } => "draft",
        Body::Info { .. } => "info",
    }
}

/// Build the gateway call for `answer_card`. The secret value moves into the call body
/// and nowhere else.
pub fn card_call_for(c: &Card, a: &Value) -> Result<cards::Call, ToolError> {
    let action = arg(a, "action").unwrap_or("answer");
    let value = arg(a, "value").map(str::to_string);
    let bad = |what: &str| ToolError(format!("a {} card takes {what}", kind_of(c)));
    let call = match (&c.body, action) {
        (Body::Info { .. }, _) => {
            return Err(ToolError(
                "this card can't be answered over MCP; use the Grok Bot app".into(),
            ));
        }
        (Body::Widget { .. } | Body::Secret { .. } | Body::Form { .. }, "dismiss")
        | (Body::Draft { .. }, "discard") => cards::dismiss(c),
        (Body::Widget { .. }, "answer") => Some(cards::answer_widget(
            c,
            value
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| bad("`value` or action=dismiss"))?,
        )),
        (Body::Approval { .. }, "approve" | "deny") => cards::approve(c, action == "approve"),
        (Body::Approval { .. }, _) => return Err(bad("action=approve or action=deny")),
        (Body::Secret { .. }, "answer") => Some(cards::submit_secret(
            c,
            value
                .filter(|v| !v.is_empty())
                .ok_or_else(|| bad("`value` or action=dismiss"))?,
        )),
        (Body::Form { fields, .. }, "answer") => {
            let m = a
                .get("values")
                .and_then(Value::as_object)
                .ok_or_else(|| bad("`values` {field_id: text} or action=dismiss"))?;
            let mut vals = Vec::new();
            for f in fields {
                match m.get(&f.id).and_then(Value::as_str) {
                    Some(v) => vals.push((f.id.clone(), v.to_string())),
                    None if f.required => {
                        return Err(ToolError(format!("form field `{}` is required", f.id)));
                    }
                    None => {}
                }
            }
            if let Some(k) = m.keys().find(|k| !fields.iter().any(|f| &f.id == *k)) {
                return Err(ToolError(format!("form has no field `{k}`")));
            }
            Some(cards::submit_form(c, vals))
        }
        (Body::Draft { .. }, "send") => cards::send_draft(c, arg(a, "body")),
        (Body::Draft { .. }, _) => return Err(bad("action=send or action=discard")),
        (_, other) => {
            return Err(ToolError(format!(
                "action `{other}` doesn't apply to a {} card",
                kind_of(c)
            )));
        }
    };
    call.ok_or_else(|| bad("a different action"))
}

/// Safe file name for a download (no separators or parent dirs).
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() { "attachment".into() } else { s }
}

/// `dest/name`, or `dest/name (2).ext` ... when taken.
fn free_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

impl Ctx {
    pub fn new(
        api: Arc<dyn ApiClient>,
        cfg: ServerConfig,
        audit: Audit,
        download_dir: PathBuf,
    ) -> Self {
        Self {
            api,
            cfg,
            audit,
            bots: Mutex::new(vec![]),
            download_dir,
            download_cap: 512 * 1024 * 1024,
            poll: Duration::from_secs(2),
            on_session: None,
        }
    }

    async fn refresh_bots(&self) -> Result<Vec<Bot>, ToolError> {
        let bots = self.api.list_bots().await?;
        for b in &bots {
            self.api
                .bind_viewer_session(&b.id, b.viewer_session_id.as_deref());
        }
        *self.bots.lock().unwrap() = bots.clone();
        Ok(bots)
    }

    /// Resolve a `bot` argument (id or name) to an allowed bot.
    async fn bot(&self, a: &Value) -> Result<Bot, ToolError> {
        let want = req(a, "bot")?.trim().to_string();
        let find = |bots: &[Bot]| {
            bots.iter()
                .find(|b| b.id == want)
                .or_else(|| bots.iter().find(|b| b.name.eq_ignore_ascii_case(&want)))
                .cloned()
        };
        let cached = find(&self.bots.lock().unwrap());
        let b = match cached {
            Some(b) => b,
            None => find(&self.refresh_bots().await?)
                .ok_or_else(|| ToolError(format!("no bot `{want}` (see list_bots)")))?,
        };
        if !self.cfg.allows(&b.id, &b.name) {
            return Err(ToolError(format!(
                "bot `{}` is not in mcp.server.allowed_bots",
                b.name
            )));
        }
        Ok(b)
    }

    fn write_guard(&self, tool: &str) -> Result<(), ToolError> {
        if self.cfg.read_only {
            return Err(ToolError(format!(
                "{tool}: this MCP server is read-only (mcp.server.read_only)"
            )));
        }
        Ok(())
    }

    /// Run a tool and audit it (tool, bot id, kind/action, ok or error; never bodies).
    pub async fn call(&self, name: &str, a: &Value) -> R {
        let a = if a.is_null() { &json!({}) } else { a };
        let mut detail = String::new();
        let r = self.dispatch(name, a, &mut detail).await;
        let bot = self
            .bots
            .lock()
            .unwrap()
            .iter()
            .find(|b| arg(a, "bot").is_some_and(|w| b.id == w || b.name.eq_ignore_ascii_case(w)))
            .map(|b| b.id.clone());
        let outcome = match &r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {}", e.0),
        };
        self.audit.record(name, bot.as_deref(), &detail, &outcome);
        if let (Some(f), Some(s)) = (&self.on_session, self.api.take_refreshed_session()) {
            f(s);
        }
        r
    }

    async fn dispatch(&self, name: &str, a: &Value, detail: &mut String) -> R {
        match name {
            "list_bots" => {
                let all = a
                    .get("include_hidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let bots = self.refresh_bots().await?;
                let v: Vec<Value> = bots
                    .iter()
                    .filter(|b| self.cfg.allows(&b.id, &b.name) && (all || !b.hidden))
                    .map(bot_json)
                    .collect();
                Ok(json!({"bots": v, "read_only": self.cfg.read_only}))
            }
            "get_bot" => Ok(bot_json(&self.bot(a).await?)),
            "send_message" => {
                self.write_guard(name)?;
                let b = self.bot(a).await?;
                let text = req(a, "text")?;
                let mut atts = vec![];
                if let Some(paths) = a.get("attachments").and_then(Value::as_array) {
                    if !paths.is_empty() && !self.api.can_attach() {
                        return Err(ToolError("this login can't upload attachments (needs a Cursor session: /login desktop or /login cursor)".into()));
                    }
                    for p in paths.iter().filter_map(Value::as_str) {
                        let path = crate::download::expand(p);
                        let data = tokio::fs::read(&path)
                            .await
                            .map_err(|e| format!("{}: {e}", path.display()))?;
                        let fname = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "file".into());
                        let up = self
                            .api
                            .upload(&b.id, &fname, data, Default::default())
                            .await?;
                        atts.push(Attachment {
                            path: up.alias,
                            name: fname,
                            ..Default::default()
                        });
                    }
                }
                *detail = format!("attachments={}", atts.len());
                let nonce = uuid::Uuid::new_v4().to_string();
                let r = self.api.send_with(&b.id, text, &nonce, &atts).await?;
                Ok(
                    json!({"message_id": r.message_id.clone().unwrap_or_else(|| r.client_nonce.clone()),
                    "nonce": r.client_nonce, "delivery": format!("{:?}", r.delivery).to_lowercase(), "bot": b.id}),
                )
            }
            "read_messages" => {
                let b = self.bot(a).await?;
                let limit = num(a, "limit", 20, 200) as usize;
                let fetch = if arg(a, "since").is_some() {
                    200
                } else {
                    limit as u32
                };
                let msgs = self.api.transcript(&b.id, fetch).await?;
                let start = match arg(a, "since") {
                    Some(s) => msgs
                        .iter()
                        .position(|m| m.remote_id.as_deref() == Some(s))
                        .map(|i| i + 1)
                        .unwrap_or(0),
                    None => msgs.len().saturating_sub(limit),
                };
                let out: Vec<Value> = msgs[start..].iter().take(limit).map(msg_json).collect();
                Ok(json!({"bot": b.id, "messages": out}))
            }
            "wait_for_reply" => {
                let b = self.bot(a).await?;
                let timeout = Duration::from_secs(num(a, "timeout_secs", 120, 600));
                let id = arg(a, "message_id").map(str::to_string);
                let started = std::time::Instant::now();
                let mut last: Option<usize> = None;
                loop {
                    let msgs = self.api.transcript(&b.id, 50).await?;
                    let anchor = id
                        .as_deref()
                        .and_then(|id| {
                            msgs.iter().rposition(|m| {
                                m.remote_id.as_deref() == Some(id)
                                    || m.remote_id.as_deref() == Some(&format!("pending:{id}"))
                            })
                        })
                        .or_else(|| msgs.iter().rposition(|m| m.role == Role::User));
                    let replies: Vec<&Message> = match anchor {
                        Some(i) => msgs[i + 1..]
                            .iter()
                            .filter(|m| m.role == Role::Bot)
                            .collect(),
                        None => vec![],
                    };
                    let pending: Vec<Value> = self
                        .api
                        .take_cards(&b.id)
                        .iter()
                        .filter(|c| c.state.is_pending())
                        .map(card_json)
                        .collect();
                    // Done once a reply is there and the transcript stopped growing (a
                    // bot may answer in several messages), or a card waits for an answer.
                    let n = replies.len();
                    if n > 0 && (last == Some(n) || !pending.is_empty()) {
                        *detail = format!("replies={n}");
                        return Ok(
                            json!({"status": "replied", "replies": replies.iter().map(|m| msg_json(m)).collect::<Vec<_>>(), "pending_cards": pending}),
                        );
                    }
                    if started.elapsed() >= timeout {
                        *detail = "timeout".into();
                        return Ok(
                            json!({"status": "timeout", "replies": replies.iter().map(|m| msg_json(m)).collect::<Vec<_>>(), "pending_cards": pending}),
                        );
                    }
                    last = (n > 0).then_some(n);
                    tokio::time::sleep(self.poll).await;
                }
            }
            "list_pending_cards" => {
                let b = self.bot(a).await?;
                self.api.transcript(&b.id, 50).await?;
                let cards: Vec<Value> = self
                    .api
                    .take_cards(&b.id)
                    .iter()
                    .filter(|c| c.state.is_pending())
                    .map(card_json)
                    .collect();
                Ok(json!({"bot": b.id, "cards": cards}))
            }
            "answer_card" => {
                self.write_guard(name)?;
                let b = self.bot(a).await?;
                let id = req(a, "card_id")?;
                self.api.transcript(&b.id, 50).await?;
                let c = self
                    .api
                    .take_cards(&b.id)
                    .into_iter()
                    .find(|c| c.entry_id == id)
                    .ok_or_else(|| {
                        ToolError(format!(
                            "no card `{id}` in the recent chat (see list_pending_cards)"
                        ))
                    })?;
                *detail = format!(
                    "kind={} action={}",
                    kind_of(&c),
                    arg(a, "action").unwrap_or("answer")
                );
                if !c.state.is_pending() {
                    return Err(ToolError(format!(
                        "card `{id}` is already {}",
                        c.state.label()
                    )));
                }
                let call = card_call_for(&c, a)?;
                // Secrets: report only that it was provided.
                let done = if matches!(c.body, Body::Secret { .. }) && call.method == "submitSecret"
                {
                    "provided".to_string()
                } else {
                    call.done.clone()
                };
                let method = call.method;
                self.api
                    .card_call(&b.id, b.viewer_session_id.as_deref(), call)
                    .await?;
                Ok(
                    json!({"card_id": id, "type": kind_of(&c), "method": method, "result": if done.is_empty() { "dismissed".into() } else { done }}),
                )
            }
            "list_attachments" => {
                let b = self.bot(a).await?;
                let msgs = self
                    .api
                    .transcript(&b.id, num(a, "limit", 50, 200) as u32)
                    .await?;
                let v: Vec<Value> = msgs
                    .iter()
                    .flat_map(|m| {
                        m.attachments.iter().map(move |at| {
                            let mut j = att_json(at);
                            j["message_id"] = json!(m.remote_id);
                            j["role"] = json!(if m.role == Role::User { "user" } else { "bot" });
                            j
                        })
                    })
                    .collect();
                Ok(json!({"bot": b.id, "attachments": v}))
            }
            "download_attachment" => {
                let b = self.bot(a).await?;
                let path = req(a, "path")?.to_string();
                let reads = self
                    .api
                    .resolve_attachments(&b.id, std::slice::from_ref(&path))
                    .await?;
                let r = reads
                    .into_iter()
                    .find(|r| !r.url.is_empty())
                    .ok_or_else(|| {
                        ToolError(format!(
                            "`{path}` has no downloadable copy (it may live only on the bot's box)"
                        ))
                    })?;
                let dir = arg(a, "dest_dir")
                    .map(crate::download::expand)
                    .unwrap_or_else(|| self.download_dir.clone());
                tokio::fs::create_dir_all(&dir)
                    .await
                    .map_err(|e| format!("{}: {e}", dir.display()))?;
                let dest = free_path(&dir, &safe_name(&path));
                let n = crate::download::fetch(r.url, dest.clone(), false, self.download_cap)
                    .await
                    .map_err(ToolError)?;
                Ok(
                    json!({"saved_to": dest.display().to_string(), "bytes": n, "content_type": r.content_type}),
                )
            }
            other => Err(ToolError(format!("unknown tool `{other}`"))),
        }
    }

    /// `grokbot://bots` (JSON).
    pub async fn resource_bots(&self) -> Result<String, ToolError> {
        let v = self.call("list_bots", &json!({})).await?;
        Ok(serde_json::to_string_pretty(&v["bots"]).unwrap_or_default())
    }

    /// `grokbot://bots/{id}/transcript` (plain text, last 100 messages).
    pub async fn resource_transcript(&self, id: &str) -> Result<String, ToolError> {
        let b = self.bot(&json!({"bot": id})).await?;
        let msgs = self.api.transcript(&b.id, 100).await?;
        self.audit
            .record("resource:transcript", Some(&b.id), "", "ok");
        Ok(msgs
            .iter()
            .map(|m| {
                format!(
                    "{}: {}",
                    if m.role == Role::User {
                        "user"
                    } else {
                        b.name.as_str()
                    },
                    m.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    /// Cached allowed bots for the resource list (refreshing once if empty).
    pub async fn allowed_bots(&self) -> Vec<Bot> {
        let cached = self.bots.lock().unwrap().clone();
        let bots = if cached.is_empty() {
            self.refresh_bots().await.unwrap_or_default()
        } else {
            cached
        };
        bots.into_iter()
            .filter(|b| self.cfg.allows(&b.id, &b.name) && !b.hidden)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::MockClient;

    fn ctx_with(cfg: ServerConfig) -> (Ctx, Arc<MockClient>) {
        let mut m = MockClient::new("mock");
        m.reply_delay = Duration::from_millis(10);
        let m = Arc::new(m);
        let mut c = Ctx::new(m.clone(), cfg, Audit::off(), std::env::temp_dir());
        c.poll = Duration::from_millis(20);
        (c, m)
    }
    fn ctx() -> (Ctx, Arc<MockClient>) {
        ctx_with(ServerConfig::default())
    }

    #[test]
    fn tool_list_is_complete_and_schemas_are_objects() {
        let names: Vec<_> = tool_defs().iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "list_bots",
                "get_bot",
                "send_message",
                "read_messages",
                "wait_for_reply",
                "list_pending_cards",
                "answer_card",
                "list_attachments",
                "download_attachment"
            ]
        );
        for t in tool_defs() {
            assert_eq!(t.schema["type"], "object", "{}", t.name);
        }
    }

    #[tokio::test]
    async fn list_and_get_bots() {
        let (c, _) = ctx();
        let v = c.call("list_bots", &json!({})).await.unwrap();
        let names: Vec<_> = v["bots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"Helper") && !names.contains(&"Secret"));
        let v = c
            .call("list_bots", &json!({"include_hidden": true}))
            .await
            .unwrap();
        assert!(
            v["bots"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["name"] == "Secret")
        );
        let b = c.call("get_bot", &json!({"bot": "coder"})).await.unwrap();
        assert_eq!(b["name"], "Coder");
        let b = c.call("get_bot", &json!({"bot": "SCRIBE"})).await.unwrap();
        assert_eq!(b["id"], "scribe");
        assert!(c.call("get_bot", &json!({"bot": "nobody"})).await.is_err());
        assert!(
            c.call("get_bot", &json!({}))
                .await
                .unwrap_err()
                .0
                .contains("`bot`")
        );
        assert!(
            c.call("nope", &json!({}))
                .await
                .unwrap_err()
                .0
                .contains("unknown tool")
        );
    }

    #[tokio::test]
    async fn allowlist_limits_everything() {
        let (c, _) = ctx_with(ServerConfig {
            allowed_bots: vec!["Helper".into()],
            ..Default::default()
        });
        let v = c.call("list_bots", &json!({})).await.unwrap();
        assert_eq!(v["bots"].as_array().unwrap().len(), 1);
        assert!(
            c.call("send_message", &json!({"bot": "coder", "text": "hi"}))
                .await
                .unwrap_err()
                .0
                .contains("allowed_bots")
        );
        assert!(
            c.call("read_messages", &json!({"bot": "helper"}))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn read_only_refuses_writes() {
        let (c, _) = ctx_with(ServerConfig {
            read_only: true,
            ..Default::default()
        });
        for (t, a) in [
            ("send_message", json!({"bot": "helper", "text": "hi"})),
            ("answer_card", json!({"bot": "helper", "card_id": "x"})),
        ] {
            assert!(
                c.call(t, &a).await.unwrap_err().0.contains("read-only"),
                "{t}"
            );
        }
        assert!(c.call("list_bots", &json!({})).await.is_ok());
    }

    #[tokio::test]
    async fn send_wait_and_read_round_trip() {
        let (c, _) = ctx();
        let s = c
            .call(
                "send_message",
                &json!({"bot": "helper", "text": "hello there"}),
            )
            .await
            .unwrap();
        let id = s["message_id"].as_str().unwrap().to_string();
        let r = c
            .call(
                "wait_for_reply",
                &json!({"bot": "helper", "message_id": id, "timeout_secs": 5}),
            )
            .await
            .unwrap();
        assert_eq!(r["status"], "replied", "{r}");
        assert!(!r["replies"].as_array().unwrap().is_empty());
        let m = c
            .call("read_messages", &json!({"bot": "helper", "limit": 10}))
            .await
            .unwrap();
        let msgs = m["messages"].as_array().unwrap();
        assert!(
            msgs.iter()
                .any(|m| m["role"] == "user" && m["text"] == "hello there"),
            "{m}"
        );
        assert!(msgs.iter().any(|m| m["role"] == "bot"));
        let first = msgs[0]["id"].as_str().unwrap();
        let after = c
            .call("read_messages", &json!({"bot": "helper", "since": first}))
            .await
            .unwrap();
        assert_eq!(after["messages"].as_array().unwrap().len(), msgs.len() - 1);
    }

    async fn make_card(c: &Ctx, kind: &str) -> Value {
        c.call(
            "send_message",
            &json!({"bot": "helper", "text": format!("card {kind}")}),
        )
        .await
        .unwrap();
        for _ in 0..100 {
            let v = c
                .call("list_pending_cards", &json!({"bot": "helper"}))
                .await
                .unwrap();
            if let Some(card) = v["cards"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["state"] == "pending")
                .cloned()
            {
                return card;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no {kind} card");
    }

    #[tokio::test]
    async fn answers_every_card_kind() {
        for (kind, args, method) in [
            ("widget", json!({"value": "Green"}), "respondToWidget"),
            (
                "approval",
                json!({"action": "approve"}),
                "resolveAutoReviewApproval",
            ),
            (
                "tool",
                json!({"action": "deny"}),
                "resolveLocalToolPermission",
            ),
            ("secret", json!({"value": "s3cr3t-value"}), "submitSecret"),
            (
                "form",
                json!({"values": {"user": "me", "pass": "pw-value"}}),
                "submitUserForm",
            ),
            ("draft", json!({"action": "send"}), "sendDraft"),
        ] {
            let (c, m) = ctx();
            let card = make_card(&c, kind).await;
            let mut a = args.clone();
            a["bot"] = json!("helper");
            a["card_id"] = card["card_id"].clone();
            let r = c
                .call("answer_card", &a)
                .await
                .unwrap_or_else(|e| panic!("{kind}: {e:?}"));
            assert_eq!(r["method"], method, "{kind}");
            let out = r.to_string();
            assert!(
                !out.contains("s3cr3t") && !out.contains("pw-value"),
                "{kind}: {out}"
            );
            assert_eq!(m.card_calls.lock().unwrap().last().unwrap().0, method);
            if kind == "secret" {
                assert_eq!(r["result"], "provided");
                assert!(!card.to_string().contains("s3cr3t"));
            }
            // Answered: no longer pending, and a second answer is refused.
            let again = c.call("answer_card", &a).await;
            assert!(again.is_err(), "{kind}");
        }
    }

    #[tokio::test]
    async fn card_argument_errors() {
        let (c, _) = ctx();
        let card = make_card(&c, "form").await;
        let id = card["card_id"].clone();
        let e = c
            .call(
                "answer_card",
                &json!({"bot": "helper", "card_id": id, "values": {"user": "x"}}),
            )
            .await
            .unwrap_err();
        assert!(e.0.contains("`pass` is required"), "{e:?}");
        let e = c
            .call(
                "answer_card",
                &json!({"bot": "helper", "card_id": id, "action": "approve"}),
            )
            .await
            .unwrap_err();
        assert!(e.0.contains("doesn't apply"), "{e:?}");
        let r = c
            .call(
                "answer_card",
                &json!({"bot": "helper", "card_id": id, "action": "dismiss"}),
            )
            .await
            .unwrap();
        assert_eq!(r["method"], "dismissUserForm");
        let e = c
            .call(
                "answer_card",
                &json!({"bot": "helper", "card_id": "missing"}),
            )
            .await
            .unwrap_err();
        assert!(e.0.contains("no card"));
        let (c, _) = ctx();
        let card = make_card(&c, "purchase").await;
        let e = c
            .call(
                "answer_card",
                &json!({"bot": "helper", "card_id": card["card_id"], "action": "approve"}),
            )
            .await
            .unwrap_err();
        assert!(e.0.contains("Grok Bot app"), "{e:?}");
    }

    #[test]
    fn download_names_are_safe() {
        assert_eq!(safe_name("/a/b/../../etc/passwd"), "passwd");
        assert_eq!(safe_name("..\\x\\..hidden"), "hidden");
        assert_eq!(safe_name(""), "attachment");
        assert_eq!(safe_name("a:b?.txt"), "a_b_.txt");
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("f.txt"), "x").unwrap();
        assert_eq!(free_path(d.path(), "f.txt"), d.path().join("f (2).txt"));
        assert_eq!(free_path(d.path(), "g"), d.path().join("g"));
    }

    #[tokio::test]
    async fn resources_render() {
        let (c, _) = ctx();
        assert!(c.resource_bots().await.unwrap().contains("Helper"));
        c.call("send_message", &json!({"bot": "helper", "text": "hi"}))
            .await
            .unwrap();
        assert!(
            c.resource_transcript("helper")
                .await
                .unwrap()
                .contains("user: hi")
        );
        assert!(!c.allowed_bots().await.iter().any(|b| b.hidden));
    }
}
