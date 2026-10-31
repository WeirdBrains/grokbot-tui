//! grokbot-tui's view of the backend: a small [`ApiClient`] trait with a real implementation on
//! top of the `grokbot_api` crate ([`GatewayApi`]) and an offline [`MockClient`] (`grokbot-tui --mock`).
use async_trait::async_trait;
use grokbot_api as gw;
pub use grokbot_api::{Delivery, SendReceipt, Session};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    pub is_group: bool,
    /// A team bot shared with you (owned by someone else). Not in the gateway's listAgents.
    pub team: bool,
    /// Viewer session from ListGrokBotTeamAgents; required as gateway `sessionId`.
    pub viewer_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    User,
    Bot,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    pub role: Role,
    pub text: String,
    /// Transcript entry id, or `pending:<nonce>` for a local send not yet seen in the transcript.
    pub remote_id: Option<String>,
    /// Files sent with the prompt (store aliases or box paths).
    pub attachments: Vec<Attachment>,
}

/// A file attached to a message: the alias/path the backend knows it by and its name.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.content_type
            .as_deref()
            .is_some_and(|c| c.starts_with("image/"))
            || gw::AttachmentKind::from_name(&self.name) == gw::AttachmentKind::Image
    }
}

/// Bytes uploaded so far, shared with the UI while an upload runs.
pub type Progress = std::sync::Arc<AtomicU64>;

/// Attachments recorded on a raw transcript entry. The key is undocumented, so this
/// accepts `attachmentPaths` + `attachmentNames`, or an `attachments` array of strings
/// or objects (`path`/`alias`, `name`/`fileName`, `sizeBytes`, `contentType`).
pub fn attachments_from_raw(raw: &serde_json::Value) -> Vec<Attachment> {
    use serde_json::Value;
    let strs = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let base = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    for (pk, nk) in [
        ("attachmentPaths", "attachmentNames"),
        ("attachment_paths", "attachment_names"),
    ] {
        let paths = strs(raw.get(pk));
        if !paths.is_empty() {
            let names = strs(raw.get(nk));
            return paths
                .into_iter()
                .enumerate()
                .map(|(i, p)| Attachment {
                    name: names.get(i).cloned().unwrap_or_else(|| base(&p)),
                    path: p,
                    ..Default::default()
                })
                .collect();
        }
    }
    let Some(arr) = raw.get("attachments").and_then(Value::as_array) else {
        return vec![];
    };
    let s = |v: &Value, ks: &[&str]| {
        ks.iter()
            .find_map(|k| v.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map(str::to_string)
    };
    arr.iter()
        .filter_map(|a| match a {
            Value::String(p) => Some(Attachment {
                name: base(p),
                path: p.clone(),
                ..Default::default()
            }),
            Value::Object(_) => {
                let path = s(a, &["path", "alias", "attachmentPath", "storePath", "url"])?;
                Some(Attachment {
                    name: s(a, &["name", "fileName", "filename", "displayName"])
                        .unwrap_or_else(|| base(&path)),
                    size: a
                        .get("sizeBytes")
                        .or_else(|| a.get("size"))
                        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok())),
                    content_type: s(a, &["contentType", "mimeType", "mime"]),
                    path,
                })
            }
            _ => None,
        })
        .collect()
}

#[async_trait]
pub trait ApiClient: Send + Sync {
    async fn list_bots(&self) -> gw::Result<Vec<Bot>>;
    /// `createAgent {name, …}`: create a bot with defaults and return it.
    async fn create_bot(&self, name: &str) -> gw::Result<Bot>;
    /// `deleteAgent {id}`.
    async fn delete_bot(&self, bot_id: &str) -> gw::Result<()>;
    async fn send(&self, bot_id: &str, text: &str, nonce: &str) -> gw::Result<SendReceipt>;
    /// `promptAcceptanceStatus`: did the prompt with this nonce land?
    async fn acceptance(&self, bot_id: &str, nonce: &str) -> gw::Result<gw::AcceptanceStatus>;
    async fn transcript(&self, bot_id: &str, limit: u32) -> gw::Result<Vec<Message>>;
    /// Page through chat history (oldest → newest), up to `max_entries`.
    async fn transcript_history(&self, bot_id: &str, max_entries: u32) -> gw::Result<Vec<Message>>;
    /// A session refreshed by a 401 re-exchange since the last call, to persist.
    fn take_refreshed_session(&self) -> Option<Session> {
        None
    }
    /// Bind a shared team bot's viewer session so send/transcript/acceptance send `sessionId`.
    /// `grokbot_api` also remembers sessions from `list_team_agents`; this re-applies from
    /// the TUI's `Bot` so a lost in-memory map (or a mock) cannot silently drop it.
    fn bind_viewer_session(&self, _bot_id: &str, session_id: Option<&str>) {
        let _ = session_id;
    }
    /// Soft notice from the last `list_bots` (e.g. team list failed). Cleared on read.
    fn take_list_notice(&self) -> Option<String> {
        None
    }
    /// This bot's settings (Settings page, 1.33.0). Read-only `listAgents`.
    async fn agent_settings(&self, _bot_id: &str) -> gw::Result<AgentSettings> {
        Err(gw::Error::Validation(
            "bot settings are not supported by this client".into(),
        ))
    }
    /// Write one bot setting the gateway supports (Settings page, 1.33.0).
    async fn update_agent_setting(&self, _bot_id: &str, _change: AgentChange) -> gw::Result<()> {
        Err(gw::Error::Validation(
            "bot settings are not supported by this client".into(),
        ))
    }
    /// Interactive cards seen by the last transcript call for `bot_id` (1.29.0).
    fn take_cards(&self, _bot_id: &str) -> Vec<crate::cards::Card> {
        vec![]
    }
    /// Run one card action (`respondToWidget`, `submitSecret`, ...). `agentId` and, for
    /// a team bot, `sessionId` are added here. Never logs the body.
    async fn card_call(
        &self,
        _bot_id: &str,
        _session: Option<&str>,
        call: crate::cards::Call,
    ) -> gw::Result<()> {
        Err(gw::Error::Validation(format!(
            "{} is not supported by this client",
            call.method
        )))
    }
    /// Send with attachments (aliases from [`ApiClient::upload`]).
    async fn send_with(
        &self,
        bot_id: &str,
        text: &str,
        nonce: &str,
        attachments: &[Attachment],
    ) -> gw::Result<SendReceipt> {
        if attachments.is_empty() {
            self.send(bot_id, text, nonce).await
        } else {
            Err(gw::Error::Attachment(gw::UploadFailure::NoCursorSession))
        }
    }
    /// Whether this login can upload attachments (needs a Cursor session).
    fn can_attach(&self) -> bool {
        false
    }
    /// Upload a file for `bot_id`; `progress` tracks bytes sent.
    async fn upload(
        &self,
        _bot_id: &str,
        _name: &str,
        _data: Vec<u8>,
        _progress: Progress,
    ) -> gw::Result<gw::UploadedAttachment> {
        Err(gw::Error::Attachment(gw::UploadFailure::NoCursorSession))
    }
    /// Presigned reads for attachment paths/aliases.
    async fn resolve_attachments(
        &self,
        _bot_id: &str,
        _paths: &[String],
    ) -> gw::Result<Vec<gw::AttachmentRead>> {
        Err(gw::Error::Attachment(gw::UploadFailure::NoCursorSession))
    }
    /// `ReportGrokBotDesktopActivity`: claim (`active`) or release this install's presence;
    /// returns the lease.
    async fn report_activity(
        &self,
        _install_id: &str,
        _active: bool,
    ) -> gw::Result<std::time::Duration> {
        Err(gw::Error::Attachment(gw::UploadFailure::NoCursorSession))
    }
}

