//! JWT issuance/verification and the `AuthDevice` extractor.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::{
    error::{AppError, AppResult},
    routes::AppState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // device id
    pub name: String,
    pub platform: Option<String>,
    pub iat: i64,
    pub exp: i64,
}

pub fn issue(
    device_id: &str,
    name: &str,
    platform: Option<&str>,
    secret: &str,
    ttl_days: u32,
) -> AppResult<String> {
    let now = Utc::now();
    let exp = now + Duration::days(ttl_days as i64);
    let claims = Claims {
        sub: device_id.to_string(),
        name: name.to_string(),
        platform: platform.map(|s| s.to_string()),
        iat: now.timestamp(),
        exp: exp.timestamp(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("jwt encode: {}", e)))
}

/// Constant-time byte comparison; used for pairing code and admin password.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// Find a `token=...` parameter in a raw query string. JWTs are base64url so
/// they're URL-safe; no percent-decoding required.
fn query_token(query: &str) -> Option<&str> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
}

pub fn verify(token: &str, secret: &str) -> AppResult<Claims> {
    let validation = Validation::default();
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| AppError::Unauthorized)?;
    Ok(data.claims)
}

/// Authenticated device, extracted from a `Bearer` token.
#[derive(Debug, Clone)]
pub struct AuthDevice {
    pub id: String,
    pub name: String,
}

#[axum::async_trait]
impl FromRequestParts<AppState> for AuthDevice {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Prefer the Authorization header; fall back to ?token=… because the
        // browser WebSocket API can't set custom headers.
        let token: String = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.to_string())
            .or_else(|| {
                parts
                    .uri
                    .query()
                    .and_then(|q| query_token(q).map(|s| s.to_string()))
            })
            .ok_or(AppError::Unauthorized)?;

        let claims = verify(&token, &state.cfg.jwt_secret)?;

        // Best-effort: bump last_seen_at. Failures here don't block the request.
        let now = Utc::now().to_rfc3339();
        let _ = sqlx::query(
            "UPDATE devices SET last_seen_at = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(&now)
        .bind(&claims.sub)
        .execute(&state.pool)
        .await;

        // If the device was revoked, reject.
        let revoked: Option<(Option<String>,)> =
            sqlx::query_as("SELECT revoked_at FROM devices WHERE id = ?")
                .bind(&claims.sub)
                .fetch_optional(&state.pool)
                .await
                .map_err(AppError::Db)?;

        match revoked {
            Some((Some(_),)) | None => Err(AppError::Unauthorized),
            Some((None,)) => Ok(AuthDevice {
                id: claims.sub,
                name: claims.name,
            }),
        }
    }
}
