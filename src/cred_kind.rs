//! Classify pasted login credentials so dashboard API keys are rejected early
//! with a clear "unsupported for Grok Bot chat" message instead of a opaque
//! EnsureSandBox 401.
//!
//! Official docs (2026-11):
//! - Cursor dashboard / team API keys (`crsr_…`, and fine-grained `grok_…` keys)
//!   authenticate Admin / Analytics / Cloud Agents / similar at `api.cursor.com`
//!   (Basic or Bearer). They are **not** documented for EnsureSandBox or the
//!   Grok Bot companion gateway (`listAgents` / `sendPrompt`).
//! - xAI / "Grok API" keys (`xai-…`) authenticate `https://api.x.ai` inference.
//! - Grok Bot chat login still needs a Cursor **access token** (or desktop /
//!   gateway session). There is no documented dashboard-key → gateway exchange.

/// What a pasted `/login cursor` string looks like, by prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredKind {
    /// Likely a session / sand access token (no known dashboard prefix).
    CursorAccessToken,
    /// Cursor team/user API key (`crsr_…`) for Admin / Cloud Agents APIs.
    CursorDashboardKey,
    /// Fine-grained Cursor key (`grok_…`) from the API Keys dashboard.
    /// Cloud Agents docs reject these on some routes (`403 key_not_supported`).
    CursorFineGrainedKey,
    /// xAI Grok inference API key (`xai-…` / `xai_…`).
    XaiGrokApiKey,
}

impl CredKind {
    /// Stable id for tests and `/whoami`-style diagnostics (never the secret).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CursorAccessToken => "cursor-access-token",
            Self::CursorDashboardKey => "cursor-dashboard-api-key",
            Self::CursorFineGrainedKey => "cursor-fine-grained-api-key",
            Self::XaiGrokApiKey => "xai-grok-api-key",
        }
    }

    /// True when this credential must not be sent to EnsureSandBox / stored as
    /// a Cursor access token for Grok Bot chat.
    pub fn unsupported_for_grokbot_gateway(self) -> bool {
        !matches!(self, Self::CursorAccessToken)
    }
}

/// Trim whitespace / surrounding quotes. Does **not** strip `Bearer ` (a pasted
/// `Bearer …` dashboard key must still classify as a dashboard key).
pub fn normalize_pasted_secret(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or(s)
        .trim();
    s.to_string()
}

/// Classify a normalized pasted secret by well-known dashboard prefixes.
pub fn classify_pasted_secret(raw: &str) -> CredKind {
    let s = normalize_pasted_secret(raw);
    // After optional "Bearer " so `Bearer grok_…` still classifies correctly.
    let body = s
        .strip_prefix("Bearer ")
        .or_else(|| s.strip_prefix("bearer "))
        .unwrap_or(&s)
        .trim();
    if body.starts_with("crsr_") {
        CredKind::CursorDashboardKey
    } else if body.starts_with("grok_") {
        CredKind::CursorFineGrainedKey
    } else if body.starts_with("xai-") || body.starts_with("xai_") {
        CredKind::XaiGrokApiKey
    } else {
        CredKind::CursorAccessToken
    }
}

/// User-facing explanation when a dashboard / inference key is pasted into
/// `/login cursor`. Never includes the secret.
pub fn unsupported_gateway_login_message(kind: CredKind) -> String {
    match kind {
        CredKind::CursorAccessToken => {
            // Caller should not use this path for access tokens.
            "This credential type is not supported for Grok Bot chat login.".into()
        }
        CredKind::CursorDashboardKey => {
            "That looks like a Cursor dashboard API key (crsr_…). Those keys authenticate \
             the Admin / Analytics / Cloud Agents APIs at api.cursor.com (Basic or Bearer), \
             not Grok Bot chat. There is no documented way to exchange them for a gateway \
             session (EnsureSandBox / listAgents / sendPrompt). Use /login desktop \
             (recommended), or /login cursor with a Cursor access token from the desktop \
             app — not a key from grok.com/dashboard or cursor.com/dashboard API Keys."
                .into()
        }
        CredKind::CursorFineGrainedKey => {
            "That looks like a fine-grained Cursor API key (grok_…). Dashboard keys of this \
             type are for Cursor HTTP APIs, not Grok Bot chat. Official docs do not support \
             exchanging them via EnsureSandBox for listAgents/sendPrompt (Cloud Agents even \
             returns 403 key_not_supported for grok_ keys on some routes). Use /login desktop \
             (recommended), or /login cursor with a Cursor access token — not a dashboard key."
                .into()
        }
        CredKind::XaiGrokApiKey => {
            "That looks like an xAI / Grok inference API key (xai-…). Those keys call \
             https://api.x.ai for model inference, not the Grok Bot companion gateway. \
             Use /login desktop (recommended), or /login cursor with a Cursor access token."
                .into()
        }
    }
}