/// Real gateway client (unofficial, undocumented API).
pub struct GatewayApi {
    pub client: gw::Client,
    list_notice: Mutex<Option<String>>,
    cards: Mutex<HashMap<String, Vec<crate::cards::Card>>>,
}

impl GatewayApi {
    pub fn new(client: gw::Client) -> Self {
        Self {
            client,
            list_notice: Mutex::new(None),
            cards: Mutex::new(HashMap::new()),
        }
    }

    fn keep_cards(&self, bot_id: &str, entries: &[gw::Entry]) {
        let c: Vec<_> = entries
            .iter()
            .filter_map(|e| crate::cards::parse(&e.raw))
            .collect();
        self.cards.lock().unwrap().insert(bot_id.into(), c);
    }
}

/// A bot's own settings, read from its `listAgents` record (1.33.0 Settings page).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentSettings {
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub avatar_shape: String,
    pub avatar_color: String,
    /// `notifyOnUpdatesEnabled` (writable: `setAgentNotifyOnUpdates`).
    pub notify_on_updates: Option<bool>,
    /// `notificationsEnabled` (read-only: no setter in the gateway API).
    pub notifications: Option<bool>,
    /// `isHiddenFromSidebar` (writable: `setAgentHiddenFromSidebar`).
    pub hidden: Option<bool>,
    /// `voiceId` / `voiceLanguage` / `voiceSpeed` (read-only).
    pub voice: Option<String>,
    /// `harness` (read-only).
    pub harness: Option<String>,
}

impl AgentSettings {
    /// Parse one raw `listAgents` record (keys surveyed live in 1.33.0).
    pub fn from_raw(v: &serde_json::Value) -> Option<Self> {
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let b = |k: &str| v.get(k).and_then(|x| x.as_bool());
        let id = v
            .get("id")
            .or_else(|| v.get("agentId"))
            .and_then(|x| x.as_str())
            .filter(|x| !x.is_empty())?
            .to_string();
        let voice = {
            let parts: Vec<String> = ["voiceId", "voiceLanguage", "voiceSpeed"]
                .iter()
                .filter_map(|k| match v.get(*k)? {
                    serde_json::Value::String(x) if !x.is_empty() => Some(x.clone()),
                    serde_json::Value::Number(n) => Some(format!("{n}×")),
                    _ => None,
                })
                .collect();
            (!parts.is_empty()).then(|| parts.join(" · "))
        };
        let harness = match v.get("harness") {
            Some(serde_json::Value::String(x)) if !x.is_empty() => Some(x.clone()),
            _ => None,
        };
        Some(Self {
            id,
            name: gw::sanitize_text(&s("name")),
            title: gw::sanitize_text(&s("title")),
            description: gw::sanitize_text(&s("description")),
            avatar_shape: s("avatarShape"),
            avatar_color: s("avatarColor"),
            notify_on_updates: b("notifyOnUpdatesEnabled").or_else(|| b("notifyOnAgentUpdates")),
            notifications: b("notificationsEnabled"),
            hidden: b("isHiddenFromSidebar").or_else(|| b("hiddenFromSidebar")),
            voice,
            harness,
        })
    }

    /// The profile `updateAgent` needs (it always sends name + description).
    pub fn profile(&self) -> gw::AgentProfile {
        gw::AgentProfile {
            name: self.name.clone(),
            description: self.description.clone(),
            title: Some(self.title.clone()),
            avatar_shape: (!self.avatar_shape.is_empty()).then(|| self.avatar_shape.clone()),
            avatar_color: (!self.avatar_color.is_empty()).then(|| self.avatar_color.clone()),
        }
    }
}

/// One change to a bot's settings (only what the gateway API can write).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentChange {
    /// `updateAgent` (name, title, description, avatar shape/color).
    Profile(gw::AgentProfile),
    /// `setAgentNotifyOnUpdates`.
    NotifyOnUpdates(bool),
    /// `setAgentHiddenFromSidebar`.
    Hidden(bool),
}

/// The agent array from a raw `listAgents` answer (an array, or `{agents: [...]}`).
pub fn raw_agents(raw: &serde_json::Value) -> Vec<serde_json::Value> {
    raw.as_array()
        .or_else(|| raw.get("agents").and_then(|a| a.as_array()))
        .cloned()
        .unwrap_or_default()
}

