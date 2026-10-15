//! Networking behind the [`ApiClient`] trait.
//!
//! [`HttpClient`] uses PLACEHOLDER endpoints from config (no confirmed Grok Bot
//! API spec exists yet). [`MockClient`] works offline (`gb --mock`).
use crate::config::Config;
use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Bot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

#[async_trait]
pub trait ApiClient: Send + Sync {
    fn set_token(&mut self, token: Option<String>);
    async fn whoami(&self) -> Result<User>;
    async fn list_bots(&self) -> Result<Vec<Bot>>;
    async fn history(&self, bot_id: &str) -> Result<Vec<Message>>;
    async fn send(&self, bot_id: &str, text: &str) -> Result<Message>;
    /// Server-side token revocation, if the API supports it.
    async fn revoke(&self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- HTTP

pub struct HttpClient {
    cfg: Config,
    http: reqwest::Client,
    token: Option<String>,
}

impl HttpClient {
    pub fn new(cfg: Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.request_timeout_secs))
            .user_agent(concat!("gb/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            cfg,
            http,
            token: None,
        })
    }

    fn url(&self, path: &str, bot: Option<&str>) -> String {
        let mut p = path.to_string();
        if let Some(b) = bot {
            p = p.replace("{bot}", b);
        }
        format!("{}{}", self.cfg.api_base_url.trim_end_matches('/'), p)
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let t = self
            .token
            .as_ref()
            .ok_or_else(|| anyhow!("not logged in (use /login)"))?;
        Ok(rb.bearer_auth(t))
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: String) -> Result<T> {
        let r = self.auth(self.http.get(&url))?.send().await?;
        check(r).await?.json().await.map_err(Into::into)
    }
}

async fn check(r: reqwest::Response) -> Result<reqwest::Response> {
    if r.status().is_success() {
        Ok(r)
    } else {
        let s = r.status();
        let body = r.text().await.unwrap_or_default();
        bail!("HTTP {s}: {}", body.chars().take(200).collect::<String>())
    }
}

#[async_trait]
impl ApiClient for HttpClient {
    fn set_token(&mut self, token: Option<String>) {
        self.token = token;
    }
    async fn whoami(&self) -> Result<User> {
        self.get_json(self.url(&self.cfg.paths.whoami, None)).await
    }
    async fn list_bots(&self) -> Result<Vec<Bot>> {
        self.get_json(self.url(&self.cfg.paths.bots, None)).await
    }
    async fn history(&self, bot_id: &str) -> Result<Vec<Message>> {
        self.get_json(self.url(&self.cfg.paths.history, Some(bot_id)))
            .await
    }
    async fn send(&self, bot_id: &str, text: &str) -> Result<Message> {
        let url = self.url(&self.cfg.paths.send, Some(bot_id));
        let r = self
            .auth(self.http.post(&url))?
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await?;
        check(r).await?.json().await.map_err(Into::into)
    }
    async fn revoke(&self) -> Result<()> {
        if let Some(p) = &self.cfg.paths.revoke {
            let r = self.auth(self.http.post(self.url(p, None)))?.send().await?;
            check(r).await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Mock

/// Offline client: any non-empty token is accepted except `"bad"`.
pub struct MockClient {
    token: Option<String>,
    chats: Mutex<HashMap<String, Vec<Message>>>,
}

impl Default for MockClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockClient {
    pub fn new() -> Self {
        Self {
            token: None,
            chats: Mutex::new(HashMap::new()),
        }
    }
    fn check(&self) -> Result<()> {
        match self.token.as_deref() {
            None => bail!("not logged in (use /login)"),
            Some("bad") => bail!("HTTP 401: invalid token (mock)"),
            _ => Ok(()),
        }
    }
    fn bots() -> Vec<Bot> {
        [
            ("helper", "Helper", "General assistant"),
            ("coder", "Coder", "Writes code"),
            ("scribe", "Scribe", "Takes notes"),
        ]
        .into_iter()
        .map(|(id, n, d)| Bot {
            id: id.into(),
            name: n.into(),
            description: d.into(),
        })
        .collect()
    }
}

#[async_trait]
impl ApiClient for MockClient {
    fn set_token(&mut self, token: Option<String>) {
        self.token = token;
    }
    async fn whoami(&self) -> Result<User> {
        self.check()?;
        Ok(User {
            id: "u_mock".into(),
            name: "mock-user".into(),
        })
    }
    async fn list_bots(&self) -> Result<Vec<Bot>> {
        self.check()?;
        Ok(Self::bots())
    }
    async fn history(&self, bot_id: &str) -> Result<Vec<Message>> {
        self.check()?;
        Ok(self
            .chats
            .lock()
            .unwrap()
            .get(bot_id)
            .cloned()
            .unwrap_or_default())
    }
    async fn send(&self, bot_id: &str, text: &str) -> Result<Message> {
        self.check()?;
        if !Self::bots().iter().any(|b| b.id == bot_id) {
            bail!("unknown bot {bot_id}");
        }
        let reply = Message {
            role: Role::Bot,
            text: format!("echo: {text}"),
        };
        let mut c = self.chats.lock().unwrap();
        let h = c.entry(bot_id.into()).or_default();
        h.push(Message {
            role: Role::User,
            text: text.into(),
        });
        h.push(reply.clone());
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_requires_login_and_echoes() {
        let mut m = MockClient::new();
        assert!(m.whoami().await.is_err());
        m.set_token(Some("bad".into()));
        assert!(m.whoami().await.is_err());
        m.set_token(Some("t".into()));
        assert_eq!(m.whoami().await.unwrap().name, "mock-user");
        let r = m.send("helper", "hi").await.unwrap();
        assert_eq!(r.text, "echo: hi");
        assert_eq!(m.history("helper").await.unwrap().len(), 2);
    }

    #[test]
    fn http_url_substitution() {
        let cfg = Config {
            api_base_url: "http://x/".into(),
            ..Default::default()
        };
        let c = HttpClient::new(cfg).unwrap();
        assert_eq!(
            c.url("/v1/bots/{bot}/messages", Some("b1")),
            "http://x/v1/bots/b1/messages"
        );
    }
}
