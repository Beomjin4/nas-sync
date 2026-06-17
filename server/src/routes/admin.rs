//! Web admin console (server-rendered HTML, no JS dependencies, EN/KO).
//!
//! Enabled only when `ONS_ADMIN_PASSWORD` is set. Session = random token in
//! an in-memory set, carried by an HttpOnly cookie — restarting the server
//! logs everyone out, which is fine for a single-admin LAN tool.
//!
//! I/O discipline: every list query is LIMIT-bounded, no polling/auto-refresh,
//! stats are single-row aggregates.

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::constant_time_eq,
    error::AppResult,
    routes::{conflicts, AppState},
    trash,
};

const COOKIE: &str = "ons_admin";
const LANG_COOKIE: &str = "ons_lang";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(|| async { Redirect::to("/admin/activity") }))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
        .route("/lang/:code", get(set_lang))
        .route("/activity", get(activity))
        .route("/files", get(files_page))
        .route("/files/view", get(file_view))
        .route("/files/raw", get(file_raw))
        .route("/files/download", get(file_download))
        .route("/conflicts", get(conflicts_page))
        .route("/conflicts/:id/compare", get(conflict_compare))
        .route("/conflicts/:id/resolve", post(conflict_resolve))
        .route("/trash", get(trash_page))
        .route("/trash/:id/restore", post(trash_restore))
        .route("/trash/:id/purge", post(trash_purge))
        .route("/devices", get(devices_page))
        .route("/devices/:id/revoke", post(device_revoke))
        .route("/settings", get(settings_page))
}

// ---------- i18n ----------

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Ko,
    En,
}

fn lang_of(headers: &HeaderMap) -> Lang {
    match read_cookie(headers, LANG_COOKIE).as_deref() {
        Some("en") => Lang::En,
        _ => Lang::Ko,
    }
}

