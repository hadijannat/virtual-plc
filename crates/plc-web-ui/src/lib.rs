//! Control-plane web UI and REST API for PLC monitoring.
//!
//! This crate provides:
//! - HTTP REST API for reading PLC state and metrics
//! - WebSocket endpoint for real-time I/O state streaming
//! - Static file serving for the dashboard UI
//!
//! # Usage
//!
//! ```ignore
//! use plc_web_ui::{WebUiConfig, WebUiServer};
//!
//! let config = WebUiConfig::default();
//! let server = WebUiServer::new(config);
//!
//! // Connect to scheduler state
//! server.set_state_provider(state_provider);
//!
//! // Start the server
//! server.start().await?;
//! ```

mod api;
mod dashboard;
mod metrics;
mod state;
mod websocket;

pub use api::*;
pub use dashboard::*;
pub use metrics::*;
pub use state::*;
pub use websocket::*;

use axum::{
    routing::{get, Router},
    Extension,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

/// Configuration for the web UI server.
#[derive(Debug, Clone)]
pub struct WebUiConfig {
    /// Address to bind the server to.
    pub bind_addr: SocketAddr,
    /// Enable CORS for development.
    pub enable_cors: bool,
    /// Path to static files directory (optional).
    pub static_dir: Option<String>,
    /// WebSocket broadcast channel capacity.
    pub ws_channel_capacity: usize,
}

impl Default for WebUiConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".parse().expect("valid default address"),
            enable_cors: true,
            static_dir: None,
            ws_channel_capacity: 256,
        }
    }
}

/// Web UI server for PLC monitoring.
pub struct WebUiServer {
    config: WebUiConfig,
    state: Arc<SharedState>,
    broadcast_tx: broadcast::Sender<StateUpdate>,
    metrics: Arc<PlcMetrics>,
}

impl WebUiServer {
    /// Create a new web UI server with the given configuration.
    pub fn new(config: WebUiConfig) -> Self {
        let (broadcast_tx, _) = broadcast::channel(config.ws_channel_capacity);
        Self {
            config,
            state: Arc::new(SharedState::default()),
            broadcast_tx,
            metrics: Arc::new(PlcMetrics::new()),
        }
    }

    /// Get a handle to update the shared state.
    ///
    /// Call this to get a `StateUpdater` that can be used to push
    /// state updates from the runtime to connected WebSocket clients.
    pub fn state_updater(&self) -> StateUpdater {
        StateUpdater {
            state: Arc::clone(&self.state),
            broadcast_tx: self.broadcast_tx.clone(),
        }
    }

    /// Get a reference to the Prometheus metrics.
    ///
    /// Use this to update metrics from the runtime.
    pub fn metrics(&self) -> Arc<PlcMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Start the web UI server.
    ///
    /// This is an async function that runs until cancelled.
    pub async fn start(self) -> anyhow::Result<()> {
        let bind_addr = self.config.bind_addr;
        info!(addr = %bind_addr, "Starting web UI server");

        let app = self.build_router();

        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        info!(addr = %bind_addr, "Web UI server listening");

        axum::serve(listener, app).await?;

        Ok(())
    }

    /// Build the axum router with all routes.
    fn build_router(self) -> Router {
        let state = Arc::clone(&self.state);
        let broadcast_tx = self.broadcast_tx.clone();
        let prom_metrics = Arc::clone(&self.metrics);
        let static_dir = self.config.static_dir.clone();
        let enable_cors = self.config.enable_cors;

        let mut app = Router::new()
            // Dashboard (root and /dashboard)
            .route("/", get(dashboard::dashboard_handler))
            .route("/dashboard", get(dashboard::dashboard_handler))
            // Health check
            .route("/health", get(api::health_check))
            // REST API routes
            .route("/api/state", get(api::get_state))
            .route("/api/metrics", get(api::get_metrics))
            .route("/api/io", get(api::get_io_state))
            .route("/api/faults", get(api::get_faults))
            // Prometheus metrics endpoint
            .route("/metrics", get(metrics::metrics_handler))
            // WebSocket endpoint
            .route("/ws", get(websocket::ws_handler))
            // Extensions
            .layer(Extension(state))
            .layer(Extension(broadcast_tx))
            .layer(Extension(prom_metrics));

        // Add CORS layer if enabled
        if enable_cors {
            let cors = CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any);
            app = app.layer(cors);
        }

        // Serve static files if configured
        if let Some(ref dir) = static_dir {
            info!(path = %dir, "Serving static files");
            // Note: tower-http ServeDir would go here
            // For now, we just log that static serving is requested
            warn!("Static file serving not yet implemented");
        }

        app
    }
}

/// Legacy function for backwards compatibility.
///
/// Creates and starts a web UI server with default configuration.
pub async fn start_server() -> anyhow::Result<()> {
    let config = WebUiConfig::default();
    let server = WebUiServer::new(config);
    server.start().await
}
