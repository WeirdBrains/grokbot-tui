//! Interactive Grok Bot cards in a chat (1.29.0): question widgets, approval cards,
//! secret requests, forms, message drafts, and side conversations (1.41.4). Parsed from the transcript entry
//! (`kind: "send-message"`, `message.type`), shown in the bubble with their state, and
//! answered from a modal with the same gateway calls the desktop app makes:
//! `respondToWidget`, `dismissWidget`, `resolveAutoReviewApproval`,
//! `resolveLocalToolPermission`, `submitSecret`, `submitUserForm`, `dismissUserForm`,
//! `sendDraft` and `discardDraft`. Card types the TUI can't act on are shown read-only.
//!
//! Secrets typed here (secret requests and secret form fields) live only in the modal,
//! are masked on screen, go out only in the gateway call's body, and are cleared after
//! sending or closing. They are never logged, echoed, stored or put in a notice.
use serde_json::{Value, json};

/// Prefix of a card's [`crate::ui::LinkHit`] url: a click opens its modal.
pub const SCHEME: &str = "gbcard:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opt {
    pub label: String,
    /// What goes back (the option's `value`, else its label).
    pub value: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub prompt: String,
    pub options: Vec<Opt>,
    pub multi: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub secret: bool,
    pub placeholder: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// A question: one or more steps (one unless the card has `steps`).
    Widget {
        help: String,
        steps: Vec<Step>,
        allow_custom: bool,
    },
    /// An approval for a blocked action. `local` = local-tool-permission.
    Approval {
        local: bool,
        request_id: String,
        summary: String,
        command: String,
        reason: String,
    },
    Secret {
        label: String,
        description: String,
    },
    Form {
        title: String,
        fields: Vec<Field>,
    },
    /// email-draft / slack-draft / meeting-chat-draft.
    Draft {
        kind: String,
        from: String,
        to: Vec<String>,
        subject: String,
        body: String,
        /// The draft object as received, sent back by `sendDraft`.
        raw: Value,
    },
    /// A card the TUI shows but can't act on (purchase approvals, connector grants, ...).
    Info {
        kind: String,
        title: String,
        lines: Vec<String>,
    },
    /// A side conversation: this bot talking to another Grok Bot (`agent`) or to a
    /// Cursor cloud agent (`cloud-agent`). The transcript carries each turn as a
    /// `kind: "message"` entry with `toAgent` / `fromAgent`; Cursor launches also
    /// post a `message.type == "cursor-agent"` card (`bcId` + `title`).
    SideChat {
        /// `"agent"` or `"cloud-agent"`.
        peer_kind: String,
        peer_name: String,
        peer_id: String,
        /// Cursor-agent card title, when one was posted.
        title: String,
        messages: Vec<SideMsg>,
    },
}

/// One turn in a [`Body::SideChat`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideMsg {
    /// `true` when this bot sent it (`toAgent`); `false` when the peer sent it (`fromAgent`).
    pub outgoing: bool,
    pub label: String,
    pub text: String,
    pub entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Pending,
    /// Answered; the text says how ("approved", the chosen option, "submitted").
    Done(String),
    Dismissed,
    Expired,
    /// Sent but not confirmed yet (drafts), or our call is in flight.
    Sending,
}