/// All UI chrome strings. Technical op names (create/modify/…) stay as-is.
fn tr(l: Lang, k: &str) -> &'static str {
    match (l, k) {
        (Lang::Ko, "activity") => "활동 로그",
        (Lang::En, "activity") => "Activity",
        (Lang::Ko, "files") => "파일",
        (Lang::En, "files") => "Files",
        (Lang::Ko, "conflicts") => "충돌",
        (Lang::En, "conflicts") => "Conflicts",
        (Lang::Ko, "trash") => "휴지통",
        (Lang::En, "trash") => "Trash",
        (Lang::Ko, "devices") => "디바이스",
        (Lang::En, "devices") => "Devices",
        (Lang::Ko, "settings") => "설정",
        (Lang::En, "settings") => "Settings",
        (Lang::Ko, "logout") => "로그아웃",
        (Lang::En, "logout") => "Logout",
        (Lang::Ko, "sign_in") => "로그인",
        (Lang::En, "sign_in") => "Sign in",
        (Lang::Ko, "password") => "비밀번호",
        (Lang::En, "password") => "password",
        (Lang::Ko, "wrong_password") => "비밀번호가 올바르지 않습니다.",
        (Lang::En, "wrong_password") => "Wrong password.",
        (Lang::Ko, "time") => "시각 (UTC)",
        (Lang::En, "time") => "time (UTC)",
        (Lang::Ko, "op") => "작업",
        (Lang::En, "op") => "op",
        (Lang::Ko, "path") => "경로",
        (Lang::En, "path") => "path",
        (Lang::Ko, "device") => "디바이스",
        (Lang::En, "device") => "device",
        (Lang::Ko, "size") => "크기",
        (Lang::En, "size") => "size",
        (Lang::Ko, "all_ops") => "전체 작업",
        (Lang::En, "all_ops") => "all ops",
        (Lang::Ko, "search_ph") => "파일 경로 검색…",
        (Lang::En, "search_ph") => "search file paths…",
        (Lang::Ko, "search") => "검색",
        (Lang::En, "search") => "Search",
        (Lang::Ko, "modified") => "수정 시각",
        (Lang::En, "modified") => "modified",
        (Lang::Ko, "modified_by") => "수정 디바이스",
        (Lang::En, "modified_by") => "modified by",
        (Lang::Ko, "download") => "다운로드",
        (Lang::En, "download") => "download",
        (Lang::Ko, "no_files") => "검색 결과가 없습니다.",
        (Lang::En, "no_files") => "No files matched.",
        (Lang::Ko, "files_shown") => "표시 (최대 100건)",
        (Lang::En, "files_shown") => "shown (max 100)",
        (Lang::Ko, "no_conflicts") => "미해결 충돌이 없습니다. 🎉",
        (Lang::En, "no_conflicts") => "No unresolved conflicts. 🎉",
        (Lang::Ko, "detected") => "감지 시각",
        (Lang::En, "detected") => "detected",
        (Lang::Ko, "losing_device") => "진 버전 디바이스",
        (Lang::En, "losing_device") => "losing device",
        (Lang::Ko, "action") => "동작",
        (Lang::En, "action") => "action",
        (Lang::Ko, "keep_active") => "현재 유지",
        (Lang::En, "keep_active") => "keep active",
        (Lang::Ko, "use_other") => "다른 버전 사용",
        (Lang::En, "use_other") => "use other",
        (Lang::Ko, "keep_both") => "둘 다 보관",
        (Lang::En, "keep_both") => "keep both",
        (Lang::Ko, "trash_empty") => "휴지통이 비어 있습니다.",
        (Lang::En, "trash_empty") => "Trash is empty.",
        (Lang::Ko, "deleted") => "삭제 시각",
        (Lang::En, "deleted") => "deleted",
        (Lang::Ko, "deleted_by") => "삭제 디바이스",
        (Lang::En, "deleted_by") => "by",
        (Lang::Ko, "expires") => "만료일",
        (Lang::En, "expires") => "expires",
        (Lang::Ko, "restore") => "복구",
        (Lang::En, "restore") => "restore",
        (Lang::Ko, "purge") => "영구 삭제",
        (Lang::En, "purge") => "purge",
        (Lang::Ko, "purge_confirm") => "영구적으로 삭제할까요?",
        (Lang::En, "purge_confirm") => "Permanently delete?",
        (Lang::Ko, "name") => "이름",
        (Lang::En, "name") => "name",
        (Lang::Ko, "platform") => "플랫폼",
        (Lang::En, "platform") => "platform",
        (Lang::Ko, "paired") => "페어링",
        (Lang::En, "paired") => "paired",
        (Lang::Ko, "last_seen") => "마지막 접속",
        (Lang::En, "last_seen") => "last seen",
        (Lang::Ko, "status") => "상태",
        (Lang::En, "status") => "status",
        (Lang::Ko, "active") => "활성",
        (Lang::En, "active") => "active",
        (Lang::Ko, "revoked") => "차단됨",
        (Lang::En, "revoked") => "revoked",
        (Lang::Ko, "revoke") => "차단",
        (Lang::En, "revoke") => "revoke",
        (Lang::Ko, "revoke_confirm") => "이 디바이스를 차단할까요? 다시 페어링해야 합니다.",
        (Lang::En, "revoke_confirm") => "Revoke this device? It must pair again.",
        (Lang::Ko, "server_config") => "서버 설정",
        (Lang::En, "server_config") => "Server configuration",
        (Lang::Ko, "vault_stats") => "보관함 상태",
        (Lang::En, "vault_stats") => "Vault status",
        (Lang::Ko, "pairing_code") => "페어링 코드",
        (Lang::En, "pairing_code") => "Pairing code",
        (Lang::Ko, "pairing_disabled") => "(미설정 — 페어링 비활성)",
        (Lang::En, "pairing_disabled") => "(unset — pairing disabled)",
        (Lang::Ko, "trash_ttl") => "휴지통 보관 기간",
        (Lang::En, "trash_ttl") => "Trash retention",
        (Lang::Ko, "days") => "일",
        (Lang::En, "days") => "days",
        (Lang::Ko, "max_file") => "파일 크기 제한",
        (Lang::En, "max_file") => "Max file size",
        (Lang::Ko, "jwt_ttl") => "토큰 유효 기간",
        (Lang::En, "jwt_ttl") => "Token lifetime",
        (Lang::Ko, "bind_addr") => "바인드 주소",
        (Lang::En, "bind_addr") => "Bind address",
        (Lang::Ko, "file_count") => "파일 수",
        (Lang::En, "file_count") => "Files",
        (Lang::Ko, "total_size") => "총 용량",
        (Lang::En, "total_size") => "Total size",
        (Lang::Ko, "db_size") => "DB 크기",
        (Lang::En, "db_size") => "DB size",
        (Lang::Ko, "active_devices") => "활성 디바이스",
        (Lang::En, "active_devices") => "Active devices",
        (Lang::Ko, "trash_count") => "휴지통 항목",
        (Lang::En, "trash_count") => "Items in trash",
        (Lang::Ko, "open_conflicts") => "미해결 충돌",
        (Lang::En, "open_conflicts") => "Open conflicts",
        (Lang::Ko, "audit_rows") => "로그 행 수",
        (Lang::En, "audit_rows") => "Audit rows",
        (Lang::Ko, "preview") => "미리보기",
        (Lang::En, "preview") => "Preview",
        (Lang::Ko, "back_to_list") => "← 목록으로",
        (Lang::En, "back_to_list") => "← Back to list",
        (Lang::Ko, "no_preview") => "이 형식은 미리보기를 지원하지 않습니다.",
        (Lang::En, "no_preview") => "Preview is not available for this file type.",
        (Lang::Ko, "too_large") => "파일이 커서 미리보기를 표시하지 않습니다 (256 KB 제한).",
        (Lang::En, "too_large") => "File too large to preview (256 KB limit).",
        (Lang::Ko, "compare") => "비교",
        (Lang::En, "compare") => "Compare",
        (Lang::Ko, "current_version") => "현재 버전 (활성)",
        (Lang::En, "current_version") => "Current version (active)",
        (Lang::Ko, "other_version") => "다른 기기 버전",
        (Lang::En, "other_version") => "Other device's version",
        (Lang::Ko, "missing_local") => "(활성 버전이 없습니다)",
        (Lang::En, "missing_local") => "(no active version)",
        (Lang::Ko, "binary_file") => "(바이너리 파일 — 미리보기 불가)",
        (Lang::En, "binary_file") => "(binary file — cannot preview)",
        (Lang::Ko, "back_to_conflicts") => "← 충돌 목록",
        (Lang::En, "back_to_conflicts") => "← Back to conflicts",
        _ => "?",
    }
}