pub fn bot_from_agent(a: &gw::Agent) -> Bot {
    let (name, description) = split_name(
        &gw::sanitize_text(a.display_name()),
        &gw::sanitize_text(&a.description),
    );
    Bot {
        id: a.id.clone(),
        name,
        description,
        hidden: a.hidden_from_sidebar.unwrap_or(false),
        is_group: a.is_group,
        team: false,
        viewer_session_id: None,
    }
}

/// A bot named "Lippy · Very ratatui-focused developer" (description typed into the
/// name): the sidebar shows "Lippy" and the rest becomes the description, unless the
/// bot already has one (then the tail is kept in front of it).
pub fn split_name(name: &str, desc: &str) -> (String, String) {
    match name.split_once(" · ") {
        Some((head, tail)) if !head.trim().is_empty() && !tail.trim().is_empty() => {
            let d = if desc.trim().is_empty() {
                tail.trim().to_string()
            } else {
                format!("{} · {}", tail.trim(), desc.trim())
            };
            (head.trim().to_string(), d)
        }
        _ => (name.to_string(), desc.to_string()),
    }
}

/// What changed between two bot lists (by id): names for the status notice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BotDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Renamed, re-described, hidden/shown, or otherwise changed.
    pub updated: Vec<String>,
}

impl BotDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.updated.is_empty()
    }

    /// e.g. "Bot list refreshed: 1 new (ratatoskn)".
    pub fn notice(&self) -> String {
        if self.is_empty() {
            return "Bot list refreshed: no changes.".into();
        }
        let part = |n: &[String], what: &str| {
            (!n.is_empty()).then(|| format!("{} {what} ({})", n.len(), n.join(", ")))
        };
        let parts: Vec<String> = [
            part(&self.added, "new"),
            part(&self.removed, "removed"),
            part(&self.updated, "updated"),
        ]
        .into_iter()
        .flatten()
        .collect();
        format!("Bot list refreshed: {}", parts.join(", "))
    }
}

pub fn diff_bots(old: &[Bot], new: &[Bot]) -> BotDiff {
    let mut d = BotDiff::default();
    for n in new {
        match old.iter().find(|o| o.id == n.id) {
            None => d.added.push(n.name.clone()),
            Some(o)
                if (&o.name, &o.description, o.hidden, o.is_group, o.team)
                    != (&n.name, &n.description, n.hidden, n.is_group, n.team) =>
            {
                if o.name != n.name {
                    d.updated.push(format!("{} → {}", o.name, n.name));
                } else {
                    d.updated.push(n.name.clone());
                }
            }
            Some(_) => {}
        }
    }
    for o in old {
        if !new.iter().any(|n| n.id == o.id) {
            d.removed.push(o.name.clone());
        }
    }
    d
}

/// Add shared team bots (from the backend team list) that the gateway list lacks.
pub fn merge_team_bots(mut bots: Vec<Bot>, team: &[gw::TeamAgent]) -> Vec<Bot> {
    for t in team {
        if t.viewer_is_owner || bots.iter().any(|b| b.id == t.agent_id) {
            continue;
        }
        let mut description = gw::sanitize_text(&t.description);
        if let Some(o) = &t.owner_display_name {
            description = format!("team bot · owner {} · {description}", gw::sanitize_text(o));
        }
        bots.push(Bot {
            id: t.agent_id.clone(),
            name: gw::sanitize_text(&t.name),
            description,
            hidden: false,
            is_group: t.is_room,
            team: true,
            viewer_session_id: t.viewer_session_id.clone(),
        });
    }
    bots
}

/// The raw entry for a mock card (`card <kind>` in mock mode).
pub fn mock_card(kind: &str, id: &str) -> serde_json::Value {
    use serde_json::json;
    let message = match kind {
        "multi" => json!({"type": "widget", "widget": {"prompt": "Which toppings?",
            "multiSelect": true, "allowCustom": true,
            "options": [{"label": "Cheese"}, {"label": "Olives"}, {"label": "Basil"}]}}),
        "steps" => json!({"type": "widget", "widget": {"prompt": "Order", "steps": [
            {"prompt": "Size?", "options": [{"label": "Small"}, {"label": "Large"}]},
            {"prompt": "Drink?", "options": [{"label": "Tea"}, {"label": "Coffee"}]}]}}),
        "approval" => json!({"type": "auto-review-approval", "approval": {"status": "pending",
            "requestId": format!("req-{id}"), "summary": "Run a shell command",
            "command": "ls -la ~/Downloads", "reason": "Reads your files"}}),
        "tool" => json!({"type": "local-tool-permission", "ask": {"status": "pending",
            "requestId": format!("req-{id}"), "title": "Use the terminal on your computer"}}),
        "secret" => json!({"type": "secret-request", "secretRequest": {"label": "Mock API key",
            "description": "Stored on your box as MOCK_KEY."}}),
        "form" => json!({"type": "user-form", "formRequest": {"title": "Sign in to Mockco",
            "fields": [{"id": "user", "label": "Username", "type": "text", "required": true},
                       {"id": "pass", "label": "Password", "type": "password", "required": true}]}}),
        "draft" => json!({"type": "email-draft", "draft": {"from": "me@example.com",
            "to": ["you@example.com"], "subject": "Hello", "body": "Hi there,\nSee you soon."}}),
        "purchase" => json!({"type": "virtual-card-approval", "request": {"status": "pending",
            "title": "Buy socks", "merchant": "Sockco", "amount": "12.00"}}),
        _ => json!({"type": "widget", "widget": {"prompt": "Pick a color",
            "options": [{"label": "Red"}, {"label": "Green"}, {"label": "Blue"}]}}),
    };
    json!({"id": id, "kind": "send-message", "message": message})
}