impl State {
    pub fn label(&self) -> String {
        match self {
            Self::Pending => "pending".into(),
            Self::Done(s) if s.is_empty() => "answered".into(),
            Self::Done(s) => format!("answered · {s}"),
            Self::Dismissed => "dismissed".into(),
            Self::Expired => "expired".into(),
            Self::Sending => "sending…".into(),
        }
    }
    pub fn is_pending(&self) -> bool {
        *self == Self::Pending
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// Transcript entry id (`entryId` in every call).
    pub entry_id: String,
    /// `message.type`.
    pub kind: String,
    pub body: Body,
    pub state: State,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .map(grokbot_api::sanitize_text)
        .unwrap_or_default()
}

fn one_line(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The desktop app's option value: `value ?? label`, newlines collapsed, trimmed.
fn opt_value(o: &Value) -> String {
    let v = o
        .get("value")
        .and_then(Value::as_str)
        .or_else(|| o.get("label").and_then(Value::as_str))
        .unwrap_or("");
    one_line(v)
}

fn options(v: &Value) -> Vec<Opt> {
    v.get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|o| match o {
                    Value::String(t) => Opt {
                        label: t.clone(),
                        value: one_line(t),
                        description: String::new(),
                    },
                    _ => Opt {
                        label: {
                            let l = s(o, "label");
                            if l.is_empty() { s(o, "value") } else { l }
                        },
                        value: opt_value(o),
                        description: s(o, "description"),
                    },
                })
                .filter(|o| !o.value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn status_state(st: &str) -> State {
    match st {
        "" | "pending" => State::Pending,
        "expired" | "timed-out" | "timeout" => State::Expired,
        "dismissed" | "cancelled" | "canceled" => State::Dismissed,
        "deny" | "denied" | "rejected" => State::Done("denied".into()),
        "allow-once" | "allowed" => State::Done("allowed once".into()),
        other => State::Done(other.into()),
    }
}

/// Parse a raw transcript entry. `None` for plain text and non-card entries.
pub fn parse(e: &Value) -> Option<Card> {
    if e.get("kind").and_then(Value::as_str) != Some("send-message") {
        return None;
    }
    let msg = e.get("message")?;
    let kind = msg.get("type").and_then(Value::as_str)?.to_string();
    let entry_id = e.get("id").and_then(Value::as_str)?.to_string();
    // `widgetSkipped`: the bot moved on without an answer (the desktop app's "skipped").
    let dismissed = ["widgetDismissed", "widgetSkipped"]
        .iter()
        .any(|k| e.get(*k).and_then(Value::as_bool) == Some(true));
    let (body, state) = match kind.as_str() {
        "text" | "attachment" => return None,
        "widget" => {
            let w = msg.get("widget")?;
            let prompt = s(w, "prompt");
            if prompt.is_empty() {
                return None;
            }
            let multi = w.get("multiSelect").and_then(Value::as_bool) == Some(true);
            let mut steps = vec![];
            if let Some(a) = w.get("steps").and_then(Value::as_array)
                && a.len() >= 2
            {
                for st in a {
                    let p = s(st, "prompt");
                    let o = options(st);
                    if p.is_empty() || o.is_empty() {
                        steps.clear();
                        break;
                    }
                    steps.push(Step {
                        prompt: p,
                        options: o,
                        multi: st.get("multiSelect").and_then(Value::as_bool) == Some(true),
                    });
                }
            }
            if steps.is_empty() {
                steps.push(Step {
                    prompt,
                    options: options(w),
                    multi,
                });
            }
            let state = match e.get("respondedValue").and_then(Value::as_str) {
                Some(v) => State::Done(one_line(v)),
                None if dismissed => State::Dismissed,
                None => State::Pending,
            };
            (
                Body::Widget {
                    help: s(w, "helpText"),
                    allow_custom: w.get("allowCustom").and_then(Value::as_bool) == Some(true),
                    steps,
                },
                state,
            )
        }
        "auto-review-approval" | "local-tool-permission" => {
            let local = kind == "local-tool-permission";
            let c = msg.get(if local { "ask" } else { "approval" })?;
            let summary = ["summary", "title", "description", "toolName", "tool"]
                .iter()
                .map(|k| s(c, k))
                .find(|t| !t.is_empty())
                .unwrap_or_default();
            let command = ["command", "input", "args"]
                .iter()
                .map(|k| match c.get(*k) {
                    Some(Value::String(t)) => grokbot_api::sanitize_text(t),
                    Some(v @ Value::Object(_)) => v.to_string(),
                    _ => String::new(),
                })
                .find(|t| !t.is_empty())
                .unwrap_or_default();
            (
                Body::Approval {
                    local,
                    request_id: s(c, "requestId"),
                    summary,
                    command,
                    reason: s(c, "reason"),
                },
                status_state(&s(c, "status")),
            )
        }
        "secret-request" | "app-secrets-request" => {
            let r = msg
                .get("secretRequest")
                .or_else(|| msg.get("request"))
                .unwrap_or(&Value::Null);
            let provided = e.get("secretProvided").and_then(Value::as_bool) == Some(true);
            let body = Body::Secret {
                label: s(r, "label"),
                description: s(r, "description"),
            };
            if kind == "app-secrets-request" {
                // Several secrets at once: the desktop app's flow; show it read-only.
                return Some(Card {
                    entry_id,
                    kind: kind.clone(),
                    body: info(&kind, msg),
                    state: if provided {
                        State::Done("provided".into())
                    } else {
                        State::Pending
                    },
                });
            }
            (
                body,
                if provided {
                    State::Done("provided".into())
                } else if dismissed {
                    State::Dismissed
                } else {
                    State::Pending
                },
            )
        }
        "user-form" => {
            let f = msg.get("formRequest")?;
            let fields = f
                .get("fields")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|x| {
                            let kind = s(x, "type");
                            Field {
                                id: s(x, "id"),
                                label: {
                                    let l = s(x, "label");
                                    if l.is_empty() { s(x, "id") } else { l }
                                },
                                secret: x.get("secret").and_then(Value::as_bool) == Some(true)
                                    || x.get("isSecret").and_then(Value::as_bool) == Some(true)
                                    || matches!(kind.as_str(), "password" | "otp"),
                                required: x.get("required").and_then(Value::as_bool) == Some(true),
                                placeholder: s(x, "placeholder"),
                                kind,
                            }
                        })
                        .filter(|x| !x.id.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let state = match e.get("formResolution").and_then(Value::as_str) {
                Some(r) => status_state(r),
                None if dismissed => State::Dismissed,
                None => State::Pending,
            };
            (
                Body::Form {
                    title: s(f, "title"),
                    fields,
                },
                state,
            )
        }
        "email-draft" | "slack-draft" | "meeting-chat-draft" => {
            let d = msg.get("draft")?;
            let to = match d.get("to") {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .map(grokbot_api::sanitize_text)
                    .collect(),
                Some(Value::String(t)) => vec![grokbot_api::sanitize_text(t)],
                _ => {
                    let ch = ["channel", "channelName", "meetingTitle", "workspace"]
                        .iter()
                        .map(|k| s(d, k))
                        .find(|t| !t.is_empty());
                    ch.into_iter().collect()
                }
            };
            let st = e
                .get("draftSendState")
                .or_else(|| e.get("draftSaveState"))
                .and_then(Value::as_str)
                .unwrap_or("editable");
            let state = match st {
                "editable" | "saved" | "" => State::Pending,
                "unconfirmed" | "sending" => State::Sending,
                "sent" => State::Done("sent".into()),
                "discarded" => State::Dismissed,
                "expired" => State::Expired,
                other => State::Done(other.into()),
            };
            (
                Body::Draft {
                    kind: kind.clone(),
                    from: s(d, "from"),
                    to,
                    subject: s(d, "subject"),
                    body: s(d, "body"),
                    raw: d.clone(),
                },
                state,
            )
        }
        "cursor-agent" => {
            let bc = s(msg, "bcId");
            let title = s(msg, "title");
            if bc.is_empty() && title.is_empty() {
                return None;
            }
            (
                Body::SideChat {
                    peer_kind: "cloud-agent".into(),
                    peer_name: title.clone(),
                    peer_id: bc,
                    title,
                    messages: vec![],
                },
                status_state(&s(msg, "status")),
            )
        }
        _ => {
            let sub = msg
                .as_object()
                .and_then(|o| o.iter().find(|(k, v)| *k != "type" && v.is_object()))
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Null);
            let st = s(&sub, "status");
            (info(&kind, msg), status_state(&st))
        }
    };
    Some(Card {
        entry_id,
        kind,
        body,
        state,
    })
}

/// A read-only card: its title and a few of its short text fields.
fn info(kind: &str, msg: &Value) -> Body {
    let mut lines = vec![];
    let mut title = String::new();
    let mut walk = |v: &Value| {
        if let Some(o) = v.as_object() {
            for (k, x) in o {
                if let Some(t) = x.as_str() {
                    let t = grokbot_api::sanitize_text(t);
                    let skip = k == "type"
                        || k.to_lowercase().contains("id")
                        || k.to_lowercase().contains("token")
                        || k.to_lowercase().contains("secret")
                        || t.is_empty()
                        || t.len() > 200;
                    if skip {
                        continue;
                    }
                    if title.is_empty() && matches!(k.as_str(), "title" | "summary" | "name") {
                        title = t;
                    } else if lines.len() < 4 {
                        lines.push(format!("{k}: {}", one_line(&t)));
                    }
                }
            }
        }
    };
    walk(msg);
    if let Some(o) = msg.as_object() {
        for (_, v) in o.iter().filter(|(_, v)| v.is_object()) {
            walk(v);
        }
    }
    Body::Info {
        kind: kind.into(),
        title,
        lines,
    }
}

impl Card {
    /// The bubble's stored text: stable (no state), never a secret value.
    pub fn summary(&self) -> String {
        match &self.body {
            Body::Widget { steps, .. } => format!("❓ {}", steps[0].prompt),
            Body::Approval { summary, local, .. } => {
                let what = if *local {
                    "Tool permission"
                } else {
                    "Approval needed"
                };
                if summary.is_empty() {
                    format!("🛡 {what}")
                } else {
                    format!("🛡 {what}: {summary}")
                }
            }
            Body::Secret { label, .. } => format!("🔑 Secret requested: {label}"),
            Body::Form { title, .. } => format!("📝 Form: {title}"),
            Body::Draft {
                kind, to, subject, ..
            } => {
                let what = match kind.as_str() {
                    "email-draft" => "Email draft",
                    "slack-draft" => "Slack draft",
                    _ => "Message draft",
                };
                let to = to.join(", ");
                if subject.is_empty() {
                    format!("✉ {what} to {to}")
                } else {
                    format!("✉ {what} to {to}: {subject}")
                }
            }
            Body::Info { kind, title, .. } => {
                if title.is_empty() {
                    format!("▣ {kind}")
                } else {
                    format!("▣ {kind}: {title}")
                }
            }
            Body::SideChat {
                peer_kind,
                peer_name,
                title,
                ..
            } => side_summary(peer_kind, peer_name, title),
        }
    }

    /// Whether the TUI can act on this card.
    pub fn actionable(&self) -> bool {
        !matches!(self.body, Body::Info { .. } | Body::SideChat { .. })
    }

    /// Lines shown in the bubble under the summary (no secret values).
    pub fn bubble_lines(&self) -> Vec<String> {
        let mut out = vec![];
        match &self.body {
            Body::Widget {
                steps,
                allow_custom,
                ..
            } => {
                let st = &steps[0];
                for (i, o) in st.options.iter().enumerate().take(9) {
                    out.push(format!("  {}. {}", i + 1, o.label));
                }
                if st.options.len() > 9 {
                    out.push(format!("  … {} more", st.options.len() - 9));
                }
                if *allow_custom {
                    out.push("  ✎ or type your own".into());
                }
                if steps.len() > 1 {
                    out.push(format!("  ({} steps)", steps.len()));
                }
            }
            Body::Approval {
                command, reason, ..
            } => {
                if !command.is_empty() {
                    out.push(format!("  $ {}", one_line(command)));
                }
                if !reason.is_empty() {
                    out.push(format!("  why: {reason}"));
                }
            }
            Body::Secret { description, .. } => {
                if !description.is_empty() {
                    out.push(format!("  {description}"));
                }
            }
            Body::Form { fields, .. } => {
                let names: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
                out.push(format!("  fields: {}", names.join(", ")));
            }
            Body::Draft { from, body, .. } => {
                if !from.is_empty() {
                    out.push(format!("  from: {from}"));
                }
                for l in body.lines().take(3) {
                    out.push(format!("  │ {l}"));
                }
                if body.lines().count() > 3 {
                    out.push("  │ …".into());
                }
            }
            Body::Info { lines, .. } => {
                out.extend(lines.iter().map(|l| format!("  {l}")));
            }
            Body::SideChat {
                peer_kind,
                peer_name,
                messages,
                ..
            } => {
                let who = if peer_kind == "cloud-agent" {
                    if peer_name.is_empty() {
                        "a Cursor agent".into()
                    } else {
                        format!("Cursor agent “{peer_name}”")
                    }
                } else if peer_name.is_empty() {
                    "another agent".into()
                } else {
                    peer_name.clone()
                };
                let n = messages.len();
                out.push(format!(
                    "  {} with {who}",
                    if n == 1 {
                        "1 message".into()
                    } else {
                        format!("{n} messages")
                    }
                ));
            }
        }
        out
    }

    /// The bubble's action button label.
    pub fn button(&self) -> &'static str {
        match (&self.state, self.actionable()) {
            (State::Pending, true) => match self.body {
                Body::Widget { .. } => "[answer]",
                Body::Approval { .. } => "[review]",
                Body::Secret { .. } => "[enter secret]",
                Body::Form { .. } => "[fill in]",
                Body::Draft { .. } => "[review draft]",
                Body::Info { .. } => "[view]",
                Body::SideChat { .. } => "[view conversation]",
            },
            (State::Pending, false) if matches!(self.body, Body::SideChat { .. }) => {
                "[view conversation]"
            }
            (State::Pending, false) => "[view · act in the desktop app]",
            (_, _) if matches!(self.body, Body::SideChat { .. }) => "[view conversation]",
            _ => "[view]",
        }
    }
}

/// The value `respondToWidget` sends, as the desktop app builds it:
/// one step → the chosen values (and custom text) joined by newlines;
/// several steps → each step's prompt followed by its values, steps separated by a blank line.
pub fn widget_value(steps: &[Step], picks: &[Vec<usize>], custom: &str) -> String {
    let clean = |v: &str| one_line(v);
    if steps.len() >= 2 {
        return steps
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let mut v = vec![clean(&st.prompt)];
                let sel = picks.get(i).cloned().unwrap_or_default();
                v.extend(
                    st.options
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| sel.contains(j))
                        .map(|(_, o)| o.value.clone()),
                );
                v.join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    let st = &steps[0];
    let sel = picks.first().cloned().unwrap_or_default();
    let mut v: Vec<String> = st
        .options
        .iter()
        .enumerate()
        .filter(|(j, _)| sel.contains(j))
        .map(|(_, o)| o.value.clone())
        .collect();
    let c = clean(custom);
    if !c.is_empty() {
        v.push(c);
    }
    v.join("\n")
}

/// One gateway call for a card action: the method and its body (without
/// `agentId`/`sessionId`, which the client adds).
#[derive(Clone, PartialEq, Eq)]
pub struct Call {
    pub method: &'static str,
    pub body: Value,
    /// What the bubble says once it worked ("approved", the answer, "sent", ...).
    pub done: String,
}

impl std::fmt::Debug for Call {
    // Never print bodies: they may carry a secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Call({})", self.method)
    }
}