// ---------- session ----------

fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(|s| s.trim())
        .find_map(|kv| kv.strip_prefix(&format!("{name}=") as &str))
        .map(|s| s.to_string())
}

fn authed(state: &AppState, headers: &HeaderMap) -> bool {
    match read_cookie(headers, COOKIE) {
        Some(sid) => state.admin_sessions.lock().unwrap().contains(&sid),
        None => false,
    }
}

fn require_auth(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    if state.cfg.admin_password.is_none() {
        return Err((StatusCode::NOT_FOUND, "admin console disabled").into_response());
    }
    if authed(state, headers) {
        Ok(())
    } else {
        Err(Redirect::to("/admin/login").into_response())
    }
}

async fn set_lang(
    headers: HeaderMap,
    AxumPath(code): AxumPath<String>,
) -> Response {
    let code = if code == "en" { "en" } else { "ko" };
    let back = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("/admin/activity")
        .to_string();
    (
        [(header::SET_COOKIE, format!("{LANG_COOKIE}={code}; SameSite=Lax; Path=/admin; Max-Age=31536000"))],
        Redirect::to(&back),
    )
        .into_response()
}

// ---------- login ----------

async fn login_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let l = lang_of(&headers);
    if state.cfg.admin_password.is_none() {
        return (StatusCode::NOT_FOUND, "admin console disabled (set ONS_ADMIN_PASSWORD)")
            .into_response();
    }
    Html(login_html(l, false)).into_response()
}

fn login_html(l: Lang, wrong: bool) -> String {
    let err = if wrong {
        format!(r#"<p class="err">{}</p>"#, tr(l, "wrong_password"))
    } else {
        String::new()
    };
    layout_bare(format!(
        r#"<form method="post" action="/admin/login" class="card">
            <h2>NAS Sync</h2>{err}
            <input type="password" name="password" placeholder="{}" autofocus>
            <button type="submit" class="primary">{}</button>
            <p class="lang"><a href="/admin/lang/ko">한국어</a> · <a href="/admin/lang/en">English</a></p>
        </form>"#,
        tr(l, "password"),
        tr(l, "sign_in"),
    ))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
}

async fn login_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<LoginForm>,
) -> Response {
    let l = lang_of(&headers);
    let Some(expected) = state.cfg.admin_password.as_deref() else {
        return (StatusCode::NOT_FOUND, "disabled").into_response();
    };
    if !constant_time_eq(f.password.as_bytes(), expected.as_bytes()) {
        return Html(login_html(l, true)).into_response();
    }
    let sid = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    state.admin_sessions.lock().unwrap().insert(sid.clone());
    (
        [(header::SET_COOKIE, format!("{COOKIE}={sid}; HttpOnly; SameSite=Lax; Path=/admin"))],
        Redirect::to("/admin/activity"),
    )
        .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(sid) = read_cookie(&headers, COOKIE) {
        state.admin_sessions.lock().unwrap().remove(&sid);
    }
    (
        [(header::SET_COOKIE, format!("{COOKIE}=; Max-Age=0; Path=/admin"))],
        Redirect::to("/admin/login"),
    )
        .into_response()
}

// ---------- activity ----------

#[derive(Deserialize)]
struct PageQuery {
    op: Option<String>,
    q: Option<String>,
    msg: Option<String>,
}

