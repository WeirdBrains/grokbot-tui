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
}

/// Real gateway client (unofficial, undocumented API).
pub struct GatewayApi {
    pub client: gw::Client,
    list_notice: Mutex<Option<String>>,
}

impl GatewayApi {
    pub fn new(client: gw::Client) -> Self {
        Self {
            client,
            list_notice: Mutex::new(None),
        }
    }
}

pub fn bot_from_agent(a: &gw::Agent) -> Bot {
    Bot {
        id: a.id.clone(),
        name: gw::sanitize_text(a.display_name()),
        description: gw::sanitize_text(&a.description),
        hidden: a.hidden_from_sidebar.unwrap_or(false),
        is_group: a.is_group,
        team: false,
        viewer_session_id: None,
    }
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

pub fn messages_from_entries(entries: &[gw::Entry]) -> Vec<Message> {
    entries
        .iter()
        .filter_map(|e| {
            let text = gw::sanitize_text(&e.text);
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
        Ok(messages_from_entries(
            &self
                .client
                .transcript_tail(bot_id, limit, None)
                .await?
                .entries,
        ))
    }
    async fn transcript_history(&self, bot_id: &str, max_entries: u32) -> gw::Result<Vec<Message>> {
        Ok(messages_from_entries(
            &self
                .client
                .transcript_history(bot_id, 200, max_entries as usize)
                .await?
                .entries,
        ))
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
}