pub fn answer_widget(c: &Card, value: String) -> Call {
    Call {
        method: "respondToWidget",
        body: json!({ "entryId": c.entry_id, "value": value }),
        done: one_line(&value),
    }
}

pub fn dismiss(c: &Card) -> Option<Call> {
    let id = &c.entry_id;
    Some(match &c.body {
        Body::Widget { .. } | Body::Secret { .. } => Call {
            method: "dismissWidget",
            body: json!({ "entryId": id }),
            done: String::new(),
        },
        Body::Form { .. } => Call {
            method: "dismissUserForm",
            body: json!({ "entryId": id, "mode": "dismissed" }),
            done: String::new(),
        },
        Body::Draft { .. } => Call {
            method: "discardDraft",
            body: json!({ "entryId": id }),
            done: String::new(),
        },
        _ => return None,
    })
}

pub fn approve(c: &Card, yes: bool) -> Option<Call> {
    let Body::Approval {
        local, request_id, ..
    } = &c.body
    else {
        return None;
    };
    let (method, res) = if *local {
        (
            "resolveLocalToolPermission",
            if yes { "allow-once" } else { "deny" },
        )
    } else {
        (
            "resolveAutoReviewApproval",
            if yes { "approved" } else { "denied" },
        )
    };
    Some(Call {
        method,
        body: json!({ "entryId": c.entry_id, "requestId": request_id, "resolution": res }),
        done: if yes { "approved" } else { "denied" }.into(),
    })
}