async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(qp): Query<PageQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);

    let op_filter = qp.op.as_deref().unwrap_or("");
    let rows: Vec<(i64, String, String, String, Option<String>, Option<i64>)> =
        if op_filter.is_empty() {
            sqlx::query_as(
                "SELECT a.id, a.ts, a.op, a.path, d.name, a.size_bytes \
                 FROM audit a LEFT JOIN devices d ON d.id = a.device_id \
                 ORDER BY a.id DESC LIMIT 200",
            )
            .fetch_all(&state.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT a.id, a.ts, a.op, a.path, d.name, a.size_bytes \
                 FROM audit a LEFT JOIN devices d ON d.id = a.device_id \
                 WHERE a.op = ? ORDER BY a.id DESC LIMIT 200",
            )
            .bind(op_filter)
            .fetch_all(&state.pool)
            .await?
        };

    let mut body = msg_html(&qp.msg);
    body.push_str(&format!(
        r#"<form method="get" class="bar"><select name="op" onchange="this.form.submit()"><option value="">{}</option>"#,
        tr(l, "all_ops")
    ));
    for op in ["create", "modify", "delete", "restore", "conflict", "conflict_resolved", "trash_purged"] {
        let sel = if op == op_filter { " selected" } else { "" };
        body.push_str(&format!(r#"<option value="{op}"{sel}>{op}</option>"#));
    }
    body.push_str(&format!(
        "</select></form><table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
        tr(l, "time"), tr(l, "op"), tr(l, "path"), tr(l, "device"), tr(l, "size"),
    ));
    for (_id, ts, op, path, device, size) in rows {
        body.push_str(&format!(
            "<tr><td>{}</td><td><span class=\"op op-{}\">{}</span></td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&ts[..19.min(ts.len())]),
            esc(&op),
            esc(&op),
            esc(&path),
            esc(device.as_deref().unwrap_or("—")),
            size.map(human_size).unwrap_or_else(|| "—".into()),
        ));
    }
    body.push_str("</table>");
    Ok(Html(layout(l, "activity", body)).into_response())
}

// ---------- files (search + download) ----------

async fn files_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(qp): Query<PageQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);
    let q = qp.q.as_deref().unwrap_or("").trim().to_string();

    let rows: Vec<(String, String, i64, String, Option<String>)> = if q.is_empty() {
        sqlx::query_as(
            "SELECT f.path, f.etag, f.size_bytes, f.modified_at, d.name \
             FROM files f LEFT JOIN devices d ON d.id = f.modified_by \
             ORDER BY f.modified_at DESC LIMIT 100",
        )
        .fetch_all(&state.pool)
        .await?
    } else {
        let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        sqlx::query_as(
            "SELECT f.path, f.etag, f.size_bytes, f.modified_at, d.name \
             FROM files f LEFT JOIN devices d ON d.id = f.modified_by \
             WHERE f.path LIKE ? ESCAPE '\\' ORDER BY f.path LIMIT 100",
        )
        .bind(&like)
        .fetch_all(&state.pool)
        .await?
    };

    let mut body = msg_html(&qp.msg);
    body.push_str(&format!(
        r#"<form method="get" class="bar"><input type="search" name="q" value="{}" placeholder="{}" class="search">
           <button type="submit" class="primary">{}</button>
           <span class="count">{} {}</span></form>"#,
        esc(&q),
        tr(l, "search_ph"),
        tr(l, "search"),
        rows.len(),
        tr(l, "files_shown"),
    ));
    if rows.is_empty() {
        body.push_str(&format!("<p>{}</p>", tr(l, "no_files")));
    } else {
        body.push_str(&format!(
            "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th></th></tr>",
            tr(l, "path"), tr(l, "size"), tr(l, "modified"), tr(l, "modified_by"),
        ));
        for (path, _etag, size, modified, device) in rows {
            body.push_str(&format!(
                r#"<tr><td><a href="/admin/files/view?path={}">{}</a></td><td>{}</td><td>{}</td><td>{}</td>
                <td><a class="btn" href="/admin/files/download?path={}">{}</a></td></tr>"#,
                urlenc(&path),
                esc(&path),
                human_size(size),
                esc(&modified[..19.min(modified.len())]),
                esc(device.as_deref().unwrap_or("—")),
                urlenc(&path),
                tr(l, "download"),
            ));
        }
        body.push_str("</table>");
    }
    Ok(Html(layout(l, "files", body)).into_response())
}

#[derive(Deserialize)]
struct DownloadQuery {
    path: String,
}

const PREVIEW_TEXT_MAX: i64 = 256 * 1024;
const TEXT_EXTS: [&str; 10] = ["md", "txt", "json", "canvas", "base", "csv", "log", "yml", "yaml", "toml"];
const IMAGE_EXTS: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
];

fn ext_of(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

/// Preview page. Costs at most one on-demand file read; oversized text is
/// rejected using the size already stored in the DB row (no disk read).
async fn file_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);
    let (_abs, canon) = state.storage.resolve_vault(&q.path)?;

    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT size_bytes, modified_at FROM files WHERE path = ?")
            .bind(&canon)
            .fetch_optional(&state.pool)
            .await?;
    let Some((size, modified)) = row else {
        return Ok((StatusCode::NOT_FOUND, "not in index").into_response());
    };

    let ext = ext_of(&canon);
    let meta = format!(
        r#"<p class="count">{} · {}</p>"#,
        human_size(size),
        esc(&modified[..19.min(modified.len())]),
    );
    let toolbar = format!(
        r#"<div class="bar"><a class="btn" href="/admin/files">{}</a>
        <a class="btn" href="/admin/files/download?path={}">{}</a></div>"#,
        tr(l, "back_to_list"),
        urlenc(&canon),
        tr(l, "download"),
    );

    let content = if IMAGE_EXTS.iter().any(|(e, _)| *e == ext) {
        format!(
            r#"<img class="preview-img" src="/admin/files/raw?path={}" alt="{}">"#,
            urlenc(&canon),
            esc(&canon),
        )
    } else if TEXT_EXTS.contains(&ext.as_str()) {
        if size > PREVIEW_TEXT_MAX {
            format!("<p>{}</p>", tr(l, "too_large"))
        } else {
            let (abs, _) = state.storage.resolve_vault(&canon)?;
            let bytes = tokio::fs::read(&abs).await?;
            match String::from_utf8(bytes) {
                Ok(text) => format!(r#"<pre class="preview-text">{}</pre>"#, esc(&text)),
                Err(_) => format!("<p>{}</p>", tr(l, "no_preview")),
            }
        }
    } else {
        format!("<p>{}</p>", tr(l, "no_preview"))
    };

    let body = format!(
        "<h3>{}</h3>{meta}{toolbar}{content}",
        esc(&canon),
    );
    Ok(Html(layout(l, "files", body)).into_response())
}

/// Inline bytes for `<img>` tags on the preview page.
async fn file_raw(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let (abs, canon) = state.storage.resolve_vault(&q.path)?;
    let ext = ext_of(&canon);
    let mime = IMAGE_EXTS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, m)| *m)
        .unwrap_or("application/octet-stream");
    let bytes = tokio::fs::read(&abs).await?;
    let mut resp = bytes.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(mime),
    );
    Ok(resp)
}

