//! gb's view of the backend: a small [`ApiClient`] trait with a real implementation on
//! top of the `grokbot_api` crate ([`GatewayApi`]) and an offline [`MockClient`] (`grokbot-tui --mock`).
use async_trait::async_trait;
use grokbot_api as gw;
pub use grokbot_api::{Delivery, SendReceipt, Session};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    pub is_group: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Bot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub text: String,
    /// Transcript entry id, or `pending:<nonce>` for a local send not yet seen in the transcript.
    pub remote_id: Option<String>,
}

#[async_trait]
pub trait ApiClient: Send + Sync {
    async fn list_bots(&self) -> gw::Result<Vec<Bot>>;
    async fn send(&self, bot_id: &str, text: &str, nonce: &str) -> gw::Result<SendReceipt>;
    async fn transcript(&self, bot_id: &str, limit: u32) -> gw::Result<Vec<Message>>;
    /// Page through chat history (oldest → newest), up to `max_entries`.
    async fn transcript_history(&self, bot_id: &str, max_entries: u32) -> gw::Result<Vec<Message>>;
    /// A session refreshed by a 401 re-exchange since the last call, to persist.
    fn take_refreshed_session(&self) -> Option<Session> {
        None
    }
}

/// Real gateway client (unofficial, undocumented API).
pub struct GatewayApi(pub gw::Client);

pub fn bot_from_agent(a: &gw::Agent) -> Bot {
    Bot {
        id: a.id.clone(),
        name: gw::sanitize_text(a.display_name()),
        description: gw::sanitize_text(&a.description),
        hidden: a.hidden_from_sidebar.unwrap_or(false),
        is_group: a.is_group,
    }
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
            })
        })
        .collect()
}

#[async_trait]
impl ApiClient for GatewayApi {
    async fn list_bots(&self) -> gw::Result<Vec<Bot>> {
        Ok(self
            .0
            .list_agents()
            .await?
            .iter()
            .map(bot_from_agent)
            .collect())
    }
    async fn send(&self, bot_id: &str, text: &str, nonce: &str) -> gw::Result<SendReceipt> {
        let req = gw::SendPrompt {
            client_nonce: Some(nonce.into()),
            ..gw::SendPrompt::new(bot_id, text)
        };
        self.0.send_prompt(&req).await
    }
    async fn transcript(&self, bot_id: &str, limit: u32) -> gw::Result<Vec<Message>> {
        Ok(messages_from_entries(
            &self.0.transcript_tail(bot_id, limit, None).await?.entries,
        ))
    }
    async fn transcript_history(&self, bot_id: &str, max_entries: u32) -> gw::Result<Vec<Message>> {
        Ok(messages_from_entries(
            &self
                .0
                .transcript_history(bot_id, 200, max_entries as usize)
                .await?
                .entries,
        ))
    }
    fn take_refreshed_session(&self) -> Option<Session> {
        self.0.take_refreshed_session()
    }
}

// ---------------------------------------------------------------- Mock

/// Offline client. Token `bad` is rejected (401). Prompt `reject` gets a 400;
/// prompt `unknown` lands but answers 503 (delivery unknown). Replies arrive after a short delay.
pub struct MockClient {
    token: String,
    chats: Mutex<HashMap<String, Vec<(Message, Instant)>>>,
    next: AtomicU64,
    pub reply_delay: Duration,
}

impl MockClient {
    pub fn new(token: &str) -> Self {
        Self {
            token: token.into(),
            chats: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            reply_delay: Duration::from_millis(300),
        }
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
        [
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
        })
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
        Ok(Self::bots())
    }
    async fn send(&self, bot_id: &str, text: &str, nonce: &str) -> gw::Result<SendReceipt> {
        self.check("sendPrompt")?;
        let err = |status, message: &str| gw::Error::Http {
            method: "sendPrompt".into(),
            status,
            message: message.into(),
        };
        if !Self::bots().iter().any(|b| b.id == bot_id) {
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
                },
                now,
            ));
            h.push((
                Message {
                    role: Role::Bot,
                    text: format!("echo: {text}"),
                    remote_id: Some(self.id()),
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