/// `submitSecret`. The value is moved into the body; the caller clears its copy.
pub fn submit_secret(c: &Card, value: String) -> Call {
    Call {
        method: "submitSecret",
        body: json!({ "entryId": c.entry_id, "value": value }),
        done: "provided".into(),
    }
}

pub fn submit_form(c: &Card, values: Vec<(String, String)>) -> Call {
    let m: serde_json::Map<String, Value> = values
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    Call {
        method: "submitUserForm",
        body: json!({ "entryId": c.entry_id, "values": m }),
        done: "submitted".into(),
    }
}

/// `sendDraft`, with the (possibly edited) body.
pub fn send_draft(c: &Card, body: Option<&str>) -> Option<Call> {
    let Body::Draft { raw, .. } = &c.body else {
        return None;
    };
    let mut d = raw.clone();
    if let (Some(b), Some(o)) = (body, d.as_object_mut()) {
        o.insert("body".into(), Value::String(b.to_string()));
    }
    Some(Call {
        method: "sendDraft",
        body: json!({ "entryId": c.entry_id, "draft": d }),
        done: "sent".into(),
    })
}

fn side_summary(peer_kind: &str, peer_name: &str, title: &str) -> String {
    if peer_kind == "cloud-agent" {
        let t = if title.is_empty() { peer_name } else { title };
        if t.is_empty() {
            "▣ cursor-agent".into()
        } else {
            format!("▣ cursor-agent: {t}")
        }
    } else if peer_name.is_empty() {
        "↔ agent chat".into()
    } else {
        format!("↔ agent: {peer_name}")
    }
}

