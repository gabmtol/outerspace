//! The MCP server agents talk to (docs/PLANO.md §2.5, §5): Streamable HTTP on 127.0.0.1 behind a
//! bearer token, hosted inside the tray app so it shares the app's [`PowerController`].

mod log;
mod tools;

use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use outerspace_core::apps::Keeper;
use outerspace_core::{PowerController, PowerState, paths};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
pub use tokio_util::sync::CancellationToken;

pub use tools::OuterSpace;

/// Serves `http://127.0.0.1:{port}/mcp` (plus an unauthenticated `/health`) until `cancel` fires.
pub async fn run(
    power: Arc<PowerController>,
    port: u16,
    token: String,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let log = Arc::new(log::ActionLog::open(&paths::logs_dir()));
    let keeper = Arc::new(Keeper::start({
        let power = power.clone();
        move || match power.state() {
            PowerState::On { display } => Some(display.rect()),
            _ => None,
        }
    }));
    // Loopback Host headers only (the default) and no browser Origins at all: web pages must not be
    // able to drive the user's machine through this port.
    let mut config = StreamableHttpServerConfig::default().enforce_origin_validation();
    config.cancellation_token = cancel.child_token();
    let last_clicked = Arc::default();
    let service = StreamableHttpService::new(
        move || Ok(OuterSpace::new(power.clone(), log.clone(), keeper.clone(), Arc::clone(&last_clicked))),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let mcp = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(Arc::new(token), require_token));
    let app = Router::new().route("/health", get(|| async { "ok" })).merge(mcp);

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    tracing::info!(port, "MCP server listening");
    axum::serve(listener, app).with_graceful_shutdown(async move { cancel.cancelled_owned().await }).await
}

async fn require_token(State(token): State<Arc<String>>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if presented == Some(token.as_str()) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "missing or wrong bearer token").into_response()
    }
}
