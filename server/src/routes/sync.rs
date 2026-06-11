//! Real-time sync events over WebSocket.
//!
//! `GET /sync` (upgrade) — connect with `Authorization: Bearer <jwt>`.
//! The server fans out `SyncEvent`s to every connected device except the
//! one that originated the event.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{auth::AuthDevice, routes::AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    FileChanged {
        path: String,
        etag: String,
        size: i64,
        origin_device: String,
    },
    FileDeleted {
        path: String,
        origin_device: String,
    },
    FileConflict {
        path: String,
        active_etag: String,
        losing_etag: String,
        conflict_id: String,
        origin_device: String,
    },
}

impl SyncEvent {
    pub fn origin(&self) -> &str {
        match self {
            SyncEvent::FileChanged { origin_device, .. } => origin_device,
            SyncEvent::FileDeleted { origin_device, .. } => origin_device,
            SyncEvent::FileConflict { origin_device, .. } => origin_device,
        }
    }
}

pub fn channel() -> broadcast::Sender<SyncEvent> {
    let (tx, _rx) = broadcast::channel(256);
    tx
}

pub fn router() -> Router<AppState> {
    Router::new().route("/sync", get(ws_handler))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    auth: AuthDevice,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, auth, state))
}

async fn handle_socket(socket: WebSocket, auth: AuthDevice, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.sync_tx.subscribe();

    tracing::info!(device_id = %auth.id, name = %auth.name, "ws connected");

    // Initial hello so the client can confirm the upgrade succeeded.
    let hello = serde_json::json!({
        "type": "hello",
        "device_id": auth.id,
    });
    if sender
        .send(Message::Text(hello.to_string()))
        .await
        .is_err()
    {
        return;
    }

    let device_id = auth.id.clone();

    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if event.origin() == device_id {
                        continue; // skip echo to origin
                    }
                    let payload = match serde_json::to_string(&event) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if sender.send(Message::Text(payload)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "ws subscriber lagged");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Close(_)) => break,
                Ok(_) => continue, // ping handled by axum; client messages ignored for now
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }

    tracing::info!(device_id = %auth.id, "ws disconnected");
}
