//! Device pairing.
//!
//! `POST /devices/pair { pairing_code, name, platform }` →
//! `{ device_id, token }` on success.
//!
//! The pairing code is set server-side via `ONS_PAIRING_CODE`. If unset,
//! pairing is disabled and the endpoint always returns 401.

use axum::{extract::State, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{self, constant_time_eq},
    error::{AppError, AppResult},
    routes::AppState,
};

#[derive(Debug, Deserialize)]
pub struct PairRequest {
    pub pairing_code: String,
    pub name: String,
    pub platform: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PairResponse {
    pub device_id: String,
    pub token: String,
}

pub async fn pair(
    State(state): State<AppState>,
    Json(req): Json<PairRequest>,
) -> AppResult<Json<PairResponse>> {
    let expected = state
        .cfg
        .pairing_code
        .as_deref()
        .ok_or(AppError::Unauthorized)?;

    if !constant_time_eq(req.pairing_code.as_bytes(), expected.as_bytes()) {
        return Err(AppError::Unauthorized);
    }

    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }

    let device_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO devices (id, name, platform, created_at, last_seen_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&device_id)
    .bind(&req.name)
    .bind(&req.platform)
    .bind(&now)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    let token = auth::issue(
        &device_id,
        &req.name,
        req.platform.as_deref(),
        &state.cfg.jwt_secret,
        state.cfg.jwt_ttl_days,
    )?;

    tracing::info!(device_id = %device_id, name = %req.name, "device paired");

    Ok(Json(PairResponse { device_id, token }))
}