pub fn messages_from_entries(entries: &[gw::Entry]) -> Vec<Message> {
    entries
        .iter()
        .filter_map(|e| {
            // Interactive cards (1.29.0) are stored as their one-line summary.
            let text = match crate::cards::parse(&e.raw) {
                Some(c) => c.summary(),
                None => gw::sanitize_text(&e.text),
            };
            if text.trim().is_empty() {
                return None;
            }
            Some(Message {
                role: if e.is_from_user() {
                    Role::User
                } else {
                    Role::Bot
                },
                text,
                remote_id: e.id.clone(),
                attachments: attachments_from_raw(&e.raw),
            })
        })
        .collect()
}

#[async_trait]
impl ApiClient for GatewayApi {
    async fn agent_settings(&self, bot_id: &str) -> gw::Result<AgentSettings> {
        let raw = self
            .client
            .call_raw("listAgents", serde_json::json!({}))
            .await?;
        raw_agents(&raw)
            .iter()
            .filter_map(AgentSettings::from_raw)
            .find(|a| a.id == bot_id)
            .ok_or_else(|| {
                gw::Error::Validation(
                    "this bot isn't on your box (a team bot's settings belong to its owner)".into(),
                )
            })
    }
    async fn update_agent_setting(&self, bot_id: &str, change: AgentChange) -> gw::Result<()> {
        match change {
            AgentChange::Profile(p) => self.client.update_agent(bot_id, &p).await.map(|_| ()),
            AgentChange::NotifyOnUpdates(on) => self
                .client
                .set_agent_notify_on_updates(bot_id, on)
                .await
                .map(|_| ()),
            AgentChange::Hidden(h) => self
                .client
                .set_agent_hidden_from_sidebar(bot_id, h)
                .await
                .map(|_| ()),
        }
    }
    async fn acceptance(&self, bot_id: &str, nonce: &str) -> gw::Result<gw::AcceptanceStatus> {
        self.client.prompt_acceptance_status(bot_id, nonce).await
    }
    /// Gateway `listAgents` (your box) plus, when a Cursor token is available, team bots
    /// from the backend's `ListGrokBotTeamAgents`. A team-list failure keeps your bots
    /// and records a notice via [`ApiClient::take_list_notice`].
    async fn list_bots(&self) -> gw::Result<Vec<Bot>> {
        let own: Vec<Bot> = self
            .client
            .list_agents()
            .await?
            .iter()
            .map(bot_from_agent)
            .collect();
        match self.client.list_team_agents().await {
            Ok(Some(team)) => {
                let bots = merge_team_bots(own, &team);
                // Belt-and-suspenders: re-bind viewer sessions from the merged list.
                for b in &bots {
                    if let Some(s) = &b.viewer_session_id {
                        self.client.set_agent_session(&b.id, s);
                    }
                }
                let missing = bots
                    .iter()
                    .filter(|b| b.team && b.viewer_session_id.is_none())
                    .map(|b| b.name.as_str())
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    *self.list_notice.lock().unwrap() = Some(format!(
                        "Team bot(s) missing viewerSessionId (cannot send/history): {}.                          Re-login with /login cursor or /login desktop.",
                        missing.join(", ")
                    ));
                }
                Ok(bots)
            }
            Ok(None) => Ok(own),
            Err(e) => {
                *self.list_notice.lock().unwrap() = Some(format!(
                    "Could not list team bots (shared agents hidden): {e}"
                ));
                Ok(own)
            }
        }
    }
    async fn create_bot(&self, name: &str) -> gw::Result<Bot> {
        let input = gw::CreateAgent {
            name: name.trim().into(),
            ..Default::default()
        };
        if let Some(a) = self.client.create_agent(&input).await? {
            return Ok(bot_from_agent(&a));
        }
        // No agent in the reply: find it by name in a fresh list.
        let want = gw::sanitize_text(name.trim());
        self.list_bots()
            .await?
            .into_iter()
            .rev()
            .find(|b| b.name == want)
            .ok_or_else(|| gw::Error::Http {
                method: "createAgent".into(),
                status: 502,
                message: "created, but the new bot is not in listAgents yet; try /bots".into(),
            })
    }
    async fn delete_bot(&self, bot_id: &str) -> gw::Result<()> {
        self.client.delete_agent(bot_id).await.map(|_| ())
    }
    async fn send(&self, bot_id: &str, text: &str, nonce: &str) -> gw::Result<SendReceipt> {
        self.send_with(bot_id, text, nonce, &[]).await
    }
    async fn send_with(
        &self,
        bot_id: &str,
        text: &str,
        nonce: &str,
        attachments: &[Attachment],
    ) -> gw::Result<SendReceipt> {
        let req = gw::SendPrompt {
            client_nonce: Some(nonce.into()),
            attachment_paths: attachments.iter().map(|a| a.path.clone()).collect(),
            attachment_names: attachments.iter().map(|a| a.name.clone()).collect(),
            ..gw::SendPrompt::new(bot_id, text)
        };
        self.client.send_prompt(&req).await
    }
    fn can_attach(&self) -> bool {
        self.client.has_cursor_session()
    }
    async fn upload(
        &self,
        bot_id: &str,
        name: &str,
        data: Vec<u8>,
        progress: Progress,
    ) -> gw::Result<gw::UploadedAttachment> {
        self.client
            .upload_attachment(bot_id, None, name, &data, |p| {
                progress.store(p.sent, Ordering::Relaxed)
            })
            .await
    }
    async fn report_activity(
        &self,
        install_id: &str,
        active: bool,
    ) -> gw::Result<std::time::Duration> {
        self.client
            .report_desktop_activity(install_id, active)
            .await
    }
    async fn resolve_attachments(
        &self,
        bot_id: &str,
        paths: &[String],
    ) -> gw::Result<Vec<gw::AttachmentRead>> {
        self.client
            .resolve_attachment_reads(bot_id, None, paths)
            .await
    }
    async fn transcript(&self, bot_id: &str, limit: u32) -> gw::Result<Vec<Message>> {
        let t = self.client.transcript_tail(bot_id, limit, None).await?;
        self.keep_cards(bot_id, &t.entries);
        Ok(messages_from_entries(&t.entries))
    }
    async fn transcript_history(&self, bot_id: &str, max_entries: u32) -> gw::Result<Vec<Message>> {
        let t = self
            .client
            .transcript_history(bot_id, 200, max_entries as usize)
            .await?;
        self.keep_cards(bot_id, &t.entries);
        Ok(messages_from_entries(&t.entries))
    }
    fn take_cards(&self, bot_id: &str) -> Vec<crate::cards::Card> {
        self.cards
            .lock()
            .unwrap()
            .remove(bot_id)
            .unwrap_or_default()
    }
    async fn card_call(
        &self,
        bot_id: &str,
        session: Option<&str>,
        call: crate::cards::Call,
    ) -> gw::Result<()> {
        let mut body = call.body;
        body["agentId"] = serde_json::Value::String(bot_id.into());
        if let Some(s) = session {
            body["sessionId"] = serde_json::Value::String(s.into());
        }
        self.client.call_raw(call.method, body).await.map(|_| ())
    }
    fn take_refreshed_session(&self) -> Option<Session> {
        self.client.take_refreshed_session()
    }
    fn bind_viewer_session(&self, bot_id: &str, session_id: Option<&str>) {
        if let Some(s) = session_id {
            self.client.set_agent_session(bot_id, s);
        }
    }
    fn take_list_notice(&self) -> Option<String> {
        self.list_notice.lock().unwrap().take()
    }
}

