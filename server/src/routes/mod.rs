use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;

use crate::{config::Config, storage::Storage};

pub mod admin;
pub mod conflicts;
pub mod devices;
pub mod file;
pub mod health;
pub mod sync;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub storage: Storage,
    pub cfg: Config,
    pub sync_tx: broadcast::Sender<sync::SyncEvent>,
    pub admin_sessions: Arc<Mutex<HashSet<String>>>,
}

pub fn router(state: AppState) -> Router {
    let max_body = (state.cfg.max_file_size_mb as usize) * 1024 * 1024;

    Router::new()
        .route("/health", get(health::health))
        .route("/devices/pair", post(devices::pair))
        .route("/files", get(file::list_files))
        .nest("/file", file::router(max_body))
        .nest("/conflicts", conflicts::router())
        .nest("/admin", admin::router())
        .merge(sync::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