async fn file_download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let (abs, canon) = state.storage.resolve_vault(&q.path)?;
    let bytes = tokio::fs::read(&abs).await?;
    let filename = canon.rsplit('/').next().unwrap_or(&canon).to_string();
    let mut resp = bytes.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!(
            "attachment; filename*=UTF-8''{}",
            urlenc(&filename)
        ))
        .unwrap(),
    );
    Ok(resp)
}

// ---------- conflicts ----------

async fn conflicts_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(qp): Query<PageQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.path, c.detected_at, d.name \
         FROM conflicts c LEFT JOIN devices d ON d.id = c.losing_device \
         WHERE c.resolved_at IS NULL ORDER BY c.detected_at DESC LIMIT 100",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut body = msg_html(&qp.msg);
    if rows.is_empty() {
        body.push_str(&format!("<p>{}</p>", tr(l, "no_conflicts")));
    } else {
        body.push_str(&format!(
            "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
            tr(l, "path"), tr(l, "detected"), tr(l, "losing_device"), tr(l, "action"),
        ));
        for (id, path, detected, device) in rows {
            body.push_str(&format!(
                r#"<tr><td><a href="/admin/conflicts/{}/compare">{}</a></td><td>{}</td><td>{}</td><td>
                <a class="btn" href="/admin/conflicts/{}/compare">{}</a>
                <form method="post" action="/admin/conflicts/{}/resolve" class="inline">
                    <button name="choice" value="keep_active">{}</button>
                    <button name="choice" value="use_other">{}</button>
                    <button name="choice" value="keep_both">{}</button>
                </form></td></tr>"#,
                esc(&id),
                esc(&path),
                esc(&detected[..19.min(detected.len())]),
                esc(device.as_deref().unwrap_or("?")),
                esc(&id),
                tr(l, "compare"),
                esc(&id),
                tr(l, "keep_active"),
                tr(l, "use_other"),
                tr(l, "keep_both"),
            ));
        }
        body.push_str("</table>");
    }
    Ok(Html(layout(l, "conflicts", body)).into_response())
}

/// Side-by-side comparison of the active version (in the vault) and the
/// losing version (preserved under conflicts/). Each side is read once,
/// on demand — same cost as a download.
async fn conflict_compare(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);

    let row: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT c.path, c.stored_path, d.name, c.detected_at \
         FROM conflicts c LEFT JOIN devices d ON d.id = c.losing_device \
         WHERE c.id = ? AND c.resolved_at IS NULL",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((path, stored_path, device, detected)) = row else {
        return Ok((StatusCode::NOT_FOUND, "conflict not found or already resolved")
            .into_response());
    };

    // Active side: current vault file (may be absent if it was deleted).
    let active_html = match state.storage.resolve_vault(&path) {
        Ok((abs, _)) => match tokio::fs::read(&abs).await {
            Ok(bytes) => render_pane(&bytes),
            Err(_) => format!("<p class=\"muted\">{}</p>", tr(l, "missing_local")),
        },
        Err(_) => format!("<p class=\"muted\">{}</p>", tr(l, "missing_local")),
    };

    // Losing side: preserved copy under conflicts/.
    let losing_html = match state.storage.conflicts_target(&stored_path) {
        Ok(abs) => match tokio::fs::read(&abs).await {
            Ok(bytes) => render_pane(&bytes),
            Err(_) => format!("<p class=\"muted\">{}</p>", tr(l, "binary_file")),
        },
        Err(_) => format!("<p class=\"muted\">{}</p>", tr(l, "binary_file")),
    };

    let body = format!(
        r#"<div class="bar"><a class="btn" href="/admin/conflicts">{back}</a></div>
<h3>{path}</h3><p class="count">{dev} · {when}</p>
<div class="cmp">
  <div class="cmp-pane"><div class="cmp-head cmp-head-active">{cur}</div>{active}</div>
  <div class="cmp-pane"><div class="cmp-head cmp-head-other">{oth}</div>{losing}</div>
</div>
<form method="post" action="/admin/conflicts/{id}/resolve" class="cmp-actions">
  <button name="choice" value="keep_active" class="primary">{keep_active}</button>
  <button name="choice" value="use_other">{use_other}</button>
  <button name="choice" value="keep_both">{keep_both}</button>
</form>"#,
        back = tr(l, "back_to_conflicts"),
        path = esc(&path),
        dev = esc(device.as_deref().unwrap_or("?")),
        when = esc(&detected[..19.min(detected.len())]),
        cur = tr(l, "current_version"),
        oth = format!("{} ({})", tr(l, "other_version"), esc(device.as_deref().unwrap_or("?"))),
        active = active_html,
        losing = losing_html,
        id = esc(&id),
        keep_active = tr(l, "keep_active"),
        use_other = tr(l, "use_other"),
        keep_both = tr(l, "keep_both"),
    );
    Ok(Html(layout(l, "conflicts", body)).into_response())
}

/// Render one comparison pane: UTF-8 text in a <pre>, or a placeholder for
/// binary content. Caps display to avoid dumping a huge file into the page.
fn render_pane(bytes: &[u8]) -> String {
    const MAX: usize = 256 * 1024;
    if bytes.len() > MAX {
        return format!("<pre class=\"cmp-text\">({} KB)</pre>", bytes.len() / 1024);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => format!("<pre class=\"cmp-text\">{}</pre>", esc(text)),
        Err(_) => "<pre class=\"cmp-text muted\">(binary)</pre>".to_string(),
    }
}