// ---------------------------------------------------------------- Mock

/// Offline client. Token `bad` is rejected (401). Prompt `reject` gets a 400;
/// prompt `unknown` lands but answers 503 (delivery unknown). Replies arrive after a short delay.
pub struct MockClient {
    token: String,
    chats: Mutex<HashMap<String, Vec<(Message, Instant)>>>,
    nonces: Mutex<HashMap<String, gw::AcceptanceStatus>>,
    /// Bots made with `create_bot` / removed with `delete_bot`.
    extra: Mutex<Vec<Bot>>,
    deleted: Mutex<Vec<String>>,
    /// Agent id → viewer sessionId (mirrors grokbot_api's map).
    sessions: Mutex<HashMap<String, String>>,
    next: AtomicU64,
    pub reply_delay: Duration,
    /// Raw card entries per bot (mock prompts `card <kind>`), and card calls made.
    cards: Mutex<HashMap<String, Vec<serde_json::Value>>>,
    pub card_calls: Mutex<Vec<(String, serde_json::Value)>>,
    /// Per-bot settings edited through the Settings page, and the writes made.
    settings: Mutex<HashMap<String, AgentSettings>>,
    pub setting_calls: Mutex<Vec<(String, AgentChange)>>,
}

impl MockClient {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.into(),
            chats: Mutex::new(HashMap::new()),
            nonces: Mutex::new(HashMap::new()),
            extra: Mutex::new(Vec::new()),
            deleted: Mutex::new(Vec::new()),
            sessions: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            reply_delay: Duration::from_millis(300),
            cards: Mutex::new(HashMap::new()),
            card_calls: Mutex::new(Vec::new()),
            settings: Mutex::new(HashMap::new()),
            setting_calls: Mutex::new(Vec::new()),
        }
    }

    fn require_team_session(&self, bot_id: &str, method: &str) -> gw::Result<()> {
        let bot = self.current_bots().into_iter().find(|b| b.id == bot_id);
        let Some(b) = bot else {
            return Ok(());
        };
        if !b.team {
            return Ok(());
        }
        let bound = self.sessions.lock().unwrap().get(bot_id).cloned();
        if bound.is_none() {
            return Err(gw::Error::Http {
                method: method.into(),
                status: 404,
                message: "team agent requires sessionId (viewer session not bound)".into(),
            });
        }
        Ok(())
    }

    fn check(&self, method: &str) -> gw::Result<()> {
        if self.token == "bad" {
            return Err(gw::Error::Http {
                method: method.into(),
                status: 401,
                message: "invalid token (mock)".into(),
            });
        }
        Ok(())
    }

    fn bots() -> Vec<Bot> {
        let mut v: Vec<Bot> = [
            ("helper", "Helper", "General assistant", false),
            ("coder", "Coder", "Writes code", false),
            ("scribe", "Scribe", "Takes notes", false),
            ("secret", "Secret", "Hidden from sidebar", true),
        ]
        .into_iter()
        .map(|(id, n, d, h)| Bot {
            id: id.into(),
            name: n.into(),
            description: d.into(),
            hidden: h,
            is_group: false,
            team: false,
            viewer_session_id: None,
        })
        .collect();
        // Shared team bot (like production Chomp): needs viewer sessionId on send/history.
        v.push(Bot {
            id: "c61d849e-c1ec-5ec8-add4-078ba3518635".into(),
            name: "Chomp".into(),
            description: "team bot · owner Mandible · mock".into(),
            hidden: false,
            is_group: false,
            team: true,
            viewer_session_id: Some("vs-chomp".into()),
        });
        v
    }

    /// Built-in bots plus created ones, minus deleted ones.
    fn current_bots(&self) -> Vec<Bot> {
        let deleted = self.deleted.lock().unwrap().clone();
        Self::bots()
            .into_iter()
            .chain(self.extra.lock().unwrap().iter().cloned())
            .filter(|b| !deleted.contains(&b.id))
            .collect()
    }

    fn id(&self) -> String {
        format!("m{}", self.next.fetch_add(1, Ordering::Relaxed))
    }
}

/// Mock cursor exchange: any token except `bad` yields a session.
pub fn mock_ensure_sandbox(cursor_token: &str) -> gw::Result<Session> {
    if cursor_token == "bad" {
        return Err(gw::Error::Http {
            method: "EnsureSandBox".into(),
            status: 401,
            message: "invalid token (mock)".into(),
        });
    }
    Ok(Session::new(
        "https://mock.cursorvm.com",
        format!("gw-{cursor_token}"),
    ))
}

