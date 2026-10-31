//! `grokbot-tui mcp ...`: grokbot-tui as an MCP server (1.31.0), over stdio or streamable
//! HTTP on localhost. See README "MCP server".
pub mod config;
pub mod tools;

use crate::api::{ApiClient, GatewayApi, MockClient};
use crate::storage::{LoginMethod, Store};
use anyhow::{Context, Result, anyhow};
use config::{Audit, ServerConfig};
use grokbot_api as gw;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tools::Ctx;

#[derive(clap::Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum McpCmd {
    /// Run the MCP server (stdio by default; --http for streamable HTTP on localhost).
    Serve {
        /// Serve over stdin/stdout (default).
        #[arg(long, conflicts_with = "http")]
        stdio: bool,
        /// Serve streamable HTTP at ADDR (loopback only; default from mcp.toml, else
        /// 127.0.0.1:8765). Clients send `Authorization: Bearer <grokbot-tui mcp token>`.
        #[arg(long, value_name = "ADDR", num_args = 0..=1, default_missing_value = "")]
        http: Option<String>,
        /// Refuse send_message and answer_card for this run (overrides mcp.toml).
        #[arg(long)]
        read_only: bool,
    },
    /// Print the HTTP bearer token (created on first use) for a client's config.
    Token {
        /// Make a new token; clients using the old one stop working.
        #[arg(long)]
        rotate: bool,
    },
    /// Show the MCP settings in use and where they live.
    Config,
}

/// MCP endpoint path for HTTP.
pub const HTTP_PATH: &str = "/mcp";

pub fn run(store: Store, mock: bool, cmd: McpCmd) -> Result<()> {
    let dir = store.dir.clone();
    match cmd {
        McpCmd::Token { rotate } => {
            // The user asked for it: stdout only, nothing logged.
            println!("{}", config::token(&dir, rotate)?);
            if rotate {
                eprintln!(
                    "grokbot-tui: new MCP token saved; restart `mcp serve --http` and update your clients."
                );
            }
            Ok(())
        }
        McpCmd::Config => {
            let c = ServerConfig::load(&dir)?;
            println!("settings: {}", dir.join(config::CONFIG_FILE).display());
            println!(
                "token:    {} (print it with `grokbot-tui mcp token`)",
                dir.join(config::TOKEN_FILE).display()
            );
            println!(
                "audit:    {}",
                if c.audit {
                    Audit::default_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "-".into())
                } else {
                    "off".into()
                }
            );
            println!(
                "[mcp.server]\nallowed_bots = {:?}\nread_only = {}\nhttp_addr = {:?}\naudit = {}",
                c.allowed_bots, c.read_only, c.http_addr, c.audit
            );
            Ok(())
        }
        McpCmd::Serve {
            stdio: _,
            http,
            read_only,
        } => {
            let mut cfg = ServerConfig::load(&dir)?;
            cfg.read_only |= read_only;
            let addr = match &http {
                Some(a) if a.is_empty() => Some(config::parse_http_addr(&cfg.http_addr)?),
                Some(a) => Some(config::parse_http_addr(a)?),
                None => None,
            };
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async move {
                let transport = if addr.is_some() { "http" } else { "stdio" };
                let ctx = build_ctx(store, mock, cfg, transport).await?;
                match addr {
                    Some(a) => serve_http(ctx, a, &dir).await,
                    None => serve_stdio(ctx).await,
                }
            })
        }
    }
}

/// Sign in with the account's stored login, the way the TUI does (no prompts).
async fn connect(store: &Store) -> Result<Arc<dyn ApiClient>> {
    let config = store.load_config()?;
    let backend_base = std::env::var(crate::config::CURSOR_API_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| gw::DEFAULT_BACKEND_BASE.into());
    let mut opts = gw::ClientOptions {
        policy: gw::Policy::from_env(),
        backend_base,
        max_response_bytes: crate::config::max_response_bytes(),
        ..Default::default()
    };
    crate::app::apply_client_identity(&mut opts, &config, true);
    let stored = store.load_login()?;
    let env_tok = std::env::var(crate::config::TOKEN_ENV)
        .ok()
        .filter(|t| !t.trim().is_empty());
    let client = if let (Some(tok), true) = (env_tok, store.is_startup_account()) {
        let url = std::env::var(crate::config::GATEWAY_URL_ENV)
            .ok()
            .filter(|u| !u.trim().is_empty())
            .or_else(|| stored.as_ref().map(|l| l.session.gateway_url.clone()))
            .ok_or_else(|| anyhow!("{} is set but no gateway URL", crate::config::TOKEN_ENV))?;
        gw::Client::new(gw::Session::new(url.trim(), tok.trim()), opts)?
    } else {
        let l = stored.ok_or_else(|| {
            anyhow!(
                "account '{}' is not logged in: run grokbot-tui and /login first",
                store.account().alias
            )
        })?;
        match (l.method, l.cursor_token) {
            (_, Some(t)) => gw::Client::with_cursor_token(l.session, t, opts)?,
            (LoginMethod::Desktop, None) => {
                let reader = crate::app::real_desktop_reader();
                let (entry, policy) = (l.desktop_entry.clone(), opts.policy);
                let (cred, _) =
                    tokio::task::spawn_blocking(move || reader(entry.as_deref(), &policy))
                        .await?
                        .map_err(|e| anyhow!("reading the desktop session: {e}"))?;
                match cred {
                    gw::desktop::DesktopCredential::CursorToken(t) => {
                        gw::Client::connect_cursor(&t, opts).await?
                    }
                    gw::desktop::DesktopCredential::Session(s) => gw::Client::new(s, opts)?,
                }
            }
            _ => gw::Client::new(l.session, opts)?,
        }
    };
    Ok(Arc::new(GatewayApi::new(client)))
}

