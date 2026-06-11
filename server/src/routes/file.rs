//! File API.
//!
//! Routes:
//! - `GET    /file/*path`  — return body + ETag header
//! - `PUT    /file/*path`  — upload with optional `If-Match` ETag
//! - `DELETE /file/*path`  — soft-delete (requires matching `If-Match`)
//!
//! Conflict policy (B+):
//! - PUT with mismatched `If-Match` does NOT reject. The incoming body becomes
//!   the new active version; the previous active is moved to `conflicts/<id>/...`
//!   and a row is inserted in the `conflicts` table. The response is 200 with
//!   `{ conflict: {...} }` so the client can surface it.
//! - DELETE with mismatched/absent `If-Match` is rejected (modify-wins). The
//!   client must GET the current version first.

use axum::{
    body::Bytes,
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde_json::{json, Value};
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

use crate::{
    auth::AuthDevice,
    error::{AppError, AppResult},
    routes::{sync::SyncEvent, AppState},
    storage::etag_of,
};

pub fn router(max_body: usize) -> Router<AppState> {
    Router::new()
        .route("/*path", get(get_file).put(put_file).delete(delete_file))
        .layer(RequestBodyLimitLayer::new(max_body))
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct FileEntry {
    pub path: String,
    pub etag: String,
    pub size_bytes: i64,
    pub modified_at: String,
}

/// `GET /files` — index of everything currently in the vault.
pub async fn list_files(
    State(state): State<AppState>,
    _auth: AuthDevice,
) -> AppResult<Json<Vec<FileEntry>>> {
    let rows = sqlx::query_as::<_, FileEntry>(
        "SELECT path, etag, size_bytes, modified_at FROM files ORDER BY path",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

fn if_match(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

// ---------- GET ----------

async fn get_file(
    State(state): State<AppState>,
    _auth: AuthDevice,
    AxumPath(path): AxumPath<String>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (abs, canon) = state.storage.resolve_vault(&path)?;

    let row = sqlx::query_as::<_, (String, i64)>(
        "SELECT etag, size_bytes FROM files WHERE path = ?",
    )
    .bind(&canon)
    .fetch_optional(&state.pool)
    .await?;

    let (etag, _size) = row.ok_or(AppError::NotFound)?;

    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if inm.trim_matches('"') == etag {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }

    let bytes = tokio::fs::read(&abs).await?;
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    h.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", etag)).unwrap(),
    );
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    Ok(resp)
}

// ---------- PUT ----------

async fn put_file(
    State(state): State<AppState>,
    auth: AuthDevice,
    AxumPath(path): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Response> {
    let (abs, canon) = state.storage.resolve_vault(&path)?;
    let device = Some(auth.id.clone());
    let if_match_val = if_match(&headers);

    let new_etag = etag_of(&body);
    let size = body.len() as i64;
    let now = Utc::now().to_rfc3339();

    let current = sqlx::query_as::<_, (String, i64)>(
        "SELECT etag, size_bytes FROM files WHERE path = ?",
    )
    .bind(&canon)
    .fetch_optional(&state.pool)
    .await?;

    match (&current, &if_match_val) {
        // Fresh create — no row, no If-Match.
        (None, None) => {
            write_active(
                &state,
                &abs,
                &canon,
                &body,
                &new_etag,
                size,
                &now,
                device.as_deref(),
                "create",
                None,
            )
            .await?;
            broadcast_changed(&state, &canon, &new_etag, size, &auth.id);
            Ok(success_response(&new_etag, None))
        }

        // Client thinks file exists, but server has no record. Stale state — reject.
        (None, Some(_)) => Err(AppError::EtagMismatch {
            current: String::new(),
        }),

        // Normal update — etag matches.
        (Some((server_etag, _)), Some(im)) if server_etag == im => {
            write_active(
                &state,
                &abs,
                &canon,
                &body,
                &new_etag,
                size,
                &now,
                device.as_deref(),
                "modify",
                Some(server_etag.clone()),
            )
            .await?;
            broadcast_changed(&state, &canon, &new_etag, size, &auth.id);
            Ok(success_response(&new_etag, None))
        }

        // CONFLICT: file exists, etag mismatches or missing.
        // Incoming write wins. Previous active is preserved under conflicts/.
        (Some((server_etag, _)), _) => {
            let conflict_id = Uuid::new_v4().to_string();
            let stored_rel = format!("{}/{}.bin", conflict_id, sanitize_for_filename(&canon));
            let stored_abs = state.storage.conflicts_target(&stored_rel)?;

            if let Some(parent) = stored_abs.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::rename(&abs, &stored_abs).await?;

            sqlx::query(
                "INSERT INTO conflicts \
                 (id, path, active_etag, losing_etag, stored_path, losing_device, detected_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&conflict_id)
            .bind(&canon)
            .bind(&new_etag)
            .bind(server_etag)
            .bind(&stored_rel)
            .bind(&device)
            .bind(&now)
            .execute(&state.pool)
            .await?;

            write_active(
                &state,
                &abs,
                &canon,
                &body,
                &new_etag,
                size,
                &now,
                device.as_deref(),
                "conflict",
                Some(server_etag.clone()),
            )
            .await?;

            // Broadcast: first the conflict notice, then the resulting active change.
            let _ = state.sync_tx.send(SyncEvent::FileConflict {
                path: canon.clone(),
                active_etag: new_etag.clone(),
                losing_etag: server_etag.clone(),
                conflict_id: conflict_id.clone(),
                origin_device: auth.id.clone(),
            });
            broadcast_changed(&state, &canon, &new_etag, size, &auth.id);

            let conflict_info = json!({
                "id": conflict_id,
                "previous_etag": server_etag,
                "stored_path": stored_rel,
            });
            Ok(success_response(&new_etag, Some(conflict_info)))
        }
    }
}

// ---------- DELETE ----------

async fn delete_file(
    State(state): State<AppState>,
    auth: AuthDevice,
    AxumPath(path): AxumPath<String>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (abs, canon) = state.storage.resolve_vault(&path)?;
    let device = Some(auth.id.clone());
    let if_match_val = if_match(&headers);

    let current = sqlx::query_as::<_, (String, i64)>(
        "SELECT etag, size_bytes FROM files WHERE path = ?",
    )
    .bind(&canon)
    .fetch_optional(&state.pool)
    .await?;

    let (server_etag, size) = current.ok_or(AppError::NotFound)?;

    // Modify-wins: require If-Match equal to current.
    match if_match_val.as_deref() {
        Some(im) if im == server_etag => {}
        _ => return Err(AppError::EtagMismatch { current: server_etag }),
    }

    let trash_id = Uuid::new_v4().to_string();
    let ts = Utc::now();
    let now = ts.to_rfc3339();
    let expires_at =
        (ts + chrono::Duration::days(state.cfg.trash_ttl_days as i64)).to_rfc3339();

    let stored_rel = format!(
        "{}/{}/{}",
        ts.format("%Y-%m-%d"),
        trash_id,
        sanitize_for_filename(&canon)
    );
    let stored_abs = state.storage.trash_target(&stored_rel)?;
    if let Some(parent) = stored_abs.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(&abs, &stored_abs).await?;
    cleanup_empty_parents(&state.storage.vault, &abs).await;

    let mut tx = state.pool.begin().await?;

    sqlx::query("DELETE FROM files WHERE path = ?")
        .bind(&canon)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO trash \
         (id, original_path, stored_path, size_bytes, etag, deleted_at, deleted_by, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&trash_id)
    .bind(&canon)
    .bind(&stored_rel)
    .bind(size)
    .bind(&server_etag)
    .bind(&now)
    .bind(&device)
    .bind(&expires_at)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO audit \
         (ts, op, path, device_id, etag_before, etag_after, size_bytes, extra) \
         VALUES (?, 'delete', ?, ?, ?, NULL, ?, ?)",
    )
    .bind(&now)
    .bind(&canon)
    .bind(&device)
    .bind(&server_etag)
    .bind(size)
    .bind(json!({"trash_id": trash_id, "expires_at": expires_at}).to_string())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let _ = state.sync_tx.send(SyncEvent::FileDeleted {
        path: canon.clone(),
        origin_device: auth.id.clone(),
    });

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn broadcast_changed(
    state: &AppState,
    path: &str,
    etag: &str,
    size: i64,
    origin_device: &str,
) {
    let _ = state.sync_tx.send(SyncEvent::FileChanged {
        path: path.to_string(),
        etag: etag.to_string(),
        size,
        origin_device: origin_device.to_string(),
    });
}

// ---------- helpers ----------

async fn write_active(
    state: &AppState,
    abs: &std::path::Path,
    canon: &str,
    body: &[u8],
    new_etag: &str,
    size: i64,
    now: &str,
    device: Option<&str>,
    op: &str,
    etag_before: Option<String>,
) -> AppResult<()> {
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Atomic write: temp file, then rename.
    let tmp = abs.with_extension("part");
    tokio::fs::write(&tmp, body).await?;
    tokio::fs::rename(&tmp, abs).await?;

    let mut tx = state.pool.begin().await?;

    sqlx::query(
        "INSERT INTO files (path, etag, size_bytes, modified_at, modified_by, is_binary) \
         VALUES (?, ?, ?, ?, ?, 0) \
         ON CONFLICT(path) DO UPDATE SET \
            etag = excluded.etag, \
            size_bytes = excluded.size_bytes, \
            modified_at = excluded.modified_at, \
            modified_by = excluded.modified_by",
    )
    .bind(canon)
    .bind(new_etag)
    .bind(size)
    .bind(now)
    .bind(device)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO audit \
         (ts, op, path, device_id, etag_before, etag_after, size_bytes, extra) \
         VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(now)
    .bind(op)
    .bind(canon)
    .bind(device)
    .bind(etag_before)
    .bind(new_etag)
    .bind(size)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

fn success_response(etag: &str, conflict: Option<Value>) -> Response {
    let body = json!({
        "etag": etag,
        "conflict": conflict,
    });
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", etag)).unwrap(),
    );
    resp
}

fn sanitize_for_filename(canon: &str) -> String {
    canon.replace('/', "__")
}

/// Walk up from `removed` inside `root` and rmdir any newly-empty parents.
/// Best-effort: stops on first non-empty or any error.
async fn cleanup_empty_parents(root: &std::path::Path, removed: &std::path::Path) {
    let mut cur = removed.parent();
    while let Some(p) = cur {
        if p == root || !p.starts_with(root) {
            break;
        }
        match tokio::fs::read_dir(p).await {
            Ok(mut rd) => {
                if rd.next_entry().await.ok().flatten().is_some() {
                    break;
                }
            }
            Err(_) => break,
        }
        if tokio::fs::remove_dir(p).await.is_err() {
            break;
        }
        cur = p.parent();
    }
}