#[async_trait]
impl ApiClient for MockClient {
    async fn agent_settings(&self, bot_id: &str) -> gw::Result<AgentSettings> {
        self.check("listAgents")?;
        if let Some(s) = self.settings.lock().unwrap().get(bot_id) {
            return Ok(s.clone());
        }
        let Some(b) = self.current_bots().into_iter().find(|b| b.id == bot_id) else {
            return Err(gw::Error::Http {
                method: "listAgents".into(),
                status: 404,
                message: "unknown agent (mock)".into(),
            });
        };
        if b.team {
            return Err(gw::Error::Validation(
                "this bot isn't on your box (a team bot's settings belong to its owner)".into(),
            ));
        }
        Ok(AgentSettings {
            id: b.id,
            name: b.name,
            title: "Mock title".into(),
            description: b.description,
            avatar_shape: "circle".into(),
            avatar_color: "blue".into(),
            notify_on_updates: Some(true),
            notifications: Some(true),
            hidden: Some(b.hidden),
            voice: None,
            harness: Some("mock".into()),
        })
    }
    async fn update_agent_setting(&self, bot_id: &str, change: AgentChange) -> gw::Result<()> {
        let mut cur = self.agent_settings(bot_id).await?;
        match &change {
            AgentChange::Profile(p) => {
                if p.name.trim().is_empty() {
                    return Err(gw::Error::Validation("Name cannot be blank.".into()));
                }
                cur.name = p.name.trim().into();
                cur.description = p.description.clone();
                if let Some(t) = &p.title {
                    cur.title = t.clone();
                }
                if let Some(x) = &p.avatar_shape {
                    cur.avatar_shape = x.clone();
                }
                if let Some(x) = &p.avatar_color {
                    cur.avatar_color = x.clone();
                }
            }
            AgentChange::NotifyOnUpdates(on) => cur.notify_on_updates = Some(*on),
            AgentChange::Hidden(h) => cur.hidden = Some(*h),
        }
        self.settings.lock().unwrap().insert(bot_id.into(), cur);
        self.setting_calls
            .lock()
            .unwrap()
            .push((bot_id.into(), change));
        Ok(())
    }
    fn take_cards(&self, bot_id: &str) -> Vec<crate::cards::Card> {
        self.cards
            .lock()
            .unwrap()
            .get(bot_id)
            .map(|v| v.iter().filter_map(crate::cards::parse).collect())
            .unwrap_or_default()
    }
    async fn card_call(
        &self,
        bot_id: &str,
        _session: Option<&str>,
        call: crate::cards::Call,
    ) -> gw::Result<()> {
        self.check(call.method)?;
        let id = call.body["entryId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let mut all = self.cards.lock().unwrap();
        let Some(raw) = all
            .get_mut(bot_id)
            .and_then(|v| v.iter_mut().find(|r| r["id"] == id.as_str()))
        else {
            return Err(gw::Error::Http {
                method: call.method.into(),
                status: 404,
                message: "unknown entry (mock)".into(),
            });
        };
        // Mirror the server's state fields so the next poll shows the outcome.
        match call.method {
            "respondToWidget" => raw["respondedValue"] = call.body["value"].clone(),
            "dismissWidget" => raw["widgetDismissed"] = true.into(),
            "submitSecret" => raw["secretProvided"] = true.into(),
            "submitUserForm" => raw["formResolution"] = "submitted".into(),
            "dismissUserForm" => raw["formResolution"] = "dismissed".into(),
            "sendDraft" => raw["draftSendState"] = "sent".into(),
            "discardDraft" => raw["draftSendState"] = "discarded".into(),
            "resolveAutoReviewApproval" => {
                raw["message"]["approval"]["status"] = call.body["resolution"].clone()
            }
            "resolveLocalToolPermission" => {
                raw["message"]["ask"]["status"] = call.body["resolution"].clone()
            }
            _ => {}
        }
        drop(all);
        let mut body = call.body;
        body["agentId"] = bot_id.into();
        self.card_calls
            .lock()
            .unwrap()
            .push((call.method.to_string(), body));
        Ok(())
    }
    async fn list_bots(&self) -> gw::Result<Vec<Bot>> {
        self.check("listAgents")?;
        let bots = self.current_bots();
        for b in &bots {
            if let Some(s) = &b.viewer_session_id {
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(b.id.clone(), s.clone());
            }
        }
        Ok(bots)
    }
    async fn create_bot(&self, name: &str) -> gw::Result<Bot> {
        self.check("createAgent")?;
        let name = name.trim();
        if name.is_empty() || name == "reject" {
            return Err(gw::Error::Http {
                method: "createAgent".into(),
                status: 400,
                message: "invalid name (mock)".into(),
            });
        }
        let b = Bot {
            id: format!("new-{}", self.id()),
            name: name.into(),
            description: String::new(),
            hidden: false,
            is_group: false,
            team: false,
            viewer_session_id: None,
        };
        self.extra.lock().unwrap().push(b.clone());
        Ok(b)
    }
    async fn delete_bot(&self, bot_id: &str) -> gw::Result<()> {
        self.check("deleteAgent")?;
        if !self.current_bots().iter().any(|b| b.id == bot_id) {
            return Err(gw::Error::Http {
                method: "deleteAgent".into(),
                status: 404,
                message: "unknown agent (mock)".into(),
            });
        }
        self.deleted.lock().unwrap().push(bot_id.into());
        Ok(())
    }
    async fn acceptance(&self, _bot_id: &str, nonce: &str) -> gw::Result<gw::AcceptanceStatus> {
        self.check("promptAcceptanceStatus")?;
        Ok(match self.nonces.lock().unwrap().get(nonce) {
            Some(s) => s.clone(),
            None => gw::AcceptanceStatus::NotFound,
        })
    }
    async fn send(&self, bot_id: &str, text: &str, nonce: &str) -> gw::Result<SendReceipt> {
        self.send_with(bot_id, text, nonce, &[]).await
    }
    fn can_attach(&self) -> bool {
        true
    }
    async fn upload(
        &self,
        _bot_id: &str,
        name: &str,
        data: Vec<u8>,
        progress: Progress,
    ) -> gw::Result<gw::UploadedAttachment> {
        self.check("BeginGrokBotAttachmentUpload")?;
        gw::attachments::validate_attachment(name, data.len() as u64)?;
        if name.starts_with("refuse") {
            return Err(gw::Error::Attachment(gw::UploadFailure::Refused {
                reason: gw::UploadRefusal::UnsupportedType,
                limit_bytes: None,
            }));
        }
        if name.starts_with("usebox") {
            return Err(gw::Error::Attachment(gw::UploadFailure::UseBox {
                reason: String::new(),
            }));
        }
        progress.store(data.len() as u64, Ordering::Relaxed);
        Ok(gw::UploadedAttachment {
            alias: format!("mock-att-{}", self.id()),
            name: name.into(),
            size_bytes: data.len() as u64,
            content_type: String::new(),
        })
    }
    async fn report_activity(
        &self,
        _install_id: &str,
        _active: bool,
    ) -> gw::Result<std::time::Duration> {
        self.check("ReportGrokBotDesktopActivity")?;
        Ok(std::time::Duration::from_secs(60))
    }
    async fn resolve_attachments(
        &self,
        _bot_id: &str,
        paths: &[String],
    ) -> gw::Result<Vec<gw::AttachmentRead>> {
        Ok(paths
            .iter()
            .map(|p| gw::AttachmentRead {
                path: p.clone(),
                outcome: gw::ReadOutcome::Store,
                url: format!("https://mock.invalid/{p}"),
                ..Default::default()
            })
            .collect())
    }
    async fn send_with(
        &self,
        bot_id: &str,
        text: &str,
        nonce: &str,
        attachments: &[Attachment],
    ) -> gw::Result<SendReceipt> {
        self.check("sendPrompt")?;
        self.require_team_session(bot_id, "sendPrompt")?;
        // Mock texts drive the acceptance status seen by a later check.
        let status = match text {
            "lost" => None,
            "refused-later" => Some(gw::AcceptanceStatus::Rejected(Some("mock_refused".into()))),
            _ => Some(gw::AcceptanceStatus::Accepted),
        };
        if let Some(s) = status {
            self.nonces.lock().unwrap().insert(nonce.into(), s);
        }
        if matches!(text, "lost" | "refused-later") {
            return Err(gw::Error::Transport {
                method: "sendPrompt".into(),
                message: "connection reset (mock)".into(),
            });
        }
        let err = |status, message: &str| gw::Error::Http {
            method: "sendPrompt".into(),
            status,
            message: message.into(),
        };
        if !self.current_bots().iter().any(|b| b.id == bot_id) {
            return Err(err(404, "unknown agent (mock)"));
        }
        if text == "reject" {
            return Err(err(400, "rejected (mock)"));
        }
        let now = Instant::now();
        let mid = self.id();
        if let Some(kind) = text.strip_prefix("card ") {
            // Mock: `card widget|multi|approval|secret|form|draft|purchase` posts that card.
            let id = self.id();
            let raw = mock_card(kind.trim(), &id);
            let card = crate::cards::parse(&raw).expect("mock card parses");
            self.cards
                .lock()
                .unwrap()
                .entry(bot_id.into())
                .or_default()
                .push(raw);
            let mut c = self.chats.lock().unwrap();
            let h = c.entry(bot_id.into()).or_default();
            h.push((
                Message {
                    role: Role::User,
                    text: text.into(),
                    remote_id: Some(mid.clone()),
                    attachments: attachments.to_vec(),
                },
                now,
            ));
            h.push((
                Message {
                    role: Role::Bot,
                    text: card.summary(),
                    remote_id: Some(id),
                    attachments: vec![],
                },
                now,
            ));
            return Ok(SendReceipt {
                message_id: Some(mid),
                client_nonce: nonce.into(),
                delivery: Delivery::Accepted,
                raw: serde_json::json!({}),
            });
        }
        {
            let mut c = self.chats.lock().unwrap();
            let h = c.entry(bot_id.into()).or_default();
            h.push((
                Message {
                    role: Role::User,
                    text: text.into(),
                    remote_id: Some(mid.clone()),
                    attachments: attachments.to_vec(),
                },
                now,
            ));
            h.push((
                Message {
                    role: Role::Bot,
                    text: format!("echo: {text}"),
                    remote_id: Some(self.id()),
                    attachments: vec![],
                },
                now + self.reply_delay,
            ));
        }
        if text == "unknown" {
            return Err(err(503, "upstream timeout (mock)"));
        }
        Ok(SendReceipt {
            message_id: Some(mid),
            client_nonce: nonce.into(),
            delivery: Delivery::Accepted,
            raw: serde_json::json!({}),
        })
    }
    async fn transcript_history(&self, bot_id: &str, max_entries: u32) -> gw::Result<Vec<Message>> {
        self.transcript(bot_id, max_entries).await
    }
    async fn transcript(&self, bot_id: &str, limit: u32) -> gw::Result<Vec<Message>> {
        self.check("getAgentTranscriptTail")?;
        self.require_team_session(bot_id, "getAgentTranscriptTail")?;
        let now = Instant::now();
        let c = self.chats.lock().unwrap();
        let v: Vec<Message> = c
            .get(bot_id)
            .map(|h| {
                h.iter()
                    .filter(|(_, t)| *t <= now)
                    .map(|(m, _)| m.clone())
                    .collect()
            })
            .unwrap_or_default();
        let skip = v.len().saturating_sub(limit as usize);
        Ok(v.into_iter().skip(skip).collect())
    }
    fn bind_viewer_session(&self, bot_id: &str, session_id: Option<&str>) {
        match session_id {
            Some(s) => {
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(bot_id.into(), s.into());
            }
            None => {
                self.sessions.lock().unwrap().remove(bot_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_bots_merged_once_and_marked() {
        let own = vec![Bot {
            id: "r".into(),
            name: "Rocky".into(),
            ..Default::default()
        }];
        let team = gw::TeamAgent::list(&json!({"entries": [
            {"agent": {"agentId": "r", "name": "Rocky", "viewerIsOwner": true}},
            {"agent": {"agentId": "c", "name": "Chomp", "viewerIsOwner": false,
                       "visibility": "GROK_BOT_AGENT_VISIBILITY_TEAM", "viewerSessionId": "s1",
                       "ownerDisplayName": "Ana"}},
            {"agent": {"name": "no id"}}
        ]}));
        assert_eq!(team.len(), 2);
        assert!(team[1].team_visible && !team[1].viewer_is_owner);
        assert_eq!(team[1].viewer_session_id.as_deref(), Some("s1"));
        let all = merge_team_bots(own, &team);
        assert_eq!(all.len(), 2, "own bot not duplicated");
        assert!(!all[0].team);
        assert_eq!(
            (all[1].id.as_str(), all[1].name.as_str(), all[1].team),
            ("c", "Chomp", true)
        );
        assert_eq!(all[1].viewer_session_id.as_deref(), Some("s1"));
        assert!(all[1].description.contains("owner Ana"));
    }

    #[tokio::test]
    async fn mock_team_bot_needs_viewer_session_for_send_and_history() {
        let m = MockClient::new("t");
        // Fresh client: sessions not bound until list_bots (or bind_viewer_session).
        let err = m
            .send("c61d849e-c1ec-5ec8-add4-078ba3518635", "hi", "n")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("sessionId"), "{err}");
        let err = m
            .transcript("c61d849e-c1ec-5ec8-add4-078ba3518635", 10)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("sessionId"), "{err}");

        let bots = m.list_bots().await.unwrap();
        let chomp = bots.iter().find(|b| b.name == "Chomp").unwrap();
        assert!(chomp.team);
        assert_eq!(chomp.viewer_session_id.as_deref(), Some("vs-chomp"));
        m.send(&chomp.id, "hi", "n2").await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        // reply_delay default 300ms — bind path only needs send Accepted
        let _ = m.transcript(&chomp.id, 10).await.unwrap();
    }

    #[tokio::test]
    async fn mock_bind_viewer_session_repairs_lost_map() {
        let m = MockClient::new("t");
        let id = "c61d849e-c1ec-5ec8-add4-078ba3518635";
        // Simulate a lost in-memory map after list (clear).
        m.list_bots().await.unwrap();
        m.sessions.lock().unwrap().clear();
        assert!(m.send(id, "x", "n").await.is_err());
        m.bind_viewer_session(id, Some("vs-chomp"));
        m.send(id, "x", "n").await.unwrap();
    }
    use serde_json::json;

    #[tokio::test]
    async fn mock_send_then_reply_after_delay() {
        let m = MockClient {
            reply_delay: Duration::from_millis(30),
            ..MockClient::new("t")
        };
        m.send("helper", "hi", "n").await.unwrap();
        assert_eq!(m.transcript("helper", 50).await.unwrap().len(), 1);
        tokio::time::sleep(Duration::from_millis(40)).await;
        let t = m.transcript("helper", 50).await.unwrap();
        assert_eq!(t[1].text, "echo: hi");
        assert_eq!(
            m.send("helper", "reject", "n")
                .await
                .unwrap_err()
                .delivery(),
            Delivery::Rejected
        );
        assert_eq!(
            m.send("helper", "unknown", "n")
                .await
                .unwrap_err()
                .delivery(),
            Delivery::Unknown
        );
        assert!(
            MockClient::new("bad")
                .list_bots()
                .await
                .unwrap_err()
                .is_unauthorized()
        );
    }

    #[test]
    fn entries_map_to_messages_sanitized() {
        let t = [
            gw::Entry::from_value(&json!({"id": "1", "kind": "user-prompt", "prompt": "hi"})),
            gw::Entry::from_value(
                &json!({"id": "2", "kind": "send-message", "message": {"content": "\u{1b}[31mred\u{1b}[0m"}}),
            ),
            gw::Entry::from_value(&json!({"id": "3", "kind": "tool"})),
        ];
        let m = messages_from_entries(&t);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].role, Role::User);
        assert_eq!((m[1].role, m[1].text.as_str()), (Role::Bot, "red"));
    }

    #[test]
    fn diff_bots_reports_new_removed_and_updated() {
        let b = |id: &str, name: &str| Bot {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        };
        let old = vec![b("a", "A"), b("b", "B"), b("c", "C")];
        let mut new = vec![b("a", "A"), b("c", "C2"), b("r", "ratatoskn")];
        new[0].viewer_session_id = Some("s".into()); // session churn is not a change
        let d = diff_bots(&old, &new);
        assert_eq!(d.added, ["ratatoskn"]);
        assert_eq!(d.removed, ["B"]);
        assert_eq!(d.updated, ["C → C2"]);
        assert_eq!(
            d.notice(),
            "Bot list refreshed: 1 new (ratatoskn), 1 removed (B), 1 updated (C → C2)"
        );
        assert!(diff_bots(&old, &old).is_empty());
        assert_eq!(
            diff_bots(&old[..1], &[b("a", "A"), b("r", "ratatoskn")]).notice(),
            "Bot list refreshed: 1 new (ratatoskn)"
        );
    }

    #[test]
    fn split_name_moves_an_inline_description_out_of_the_name() {
        assert_eq!(
            split_name("Lippy · Very ratatui-lipgloss focused rust developer", ""),
            (
                "Lippy".into(),
                "Very ratatui-lipgloss focused rust developer".into()
            )
        );
        assert_eq!(
            split_name("Lippy · x", "y"),
            ("Lippy".into(), "x · y".into())
        );
        assert_eq!(split_name("Rocky", "d"), ("Rocky".into(), "d".into()));
        assert_eq!(split_name(" · x", ""), (" · x".into(), "".into()));
    }
}