async fn build_ctx(
    store: Store,
    mock: bool,
    cfg: ServerConfig,
    transport: &'static str,
) -> Result<Arc<Ctx>> {
    let api: Arc<dyn ApiClient> = if mock {
        Arc::new(MockClient::new("mock"))
    } else {
        connect(&store).await?
    };
    // Validate the login (there is no whoami).
    api.list_bots()
        .await
        .map_err(|e| anyhow!("signing in: {e}"))?;
    let audit = if cfg.audit {
        Audit::open(Audit::default_path(), transport)
    } else {
        Audit::off()
    };
    let download_dir = cfg
        .download_dir
        .clone()
        .or_else(dirs::download_dir)
        .unwrap_or_else(|| store.dir.join("downloads"));
    let cap = store
        .load_config()
        .map(|c| c.download_max_mb)
        .unwrap_or(512)
        * 1024
        * 1024;
    let mut ctx = Ctx::new(api, cfg, audit, download_dir);
    ctx.download_cap = cap;
    if !mock {
        let store = Mutex::new(store);
        ctx.on_session = Some(Box::new(move |s| {
            let st = store.lock().unwrap();
            if let Ok(Some(mut l)) = st.load_login() {
                l.session = s;
                let _ = st.save_login(&l);
            }
        }));
    }
    Ok(Arc::new(ctx))
}

#[derive(Clone)]
pub struct Handler {
    ctx: Arc<Ctx>,
}

impl Handler {
    pub fn new(ctx: Arc<Ctx>) -> Self {
        Self { ctx }
    }
}

const TRANSCRIPT_PREFIX: &str = "grokbot://bots/";

fn obj(v: Value) -> Arc<JsonObject> {
    match v {
        Value::Object(m) => Arc::new(m),
        _ => Arc::new(JsonObject::new()),
    }
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig_ {
        ServerConfig_::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "grokbot-tui",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Talk to the user's Grok Bots: list_bots, then send_message + wait_for_reply. \
                 Cards (questions, approvals, secret requests, forms, drafts) are listed by \
                 list_pending_cards and answered with answer_card.",
        )
    }

    async fn list_tools(
        &self,
        _r: Option<PaginatedRequestParams>,
        _c: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = tools::tool_defs()
            .into_iter()
            .map(|t| {
                let mut tool = Tool::new(t.name, t.description, obj(t.schema));
                let mut ann = ToolAnnotations::new();
                ann.read_only_hint = Some(!t.writes);
                ann.open_world_hint = Some(true);
                tool.annotations = Some(ann);
                tool
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        r: CallToolRequestParams,
        _c: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = r.arguments.map(Value::Object).unwrap_or(Value::Null);
        Ok(match self.ctx.call(&r.name, &args).await {
            Ok(v) => CallToolResult::structured(v),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e.0)]),
        }
        .into())
    }

    async fn list_resources(
        &self,
        _r: Option<PaginatedRequestParams>,
        _c: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut v = vec![{
            let mut r = Resource::new("grokbot://bots", "bots");
            r.description = Some("The bots this server may use (JSON)".into());
            r.mime_type = Some("application/json".into());
            r
        }];
        for b in self.ctx.allowed_bots().await {
            let mut r = Resource::new(
                format!("{TRANSCRIPT_PREFIX}{}/transcript", b.id),
                format!("{} transcript", b.name),
            );
            r.mime_type = Some("text/plain".into());
            v.push(r);
        }
        Ok(ListResourcesResult::with_all_items(v))
    }

    async fn list_resource_templates(
        &self,
        _r: Option<PaginatedRequestParams>,
        _c: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let mut t = ResourceTemplate::new("grokbot://bots/{id}/transcript", "transcript");
        t.description = Some("A bot's recent chat (last 100 messages, plain text)".into());
        t.mime_type = Some("text/plain".into());
        Ok(ListResourceTemplatesResult::with_all_items(vec![t]))
    }

    async fn read_resource(
        &self,
        r: ReadResourceRequestParams,
        _c: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = r.uri;
        let text = if uri == "grokbot://bots" {
            self.ctx.resource_bots().await
        } else if let Some(id) = uri
            .strip_prefix(TRANSCRIPT_PREFIX)
            .and_then(|s| s.strip_suffix("/transcript"))
        {
            self.ctx.resource_transcript(id).await
        } else {
            return Err(ErrorData::resource_not_found(
                format!("unknown resource {uri}"),
                None,
            ));
        }
        .map_err(|e| ErrorData::internal_error(e.0, None))?;
        let mime = if uri == "grokbot://bots" {
            "application/json"
        } else {
            "text/plain"
        };
        Ok(
            ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                uri,
                mime_type: Some(mime.into()),
                text,
                meta: None,
            }])
            .into(),
        )
    }
}