/// Redact a secret for logs/errors: keep a short prefix label, never the body.
pub fn redact_secret_for_display(raw: &str) -> String {
    let kind = classify_pasted_secret(raw);
    match kind {
        CredKind::CursorDashboardKey => "crsr_<redacted>".into(),
        CredKind::CursorFineGrainedKey => "grok_<redacted>".into(),
        CredKind::XaiGrokApiKey => "xai-<redacted>".into(),
        CredKind::CursorAccessToken => {
            let n = normalize_pasted_secret(raw);
            if n.is_empty() {
                "<empty>".into()
            } else {
                format!("access-token(<{} chars>)", n.chars().count())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_dashboard_and_inference_prefixes() {
        assert_eq!(
            classify_pasted_secret("crsr_abcdefghijklmnopqrstuvwxyz0123456789"),
            CredKind::CursorDashboardKey
        );
        assert_eq!(
            classify_pasted_secret("  grok_abc123  "),
            CredKind::CursorFineGrainedKey
        );
        assert_eq!(
            classify_pasted_secret("\"xai-deadbeef\""),
            CredKind::XaiGrokApiKey
        );
        assert_eq!(classify_pasted_secret("xai_also"), CredKind::XaiGrokApiKey);
        assert_eq!(
            classify_pasted_secret("Bearer grok_teamkey"),
            CredKind::CursorFineGrainedKey
        );
    }

    #[test]
    fn access_token_shaped_strings_are_not_rejected_by_prefix() {
        // Opaque access tokens have no public prefix convention.
        assert_eq!(
            classify_pasted_secret("eyJhbGciOiJIUzI1NiJ9.payload.sig"),
            CredKind::CursorAccessToken
        );
        assert_eq!(
            classify_pasted_secret("some-long-session-secret-without-known-prefix"),
            CredKind::CursorAccessToken
        );
        assert!(!CredKind::CursorAccessToken.unsupported_for_grokbot_gateway());
        assert!(CredKind::CursorDashboardKey.unsupported_for_grokbot_gateway());
        assert!(CredKind::CursorFineGrainedKey.unsupported_for_grokbot_gateway());
        assert!(CredKind::XaiGrokApiKey.unsupported_for_grokbot_gateway());
    }

    #[test]
    fn unsupported_messages_name_official_apis_and_never_echo_secret() {
        let secret = "grok_this_must_not_appear_in_the_message_body_xyz";
        let msg = unsupported_gateway_login_message(classify_pasted_secret(secret));
        assert!(!msg.contains("this_must_not_appear"));
        assert!(msg.contains("grok_"));
        assert!(msg.to_ascii_lowercase().contains("not") || msg.contains("not "));
        assert!(msg.contains("/login desktop"));
        let xai = unsupported_gateway_login_message(CredKind::XaiGrokApiKey);
        assert!(xai.contains("api.x.ai"));
        assert!(!xai.contains(secret));
        let crsr = unsupported_gateway_login_message(CredKind::CursorDashboardKey);
        assert!(crsr.contains("api.cursor.com"));
    }

    #[test]
    fn redact_never_includes_secret_body() {
        let s = "grok_SUPERSECRETVALUE123456";
        let r = redact_secret_for_display(s);
        assert_eq!(r, "grok_<redacted>");
        assert!(!r.contains("SUPERSECRET"));
        let a = redact_secret_for_display("abcdef");
        assert!(a.starts_with("access-token("));
        assert!(!a.contains("abcdef"));
    }
}
