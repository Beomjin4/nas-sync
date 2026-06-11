mod auth;
mod config;
mod db;
mod error;
mod routes;
mod storage;
mod trash;

use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,obsidian_nas_server=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = config::load()?;
    tracing::info!(bind = %cfg.bind, data_dir = %cfg.data_dir.display(), "starting server");

    let pool = db::init(&cfg).await?;
    let storage = storage::Storage::new(
        cfg.vault_path.clone(),
        cfg.trash_path.clone(),
        cfg.conflicts_path.clone(),
    );
    storage.ensure_dirs().await?;

    let sync_tx = routes::sync::channel();

    let state = routes::AppState {
        pool,
        storage,
        cfg: cfg.clone(),
        sync_tx,
        admin_sessions: Default::default(),
    };
    trash::spawn_ttl_task(state.clone());

    let app = routes::router(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, app).await?;

    Ok(())
}
