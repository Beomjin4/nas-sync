//! Conflict inspection and resolution.
//!
//! - `GET  /conflicts`            — unresolved conflicts
//! - `GET  /conflicts/:id/file`   — body of the losing version
//! - `POST /conflicts/:id/resolve { choice }` where choice is one of
//!   `keep_active` | `use_other` | `keep_both`
//!
//! Resolution semantics (B+ policy):
//! - keep_active: discard the preserved losing version.
//! - use_other:   the losing version becomes active again (normal modify,
//!                broadcast to all other devices).
//! - keep_both:   losing version is written next to the original as
//!                `<stem> (conflict-<device>-<ts>).<ext>`; active unchanged.

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::AuthDevice,
    error::{AppError, AppResult},
    routes::{sync::SyncEvent, AppState},
    storage::etag_of,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/:id/file", get(losing_file))
        .route("/:id/resolve", post(resolve))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct ConflictRow {
    id: String,
    path: String,
    active_etag: String,
    losing_etag: String,
    losing_device: Option<String>,
    detected_at: String,
}

async fn list(
    State(state): State<AppState>,
    _auth: AuthDevice,
) -> AppResult<Json<Vec<ConflictRow>>> {
    let rows = sqlx::query_as::<_, ConflictRow>(
        "SELECT id, path, active_etag, losing_etag, losing_device, detected_at \
         FROM conflicts WHERE resolved_at IS NULL ORDER BY detected_at DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

async fn losing_file(
    State(state): State<AppState>,
    _auth: AuthDevice,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Response> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT stored_path, losing_etag FROM conflicts WHERE id = ? AND resolved_at IS NULL",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let abs = state.storage.conflicts_target(&row.0)?;
    let bytes = tokio::fs::read(&abs).await?;
    let mut resp = bytes.into_response();
    resp.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", row.1)).unwrap(),
    );
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(resp)
}

#[derive(Debug, Deserialize)]
struct ResolveRequest {
    choice: String,
}

async fn resolve(
    State(state): State<AppState>,
    auth: AuthDevice,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<ResolveRequest>,
) -> AppResult<Response> {
    let extra = apply_resolution(&state, &id, &req.choice, Some(&auth.id)).await?;
    Ok((
        StatusCode::OK,
        Json(json!({"resolved": id, "choice": req.choice, "detail": extra})),
    )
        .into_response())
}

/// Shared resolution engine, called from the REST handler (actor = device id)
/// and the admin console (actor = None → broadcasts reach every device).
pub async fn apply_resolution(
    state: &AppState,
    id: &str,
    choice: &str,
    actor: Option<&str>,
) -> AppResult<serde_json::Value> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT path, stored_path, losing_etag, losing_device \
         FROM conflicts WHERE id = ? AND resolved_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let (path, stored_rel, losing_etag, losing_device) = row;

    let stored_abs = state.storage.conflicts_target(&stored_rel)?;
    let now = Utc::now().to_rfc3339();

    let extra: serde_json::Value = match choice {
        "keep_active" => {
            // Nothing changes in the vault; just discard the preserved copy.
            json!({})
        }

        "use_other" => {
            let bytes = tokio::fs::read(&stored_abs).await?;
            let new_etag = etag_of(&bytes);
            debug_assert_eq!(new_etag, losing_etag);
            let (vault_abs, canon) = state.storage.resolve_vault(&path)?;
            if let Some(parent) = vault_abs.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let tmp = vault_abs.with_extension("part");
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &vault_abs).await?;

            sqlx::query(
                "UPDATE files SET etag = ?, size_bytes = ?, modified_at = ?, modified_by = ? \
                 WHERE path = ?",
            )
            .bind(&new_etag)
            .bind(bytes.len() as i64)
            .bind(&now)
            .bind(actor)
            .bind(&canon)
            .execute(&state.pool)
            .await?;

            let _ = state.sync_tx.send(SyncEvent::FileChanged {
                path: canon.clone(),
                etag: new_etag.clone(),
                size: bytes.len() as i64,
                origin_device: actor.unwrap_or("").to_string(),
            });
            json!({"restored_etag": new_etag})
        }

        "keep_both" => {
            let bytes = tokio::fs::read(&stored_abs).await?;
            // Prefer the human-readable device name over its UUID for the filename.
            let device_name: Option<(String,)> = match &losing_device {
                Some(dev_id) => {
                    sqlx::query_as("SELECT name FROM devices WHERE id = ?")
                        .bind(dev_id)
                        .fetch_optional(&state.pool)
                        .await?
                }
                None => None,
            };
            let device_label = device_name
                .as_ref()
                .map(|(n,)| sanitize_label(n))
                .unwrap_or_else(|| "unknown".to_string());
            let device_label = device_label.as_str();
            let ts_label = Utc::now().format("%Y%m%d-%H%M");
            let copy_path = conflict_copy_name(&path, device_label, &ts_label.to_string());

            let (copy_abs, copy_canon) = state.storage.resolve_vault(&copy_path)?;
            if let Some(parent) = copy_abs.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let tmp = copy_abs.with_extension("part");
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &copy_abs).await?;

            sqlx::query(
                "INSERT INTO files (path, etag, size_bytes, modified_at, modified_by, is_binary) \
                 VALUES (?, ?, ?, ?, ?, 0) \
                 ON CONFLICT(path) DO UPDATE SET etag = excluded.etag, \
                    size_bytes = excluded.size_bytes, modified_at = excluded.modified_at, \
                    modified_by = excluded.modified_by",
            )
            .bind(&copy_canon)
            .bind(&losing_etag)
            .bind(bytes.len() as i64)
            .bind(&now)
            .bind(actor)
            .execute(&state.pool)
            .await?;

            let _ = state.sync_tx.send(SyncEvent::FileChanged {
                path: copy_canon.clone(),
                etag: losing_etag.clone(),
                size: bytes.len() as i64,
                origin_device: String::new(), // deliver to every device, including resolver
            });
            json!({"copy_path": copy_canon})
        }

        other => {
            return Err(AppError::BadRequest(format!(
                "unknown choice: {other} (expected keep_active | use_other | keep_both)"
            )))
        }
    };

    sqlx::query(
        "UPDATE conflicts SET resolved_at = ?, resolution = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(choice)
    .bind(id)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO audit (ts, op, path, device_id, etag_before, etag_after, size_bytes, extra) \
         VALUES (?, 'conflict_resolved', ?, ?, NULL, NULL, NULL, ?)",
    )
    .bind(&now)
    .bind(&path)
    .bind(actor)
    .bind(json!({"conflict_id": id, "choice": choice, "detail": extra}).to_string())
    .execute(&state.pool)
    .await?;

    // Best-effort cleanup of the preserved copy (and its parent dir).
    let _ = tokio::fs::remove_file(&stored_abs).await;
    if let Some(parent) = stored_abs.parent() {
        let _ = tokio::fs::remove_dir(parent).await;
    }

    Ok(extra)
}

/// Keep device labels filesystem-safe inside the copy filename.
fn sanitize_label(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    cleaned.chars().take(24).collect()
}

/// `notes/foo.md` + device `abc` → `notes/foo (conflict-abc-20260610-1530).md`
fn conflict_copy_name(path: &str, device: &str, ts: &str) -> String {
    match path.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !stem.ends_with('/') => {
            format!("{stem} (conflict-{device}-{ts}).{ext}")
        }
        _ => format!("{path} (conflict-{device}-{ts})"),
    }
}

#[cfg(test)]
mod tests {
    use super::conflict_copy_name;

    #[test]
    fn copy_name_with_extension() {
        assert_eq!(
            conflict_copy_name("notes/foo.md", "dev", "20260101-0000"),
            "notes/foo (conflict-dev-20260101-0000).md"
        );
    }

    #[test]
    fn copy_name_without_extension() {
        assert_eq!(
            conflict_copy_name("notes/foo", "dev", "20260101-0000"),
            "notes/foo (conflict-dev-20260101-0000)"
        );
    }
}
