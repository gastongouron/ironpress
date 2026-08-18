//! `ironpress-server` — a small HTTP wrapper around the ironpress HTML-to-PDF
//! engine, with a Gotenberg-compatible multipart form API.
//!
//! Routes:
//! - `POST /convert/html` — convert `index.html` (+ optional header/footer/asset
//!   files and rendering options) to a PDF.
//! - `GET /health` — liveness probe.
//! - `GET /version` — ironpress and server versions.
//!
//! Security-relevant settings (HTML sanitization, body-size limit) are fixed at
//! startup via environment variables and cannot be overridden per request. See
//! [`config::ServerConfig`].

mod config;
mod error;
mod form;
mod handlers;
mod header_footer;
mod params;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use config::ServerConfig;

#[tokio::main]
async fn main() {
    // Self-contained container HEALTHCHECK: `ironpress-server --health` exits 0
    // if the listener accepts a TCP connection, 1 otherwise. Avoids needing curl
    // or wget in the runtime image.
    if std::env::args().any(|a| a == "--health") {
        let port: u16 = std::env::var("IRONPRESS_PORT")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(3000);
        let ok = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
        std::process::exit(if ok { 0 } else { 1 });
    }

    let config = match ServerConfig::from_env() {
        Ok(config) => Arc::new(config),
        Err(e) => {
            eprintln!("ironpress-server: invalid configuration: {e}");
            std::process::exit(1);
        }
    };

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/version", get(handlers::version))
        .route("/convert/html", post(handlers::convert_html))
        .layer(DefaultBodyLimit::max(config.max_body_bytes))
        .with_state(config.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("ironpress-server: failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "ironpress-server {} listening on http://{addr} (sanitize={}, max_body={} bytes, remote={})",
        env!("CARGO_PKG_VERSION"),
        config.sanitize,
        config.max_body_bytes,
        if config.remote_enabled {
            "enabled (policy from IRONPRESS_REMOTE_* env)"
        } else {
            "disabled (remote fetches blocked)"
        },
    );

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("ironpress-server: server error: {e}");
        std::process::exit(1);
    }
}