type ServerConfig_ = rmcp::model::ServerConfig;

async fn serve_stdio(ctx: Arc<Ctx>) -> Result<()> {
    eprintln!(
        "grokbot-tui {}: MCP server on stdio",
        env!("CARGO_PKG_VERSION")
    );
    let svc = Handler::new(ctx)
        .serve(rmcp::transport::stdio())
        .await
        .context("starting the MCP stdio server")?;
    svc.waiting().await?;
    Ok(())
}

async fn serve_http(
    ctx: Arc<Ctx>,
    addr: std::net::SocketAddr,
    dir: &std::path::Path,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    let token = Arc::new(config::token(dir, false)?);
    let service: StreamableHttpService<Handler, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(Handler::new(ctx.clone())),
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    let app =
        axum::Router::new()
            .nest_service(HTTP_PATH, service)
            .layer(axum::middleware::from_fn(
                move |req: axum::extract::Request, next: axum::middleware::Next| {
                    let token = token.clone();
                    async move {
                        let ok = req
                            .headers()
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.strip_prefix("Bearer "))
                            .is_some_and(|t| config::token_eq(t.trim(), &token));
                        if ok {
                            next.run(req).await
                        } else {
                            axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::UNAUTHORIZED,
                                [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
                                "missing or wrong bearer token (see `grokbot-tui mcp token`)\n",
                            ))
                        }
                    }
                },
            ));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let local = listener.local_addr()?;
    eprintln!(
        "grokbot-tui {}: MCP server on http://{local}{HTTP_PATH} (bearer token: `grokbot-tui mcp token`)",
        env!("CARGO_PKG_VERSION")
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Used by tests: a mock-backed handler.
#[cfg(test)]
pub fn mock_handler() -> Handler {
    Handler::new(Arc::new(Ctx::new(
        Arc::new(MockClient::new("mock")),
        ServerConfig::default(),
        Audit::off(),
        std::env::temp_dir(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cli_parses_serve_flags() {
        use clap::Parser;
        #[derive(clap::Parser)]
        struct T {
            #[command(subcommand)]
            c: McpCmd,
        }
        let p = |a: &[&str]| {
            T::try_parse_from(std::iter::once("x").chain(a.iter().copied())).map(|t| t.c)
        };
        assert_eq!(
            p(&["serve"]).unwrap(),
            McpCmd::Serve {
                stdio: false,
                http: None,
                read_only: false
            }
        );
        assert_eq!(
            p(&["serve", "--http"]).unwrap(),
            McpCmd::Serve {
                stdio: false,
                http: Some(String::new()),
                read_only: false
            }
        );
        assert_eq!(
            p(&["serve", "--http", "127.0.0.1:9"]).unwrap(),
            McpCmd::Serve {
                stdio: false,
                http: Some("127.0.0.1:9".into()),
                read_only: false
            }
        );
        assert!(p(&["serve", "--stdio", "--http"]).is_err());
        assert_eq!(
            p(&["token", "--rotate"]).unwrap(),
            McpCmd::Token { rotate: true }
        );
    }

    #[tokio::test]
    async fn handler_lists_tools_and_resources_over_a_duplex() {
        use rmcp::ServiceExt;
        let (a, b) = tokio::io::duplex(1 << 16);
        let server =
            tokio::spawn(async move { mock_handler().serve(a).await.unwrap().waiting().await });
        let client = ().serve(b).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 9);
        let res = client.list_all_resources().await.unwrap();
        assert!(res.iter().any(|r| r.uri == "grokbot://bots"));
        let r = client
            .call_tool(
                CallToolRequestParams::new("get_bot")
                    .with_arguments(json!({"bot": "helper"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(r.structured_content.unwrap()["name"], "Helper");
        client.cancel().await.unwrap();
        let _ = server.await;
    }
}
