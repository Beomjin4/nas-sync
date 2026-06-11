//! Trash restore / purge and the TTL cleanup background task.

use chrono::Utc;
use serde_json::json;

use crate::{
    error::{AppError, AppResult},
    routes::{sync::SyncEvent, AppState},
};

/// Move a trashed file back to its original vault path.
/// Fails if the original path is occupied again.
pub async fn restore(state: &AppState, trash_id: &str) -> AppResult<String> {
    let row = sqlx::query_as::<_, (String, String, i64, String)>(
        "SELECT original_path, stored_path, size_bytes, etag \
         FROM trash WHERE id = ? AND restored_at IS NULL",
    )
    .bind(trash_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (original_path, stored_rel, size, etag) = row;

    let occupied: Option<(String,)> =
        sqlx::query_as("SELECT etag FROM files WHERE path = ?")
            .bind(&original_path)
            .fetch_optional(&state.pool)
            .await?;
    if occupied.is_some() {
        return Err(AppError::BadRequest(format!(
            "path '{original_path}' already exists in the vault; delete or rename it first"
        )));
    }

    let stored_abs = state.storage.trash_target(&stored_rel)?;
    let (vault_abs, canon) = state.storage.resolve_vault(&original_path)?;
    if let Some(parent) = vault_abs.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(&stored_abs, &vault_abs).await?;

    let now = Utc::now().to_rfc3339();
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO files (path, etag, size_bytes, modified_at, modified_by, is_binary) \
         VALUES (?, ?, ?, ?, NULL, 0)",
    )
    .bind(&canon)
    .bind(&etag)
    .bind(size)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE trash SET restored_at = ? WHERE id = ?")
        .bind(&now)
        .bind(trash_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO audit (ts, op, path, device_id, etag_before, etag_after, size_bytes, extra) \
         VALUES (?, 'restore', ?, NULL, NULL, ?, ?, ?)",
    )
    .bind(&now)
    .bind(&canon)
    .bind(&etag)
    .bind(size)
    .bind(json!({"trash_id": trash_id}).to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Empty origin → every connected device pulls the restored file.
    let _ = state.sync_tx.send(SyncEvent::FileChanged {
        path: canon.clone(),
        etag,
        size,
        origin_device: String::new(),
    });

    Ok(canon)
}

/// Permanently delete a trash entry (file + row).
pub async fn purge(state: &AppState, trash_id: &str) -> AppResult<()> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT original_path, stored_path FROM trash WHERE id = ?",
    )
    .bind(trash_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (original_path, stored_rel) = row;

    let stored_abs = state.storage.trash_target(&stored_rel)?;
    let _ = tokio::fs::remove_file(&stored_abs).await; // already-gone is fine
    if let Some(parent) = stored_abs.parent() {
        let _ = tokio::fs::remove_dir(parent).await;
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query("DELETE FROM trash WHERE id = ?")
        .bind(trash_id)
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "INSERT INTO audit (ts, op, path, device_id, etag_before, etag_after, size_bytes, extra) \
         VALUES (?, 'trash_purged', ?, NULL, NULL, NULL, NULL, ?)",
    )
    .bind(&now)
    .bind(&original_path)
    .bind(json!({"trash_id": trash_id}).to_string())
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Background task: purge expired, non-restored trash entries periodically.
pub fn spawn_ttl_task(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
        loop {
            tick.tick().await;
            let now = Utc::now().to_rfc3339();
            let expired: Vec<(String,)> = match sqlx::query_as(
                "SELECT id FROM trash WHERE restored_at IS NULL AND expires_at < ?",
            )
            .bind(&now)
            .fetch_all(&state.pool)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::error!(error = ?e, "ttl scan failed");
                    continue;
                }
            };
            for (id,) in expired {
                if let Err(e) = purge(&state, &id).await {
                    tracing::error!(trash_id = %id, error = ?e, "ttl purge failed");
                } else {
                    tracing::info!(trash_id = %id, "ttl purge");
                }
            }
        }
    });
}