/// A `kind: "message"` turn between this bot and another agent (`toAgent` / `fromAgent`).
pub fn is_side_message(raw: &Value) -> bool {
    raw.get("kind").and_then(Value::as_str) == Some("message")
        && (raw.get("toAgent").is_some_and(|v| v.is_object())
            || raw.get("fromAgent").is_some_and(|v| v.is_object()))
}

fn peer_obj(raw: &Value) -> Option<(&Value, bool)> {
    if let Some(v) = raw.get("toAgent").filter(|v| v.is_object()) {
        return Some((v, true));
    }
    raw.get("fromAgent")
        .filter(|v| v.is_object())
        .map(|v| (v, false))
}

/// Attach `toAgent` / `fromAgent` turns to cursor-agent cards, and mint a card for
/// agent-to-agent threads that have no `cursor-agent` send-message.
pub fn enrich_cards(mut cards: Vec<Card>, raws: &[Value]) -> Vec<Card> {
    #[derive(Default)]
    struct Acc {
        peer_kind: String,
        peer_name: String,
        title: String,
        messages: Vec<SideMsg>,
    }
    let mut by_id: std::collections::BTreeMap<String, Acc> = std::collections::BTreeMap::new();
    for raw in raws {
        if let Some(c) = parse(raw)
            && let Body::SideChat {
                peer_id,
                peer_kind,
                peer_name,
                title,
                ..
            } = &c.body
            && !peer_id.is_empty()
        {
            let acc = by_id.entry(peer_id.clone()).or_default();
            if acc.peer_kind.is_empty() {
                acc.peer_kind = peer_kind.clone();
            }
            if acc.title.is_empty() {
                acc.title = title.clone();
            }
            if acc.peer_name.is_empty() {
                acc.peer_name = peer_name.clone();
            }
        }
        let Some((peer, outgoing)) = peer_obj(raw) else {
            continue;
        };
        let id = s(peer, "id");
        if id.is_empty() {
            continue;
        }
        let acc = by_id.entry(id).or_default();
        let kind = s(peer, "kind");
        if acc.peer_kind.is_empty() {
            acc.peer_kind = if kind.is_empty() {
                "agent".into()
            } else {
                kind
            };
        }
        let name = s(peer, "name");
        if acc.peer_name.is_empty() {
            acc.peer_name = name.clone();
        }
        let text = grokbot_api::sanitize_text(
            raw.get("content")
                .and_then(Value::as_str)
                .or_else(|| raw.get("text").and_then(Value::as_str))
                .unwrap_or(""),
        );
        if text.trim().is_empty() {
            continue;
        }
        let label = if outgoing {
            if acc.peer_name.is_empty() {
                "this bot".into()
            } else {
                format!("this bot → {}", acc.peer_name)
            }
        } else if name.is_empty() {
            acc.peer_name.clone()
        } else {
            name
        };
        acc.messages.push(SideMsg {
            outgoing,
            label,
            text,
            entry_id: raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    for c in &mut cards {
        if let Body::SideChat {
            peer_id,
            peer_kind,
            peer_name,
            title,
            messages,
        } = &mut c.body
            && let Some(acc) = by_id.remove(peer_id)
        {
            if peer_kind.is_empty() {
                *peer_kind = acc.peer_kind;
            }
            if peer_name.is_empty() {
                *peer_name = acc.peer_name;
            }
            if title.is_empty() {
                *title = acc.title;
            }
            *messages = acc.messages;
        }
    }
    for (peer_id, acc) in by_id {
        if acc.messages.is_empty() {
            continue;
        }
        let entry_id = acc
            .messages
            .iter()
            .find(|m| !m.entry_id.is_empty())
            .map(|m| m.entry_id.clone())
            .unwrap_or_else(|| peer_id.clone());
        let peer_kind = if acc.peer_kind.is_empty() {
            "agent".into()
        } else {
            acc.peer_kind
        };
        let n = acc.messages.len();
        cards.push(Card {
            entry_id,
            kind: if peer_kind == "cloud-agent" {
                "cursor-agent".into()
            } else {
                "agent-chat".into()
            },
            body: Body::SideChat {
                peer_kind,
                peer_name: acc.peer_name,
                peer_id,
                title: acc.title,
                messages: acc.messages,
            },
            state: State::Done(format!("{n} messages")),
        });
    }
    cards
}

/// Remote ids of side-conversation turns that should not stay in the main feed
/// (everything except the card / stub's own `entry_id`).
pub fn hidden_side_ids(cards: &[Card]) -> Vec<String> {
    let mut out = vec![];
    for c in cards {
        if let Body::SideChat { messages, .. } = &c.body {
            for m in messages {
                if !m.entry_id.is_empty() && m.entry_id != c.entry_id {
                    out.push(m.entry_id.clone());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(extra: Value) -> Value {
        let mut e = json!({"id": "w1", "kind": "send-message", "message": {"type": "widget",
            "widget": {"prompt": "Pick one", "options": [
                {"label": "Red", "value": "red\nish"}, {"label": "Blue"}, "Green"]}}});
        if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
            for (k, v) in x {
                o.insert(k.clone(), v.clone());
            }
        }
        e
    }

    #[test]
    fn parses_widget_states_and_values() {
        let c = parse(&widget(json!({}))).unwrap();
        assert_eq!(c.state, State::Pending);
        assert_eq!(c.summary(), "❓ Pick one");
        let Body::Widget { steps, .. } = &c.body else {
            panic!()
        };
        let vals: Vec<&str> = steps[0].options.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(vals, ["red ish", "Blue", "Green"]);
        assert_eq!(widget_value(steps, &[vec![1]], ""), "Blue");
        assert_eq!(
            widget_value(steps, &[vec![0, 2]], " my\nown "),
            "red ish\nGreen\nmy own"
        );
        let done = parse(&widget(json!({"respondedValue": "Blue"}))).unwrap();
        assert_eq!(done.state, State::Done("Blue".into()));
        let gone = parse(&widget(json!({"widgetDismissed": true}))).unwrap();
        assert_eq!(gone.state, State::Dismissed);
        assert!(
            parse(
                &json!({"id":"t","kind":"send-message","message":{"type":"text","content":"hi"}})
            )
            .is_none()
        );
    }

    #[test]
    fn multi_step_value_matches_the_desktop_app() {
        let e = json!({"id": "w2", "kind": "send-message", "message": {"type": "widget",
            "widget": {"prompt": "Setup", "steps": [
                {"prompt": "Size?", "options": ["S", "M"]},
                {"prompt": "Color  ?", "options": [{"label": "R"}, {"label": "G"}], "multiSelect": true}]}}});
        let c = parse(&e).unwrap();
        let Body::Widget { steps, .. } = &c.body else {
            panic!()
        };
        assert_eq!(steps.len(), 2);
        assert_eq!(
            widget_value(steps, &[vec![1], vec![0, 1]], ""),
            "Size?\nM\n\nColor ?\nR\nG"
        );
    }

    #[test]
    fn approval_secret_form_and_draft_calls() {
        let a = parse(&json!({"id": "a1", "kind": "send-message", "message": {"type": "auto-review-approval",
            "approval": {"status": "pending", "requestId": "r1", "summary": "Run rm", "command": "rm x", "reason": "deletes"}}})).unwrap();
        assert!(a.state.is_pending());
        let call = approve(&a, true).unwrap();
        assert_eq!(call.method, "resolveAutoReviewApproval");
        assert_eq!(
            call.body,
            json!({"entryId": "a1", "requestId": "r1", "resolution": "approved"})
        );
        let l = parse(
            &json!({"id": "l1", "kind": "send-message", "message": {"type": "local-tool-permission",
            "ask": {"status": "pending", "requestId": "r2", "title": "Shell"}}}),
        )
        .unwrap();
        assert_eq!(approve(&l, false).unwrap().body["resolution"], "deny");
        let s = parse(
            &json!({"id": "s1", "kind": "send-message", "message": {"type": "secret-request",
            "secretRequest": {"label": "API key", "description": "for X"}}}),
        )
        .unwrap();
        assert_eq!(s.summary(), "🔑 Secret requested: API key");
        let call = submit_secret(&s, "hunter2".into());
        assert_eq!(
            format!("{call:?}"),
            "Call(submitSecret)",
            "Debug never shows the value"
        );
        assert!(!s.summary().contains("hunter2"));
        let f = parse(
            &json!({"id": "f1", "kind": "send-message", "message": {"type": "user-form",
            "formRequest": {"title": "Login", "fields": [
                {"id": "user", "label": "User", "type": "text", "required": true},
                {"id": "pw", "label": "Password", "type": "password"}]}}}),
        )
        .unwrap();
        let Body::Form { fields, .. } = &f.body else {
            panic!()
        };
        assert!(!fields[0].secret && fields[1].secret);
        let call = submit_form(&f, vec![("user".into(), "me".into())]);
        assert_eq!(call.body["values"], json!({"user": "me"}));
        assert_eq!(dismiss(&f).unwrap().body["mode"], "dismissed");
        let d = parse(&json!({"id": "d1", "kind": "send-message", "draftSendState": "editable",
            "message": {"type": "email-draft", "draft": {"from": "a@x", "to": ["b@y"], "subject": "Hi", "body": "Hello"}}})).unwrap();
        assert_eq!(d.summary(), "✉ Email draft to b@y: Hi");
        let call = send_draft(&d, Some("Hello!")).unwrap();
        assert_eq!(call.body["draft"]["body"], "Hello!");
        assert_eq!(call.body["draft"]["to"], json!(["b@y"]));
        assert_eq!(dismiss(&d).unwrap().method, "discardDraft");
        let sent = parse(
            &json!({"id": "d2", "kind": "send-message", "draftSendState": "sent",
            "message": {"type": "email-draft", "draft": {"to": [], "body": ""}}}),
        )
        .unwrap();
        assert_eq!(sent.state, State::Done("sent".into()));
    }

    #[test]
    fn unknown_cards_are_read_only_and_hide_ids() {
        let v = parse(&json!({"id": "v1", "kind": "send-message", "message": {"type": "virtual-card-approval",
            "request": {"status": "pending", "title": "Buy socks", "merchant": "Sockco", "requestId": "zz", "amount": "12.00"}}})).unwrap();
        assert!(!v.actionable());
        assert_eq!(v.summary(), "▣ virtual-card-approval: Buy socks");
        let l = v.bubble_lines().join("\n");
        assert!(l.contains("Sockco") && !l.contains("zz"));
        assert!(approve(&v, true).is_none() && dismiss(&v).is_none());
    }

    #[test]
    fn cursor_agent_card_and_side_messages_group_into_a_conversation() {
        let card = parse(&json!({"id": "c1", "kind": "send-message",
            "message": {"type": "cursor-agent", "bcId": "bc-abc", "title": "Patch bump"}}))
        .unwrap();
        assert!(!card.actionable());
        assert_eq!(card.summary(), "▣ cursor-agent: Patch bump");
        assert_eq!(card.button(), "[view conversation]");
        let raws = [
            json!({"id": "c1", "kind": "send-message",
                "message": {"type": "cursor-agent", "bcId": "bc-abc", "title": "Patch bump"}}),
            json!({"id": "m1", "kind": "message", "role": "assistant", "content": "Please patch it",
                "toAgent": {"id": "bc-abc", "kind": "cloud-agent", "name": "Patch bump"}}),
            json!({"id": "m2", "kind": "message", "role": "user", "content": "Done.\nCommits on main",
                "fromAgent": {"id": "bc-abc", "kind": "cloud-agent", "name": "Patch bump"}}),
        ];
        let cards = enrich_cards(vec![card], &raws);
        let Body::SideChat {
            peer_id,
            messages,
            title,
            ..
        } = &cards[0].body
        else {
            panic!()
        };
        assert_eq!(peer_id, "bc-abc");
        assert_eq!(title, "Patch bump");
        assert_eq!(messages.len(), 2);
        assert!(messages[0].outgoing && messages[0].text.contains("Please patch"));
        assert!(!messages[1].outgoing && messages[1].text.contains("Commits on main"));
        assert_eq!(
            hidden_side_ids(&cards),
            vec!["m1".to_string(), "m2".to_string()]
        );
    }

    #[test]
    fn agent_to_agent_messages_mint_a_clickable_card() {
        let raws = [
            json!({"id": "a1", "kind": "message", "content": "Can you review this?",
                "toAgent": {"id": "11111111-1111-1111-1111-111111111111", "kind": "agent", "name": "Rocky"}}),
            json!({"id": "a2", "kind": "message", "content": "Looks good.",
                "fromAgent": {"id": "11111111-1111-1111-1111-111111111111", "name": "Rocky"}}),
        ];
        let cards = enrich_cards(vec![], &raws);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].summary(), "↔ agent: Rocky");
        assert_eq!(cards[0].button(), "[view conversation]");
        let Body::SideChat {
            messages,
            peer_kind,
            ..
        } = &cards[0].body
        else {
            panic!()
        };
        assert_eq!(peer_kind, "agent");
        assert_eq!(messages.len(), 2);
        assert_eq!(hidden_side_ids(&cards), vec!["a2".to_string()]);
        assert!(is_side_message(&raws[0]) && is_side_message(&raws[1]));
        assert!(!is_side_message(
            &json!({"kind": "message", "role": "user", "content": "hi"})
        ));
    }
}