#[derive(Deserialize)]
struct ChoiceForm {
    choice: String,
}

async fn conflict_resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Form(f): Form<ChoiceForm>,
) -> Response {
    if let Err(r) = require_auth(&state, &headers) {
        return r;
    }
    let msg = match conflicts::apply_resolution(&state, &id, &f.choice, None).await {
        Ok(_) => format!("OK: {}", f.choice),
        Err(e) => format!("error: {e}"),
    };
    Redirect::to(&format!("/admin/conflicts?msg={}", urlenc(&msg))).into_response()
}

// ---------- trash ----------

async fn trash_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(qp): Query<PageQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);
    let rows: Vec<(String, String, i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.original_path, t.size_bytes, t.deleted_at, t.expires_at, d.name \
         FROM trash t LEFT JOIN devices d ON d.id = t.deleted_by \
         WHERE t.restored_at IS NULL ORDER BY t.deleted_at DESC LIMIT 200",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut body = msg_html(&qp.msg);
    if rows.is_empty() {
        body.push_str(&format!("<p>{}</p>", tr(l, "trash_empty")));
    } else {
        body.push_str(&format!(
            "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
            tr(l, "path"), tr(l, "size"), tr(l, "deleted"), tr(l, "deleted_by"), tr(l, "expires"), tr(l, "action"),
        ));
        for (id, path, size, deleted, expires, device) in rows {
            body.push_str(&format!(
                r#"<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>
                <form method="post" action="/admin/trash/{}/restore" class="inline"><button>{}</button></form>
                <form method="post" action="/admin/trash/{}/purge" class="inline" onsubmit="return confirm('{}')"><button class="danger">{}</button></form>
                </td></tr>"#,
                esc(&path),
                human_size(size),
                esc(&deleted[..19.min(deleted.len())]),
                esc(device.as_deref().unwrap_or("—")),
                esc(&expires[..10.min(expires.len())]),
                esc(&id),
                tr(l, "restore"),
                esc(&id),
                tr(l, "purge_confirm"),
                tr(l, "purge"),
            ));
        }
        body.push_str("</table>");
    }
    Ok(Html(layout(l, "trash", body)).into_response())
}

async fn trash_restore(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if let Err(r) = require_auth(&state, &headers) {
        return r;
    }
    let msg = match trash::restore(&state, &id).await {
        Ok(path) => format!("OK: {path}"),
        Err(e) => format!("error: {e}"),
    };
    Redirect::to(&format!("/admin/trash?msg={}", urlenc(&msg))).into_response()
}

async fn trash_purge(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if let Err(r) = require_auth(&state, &headers) {
        return r;
    }
    let msg = match trash::purge(&state, &id).await {
        Ok(()) => "OK".to_string(),
        Err(e) => format!("error: {e}"),
    };
    Redirect::to(&format!("/admin/trash?msg={}", urlenc(&msg))).into_response()
}

// ---------- devices ----------

async fn devices_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(qp): Query<PageQuery>,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);
    let rows: Vec<(String, String, Option<String>, String, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, name, platform, created_at, last_seen_at, revoked_at \
             FROM devices ORDER BY created_at LIMIT 200",
        )
        .fetch_all(&state.pool)
        .await?;

    let mut body = msg_html(&qp.msg);
    body.push_str(&format!(
        "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th></th></tr>",
        tr(l, "name"), tr(l, "platform"), tr(l, "paired"), tr(l, "last_seen"), tr(l, "status"),
    ));
    for (id, name, platform, created, last_seen, revoked) in rows {
        let status = if revoked.is_some() {
            format!(r#"<span class="badge off">{}</span>"#, tr(l, "revoked"))
        } else {
            format!(r#"<span class="badge on">{}</span>"#, tr(l, "active"))
        };
        let action = if revoked.is_none() {
            format!(
                r#"<form method="post" action="/admin/devices/{}/revoke" class="inline" onsubmit="return confirm('{}')"><button class="danger">{}</button></form>"#,
                esc(&id),
                tr(l, "revoke_confirm"),
                tr(l, "revoke"),
            )
        } else {
            String::new()
        };
        body.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            esc(&name),
            esc(platform.as_deref().unwrap_or("—")),
            esc(&created[..10.min(created.len())]),
            esc(last_seen.as_deref().map(|s| &s[..19.min(s.len())]).unwrap_or("—")),
            status,
            action,
        ));
    }
    body.push_str("</table>");
    Ok(Html(layout(l, "devices", body)).into_response())
}

async fn device_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if let Err(r) = require_auth(&state, &headers) {
        return r;
    }
    let now = Utc::now().to_rfc3339();
    let msg = match sqlx::query("UPDATE devices SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(&now)
        .bind(&id)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => "OK".to_string(),
        Ok(_) => "not found / already revoked".to_string(),
        Err(e) => format!("error: {e}"),
    };
    Redirect::to(&format!("/admin/devices?msg={}", urlenc(&msg))).into_response()
}

// ---------- settings ----------

