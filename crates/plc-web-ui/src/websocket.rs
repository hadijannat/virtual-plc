//! WebSocket handler for real-time state streaming.

use crate::state::{SharedState, StateUpdate};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// WebSocket upgrade handler.
///
/// GET /ws
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(state): Extension<Arc<SharedState>>,
    Extension(broadcast_tx): Extension<broadcast::Sender<StateUpdate>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, broadcast_tx))
}

/// Handle an individual WebSocket connection.
async fn handle_socket(
    socket: WebSocket,
    state: Arc<SharedState>,
    broadcast_tx: broadcast::Sender<StateUpdate>,
) {
    info!("WebSocket client connected");

    let (mut sender, mut receiver) = socket.split();

    // Subscribe to state updates
    let mut broadcast_rx = broadcast_tx.subscribe();

    // Send initial full state snapshot
    let initial_snapshot = state.snapshot();
    let initial_msg = StateUpdate::Full(initial_snapshot);
    if let Ok(json) = serde_json::to_string(&initial_msg) {
        if sender.send(Message::Text(json.into())).await.is_err() {
            warn!("Failed to send initial state to WebSocket client");
            return;
        }
    }

    // Spawn task to forward broadcast messages to WebSocket
    let send_task = tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(update) => {
                    if let Ok(json) = serde_json::to_string(&update) {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, "WebSocket client lagged, dropped messages");
                    // Continue - client will get next update
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    // Handle incoming messages from client (for commands, etc.)
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    debug!(text = %text, "Received WebSocket message");
                    // Could handle client commands here
                    // For now, we just log them
                }
                Ok(Message::Ping(data)) => {
                    debug!("Received WebSocket ping");
                    // Pong is automatically sent by axum
                    let _ = data; // Silence unused warning
                }
                Ok(Message::Pong(_)) => {
                    // Ignore pong responses
                }
                Ok(Message::Close(_)) => {
                    debug!("WebSocket client sent close");
                    break;
                }
                Ok(Message::Binary(_)) => {
                    // We don't expect binary messages
                    debug!("Received unexpected binary WebSocket message");
                }
                Err(e) => {
                    warn!(error = %e, "WebSocket receive error");
                    break;
                }
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {
            debug!("WebSocket send task ended");
        }
        _ = recv_task => {
            debug!("WebSocket receive task ended");
        }
    }

    info!("WebSocket client disconnected");
}