async fn settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Response> {
    if let Err(r) = require_auth(&state, &headers) {
        return Ok(r);
    }
    let l = lang_of(&headers);

    // Single-row aggregates only — negligible I/O.
    let (file_count, total_size): (i64, Option<i64>) =
        sqlx::query_as("SELECT COUNT(*), SUM(size_bytes) FROM files")
            .fetch_one(&state.pool)
            .await?;
    let (device_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL")
            .fetch_one(&state.pool)
            .await?;
    let (trash_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM trash WHERE restored_at IS NULL")
            .fetch_one(&state.pool)
            .await?;
    let (conflict_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM conflicts WHERE resolved_at IS NULL")
            .fetch_one(&state.pool)
            .await?;
    let (audit_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit")
        .fetch_one(&state.pool)
        .await?;
    let db_size = tokio::fs::metadata(&state.cfg.db_path)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let pairing = match &state.cfg.pairing_code {
        Some(c) => format!("<code>{}</code>", esc(c)),
        None => tr(l, "pairing_disabled").to_string(),
    };

    let kv = |k: &str, v: String| format!("<tr><th>{k}</th><td>{v}</td></tr>");
    let body = format!(
        r#"<h3>{}</h3><table class="kv">{}{}{}{}{}</table>
           <h3>{}</h3><table class="kv">{}{}{}{}{}{}</table>"#,
        tr(l, "server_config"),
        kv(tr(l, "pairing_code"), pairing),
        kv(tr(l, "trash_ttl"), format!("{} {}", state.cfg.trash_ttl_days, tr(l, "days"))),
        kv(tr(l, "max_file"), format!("{} MB", state.cfg.max_file_size_mb)),
        kv(tr(l, "jwt_ttl"), format!("{} {}", state.cfg.jwt_ttl_days, tr(l, "days"))),
        kv(tr(l, "bind_addr"), esc(&state.cfg.bind)),
        tr(l, "vault_stats"),
        kv(tr(l, "file_count"), file_count.to_string()),
        kv(tr(l, "total_size"), human_size(total_size.unwrap_or(0))),
        kv(tr(l, "db_size"), human_size(db_size)),
        kv(tr(l, "active_devices"), device_count.to_string()),
        kv(tr(l, "trash_count"), trash_count.to_string()),
        kv(
            tr(l, "open_conflicts"),
            format!("{conflict_count} / {} {}", audit_count, tr(l, "audit_rows")),
        ),
    );
    Ok(Html(layout(l, "settings", body)).into_response())
}

// ---------- html helpers ----------

fn msg_html(msg: &Option<String>) -> String {
    match msg {
        Some(m) => format!(r#"<p class="msg">{}</p>"#, esc(m)),
        None => String::new(),
    }
}

const NAV_ITEMS: [(&str, &str); 6] = [
    ("activity", "📋"),
    ("files", "📁"),
    ("conflicts", "⚠️"),
    ("trash", "🗑"),
    ("devices", "💻"),
    ("settings", "⚙️"),
];

fn layout(l: Lang, active: &str, body: String) -> String {
    let mut nav = String::new();
    for (slug, icon) in NAV_ITEMS {
        let cls = if slug == active { " class=\"on\"" } else { "" };
        nav.push_str(&format!(
            r#"<a href="/admin/{slug}"{cls}><span class="ico">{icon}</span><span class="lbl">{}</span></a>"#,
            tr(l, slug)
        ));
    }
    let (lang_a, lang_b) = match l {
        Lang::Ko => (r#"<b>한국어</b>"#.to_string(), r#"<a href="/admin/lang/en">EN</a>"#.to_string()),
        Lang::En => (r#"<a href="/admin/lang/ko">한국어</a>"#.to_string(), "<b>EN</b>".to_string()),
    };
    layout_bare(format!(
        r#"<div class="shell">
<aside><div class="logo">🗄 NAS Sync</div><nav>{nav}</nav>
<div class="aside-foot"><span class="lang">{lang_a} · {lang_b}</span>
<form method="post" action="/admin/logout"><button>{}</button></form></div></aside>
<main><header><h1>{}</h1></header><section>{body}</section></main></div>"#,
        tr(l, "logout"),
        tr(l, active),
    ))
}

fn layout_bare(content: String) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>NAS Sync</title>
<style>
*{{box-sizing:border-box}}
body{{font:14px/1.5 -apple-system,system-ui,'Apple SD Gothic Neo','Malgun Gothic',sans-serif;margin:0;background:#eef1f5;color:#22293a}}
.shell{{display:flex;min-height:100vh}}
aside{{width:210px;background:#252e42;color:#aab3c5;display:flex;flex-direction:column;flex-shrink:0}}
.logo{{padding:18px 16px;font-weight:700;color:#fff;font-size:15px;border-bottom:1px solid #323d56}}
aside nav{{display:flex;flex-direction:column;padding:8px;gap:2px;flex:1}}
aside nav a{{color:#aab3c5;text-decoration:none;padding:9px 12px;border-radius:8px;display:flex;align-items:center;gap:10px}}
aside nav a:hover{{background:#2e3950;color:#fff}}
aside nav a.on{{background:#3b6fde;color:#fff}}
.ico{{width:18px;text-align:center}}
.aside-foot{{padding:12px 16px;border-top:1px solid #323d56;display:flex;justify-content:space-between;align-items:center}}
.aside-foot .lang a{{color:#aab3c5}}
.aside-foot .lang b{{color:#fff}}
main{{flex:1;min-width:0}}
header{{background:#fff;border-bottom:1px solid #e2e6ee;padding:14px 28px}}
header h1{{margin:0;font-size:17px}}
section{{padding:24px 28px;max-width:1100px}}
table{{width:100%;border-collapse:collapse;background:#fff;border-radius:10px;overflow:hidden;box-shadow:0 1px 3px rgba(20,30,60,.07)}}
th,td{{text-align:left;padding:9px 14px;border-bottom:1px solid #eef0f4;word-break:break-all}}
th{{background:#f7f9fc;font-weight:600;font-size:12px;color:#5a6478}}
table.kv{{max-width:560px;margin-bottom:24px}}
table.kv th{{width:200px;background:#f7f9fc}}
h3{{margin:8px 0 10px}}
code{{background:#f0f3f8;padding:2px 8px;border-radius:6px;font-size:13px}}
button,.btn{{cursor:pointer;border:1px solid #cdd4e0;background:#fff;border-radius:7px;padding:5px 11px;margin-right:4px;font-size:13px;color:#22293a;text-decoration:none;display:inline-block}}
button:hover,.btn:hover{{background:#eef2ff;border-color:#9db4e8}}
button.primary{{background:#3b6fde;border-color:#3b6fde;color:#fff}}
button.danger{{color:#c0392b;border-color:#e4b6b0}}
.inline{{display:inline}}
.card{{max-width:330px;margin:90px auto;background:#fff;padding:28px;border-radius:12px;box-shadow:0 4px 16px rgba(20,30,60,.1);display:flex;flex-direction:column;gap:12px}}
.card input{{padding:9px;border:1px solid #cdd4e0;border-radius:7px}}
.card .lang{{text-align:center;margin:0}}
.msg{{background:#e7f5ec;border:1px solid #bde3cb;padding:8px 12px;border-radius:8px}}
.err{{color:#c0392b;margin:0}}
.bar{{margin-bottom:14px;display:flex;align-items:center;gap:8px}}
.search{{padding:7px 10px;border:1px solid #cdd4e0;border-radius:7px;width:300px}}
.count{{color:#5a6478;font-size:13px}}
select{{padding:7px;border-radius:7px;border:1px solid #cdd4e0;background:#fff}}
.op{{font-size:12px;padding:2px 7px;border-radius:5px;background:#eef0f4}}
.op-delete,.op-trash_purged{{background:#fdecea;color:#c0392b}}
.op-conflict{{background:#fff3df;color:#a96b00}}
.op-create{{background:#e7f5ec;color:#1e7b45}}
.op-restore,.op-conflict_resolved{{background:#e8efff;color:#2455b5}}
.badge{{font-size:12px;padding:2px 8px;border-radius:10px}}
.badge.on{{background:#e7f5ec;color:#1e7b45}}
.badge.off{{background:#fdecea;color:#c0392b}}
.preview-img{{max-width:100%;border-radius:10px;box-shadow:0 1px 4px rgba(20,30,60,.15);background:#fff}}
.preview-text{{background:#fff;border-radius:10px;padding:16px;box-shadow:0 1px 3px rgba(20,30,60,.07);white-space:pre-wrap;word-break:break-word;max-height:70vh;overflow:auto}}
td a{{color:#2455b5;text-decoration:none}}
td a:hover{{text-decoration:underline}}
.muted{{color:#8a93a6}}
.cmp{{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-bottom:14px}}
.cmp-pane{{background:#fff;border-radius:10px;overflow:hidden;box-shadow:0 1px 3px rgba(20,30,60,.07);min-width:0}}
.cmp-head{{padding:8px 12px;font-weight:600;font-size:12px;border-bottom:1px solid #eef0f4}}
.cmp-head-active{{background:#e8efff;color:#2455b5}}
.cmp-head-other{{background:#fff3df;color:#a96b00}}
.cmp-text{{margin:0;padding:12px;white-space:pre-wrap;word-break:break-word;max-height:60vh;overflow:auto;font-size:13px}}
.cmp-pane p{{padding:12px;margin:0}}
.cmp-actions{{display:flex;flex-wrap:wrap;gap:8px}}
/* Tables scroll horizontally rather than overflowing the viewport. */
table{{display:block;overflow-x:auto;white-space:nowrap}}
table.kv{{white-space:normal}}
td,th{{white-space:nowrap}}
@media (max-width:760px){{
  .shell{{flex-direction:column}}
  aside{{width:100%;flex-direction:row;align-items:center}}
  aside nav{{flex-direction:row;overflow-x:auto}}
  aside nav a .lbl{{display:none}}
  aside nav a{{padding:9px 11px}}
  .logo,.aside-foot{{border:0}}
  .aside-foot{{flex-shrink:0}}
  header{{padding:12px 16px}}
  section{{padding:16px}}
  .cmp{{grid-template-columns:1fr}}
  .search{{width:100%}}
  .bar{{flex-wrap:wrap}}
}}
</style></head><body>{content}</body></html>"#
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn human_size(bytes: i64) -> String {
    let b = bytes as f64;
    if b >= 1_073_741_824.0 {
        format!("{:.2} GB", b / 1_073_741_824.0)
    } else if b >= 1_048_576.0 {
        format!("{:.1} MB", b / 1_048_576.0)
    } else if b >= 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
