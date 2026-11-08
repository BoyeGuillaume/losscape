//! Accounts: registration, password sign-in, sessions, API keys.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};

use super::avatar::AVATAR_URL_SQL;
use super::{
    AuthUser, ClientIp, SESSION_COOKIE, Via, check_same_origin, removal_cookie, secrets,
    start_session,
};
use crate::config::{OAuthKind, OAuthProvider, Signup};
use crate::error::ApiError;
use crate::read::UiError;
use crate::state::AppState;

const MAX_API_KEYS: i64 = 100;
const MAX_DISPLAY_NAME_CHARS: usize = 64;

#[derive(Serialize)]
pub struct AuthSettings {
    /// A visitor may create an account (always true until the first account exists).
    signup: bool,
    /// No account exists yet: the next one becomes admin.
    first_user: bool,
    providers: Vec<Provider>,
}

#[derive(Serialize)]
struct Provider {
    id: String,
    name: String,
    /// Which logo the sign-in button shows: `google`, `github`, `microsoft`, `gitlab`, or
    /// `key` for any other provider.
    icon: &'static str,
}

fn provider_icon(provider: &OAuthProvider) -> &'static str {
    match &provider.kind {
        OAuthKind::GitHub { .. } => "github",
        OAuthKind::Oidc { issuer } if issuer.contains("accounts.google.com") => "google",
        OAuthKind::Oidc { issuer } if issuer.contains("login.microsoftonline.com") => "microsoft",
        OAuthKind::Oidc { issuer } if issuer.contains("gitlab") => "gitlab",
        OAuthKind::Oidc { .. } => "key",
    }
}

/// `GET /api/v0/auth/settings` (public): what the sign-in page should offer.
pub async fn settings(State(state): State<AppState>) -> Result<Json<AuthSettings>, UiError> {
    let first_user = !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(state.pg())
        .await?;
    let config = state.auth_config();
    Ok(Json(AuthSettings {
        signup: first_user || config.signup == Signup::Open,
        first_user,
        providers: config
            .oauth
            .iter()
            .map(|p| Provider {
                id: p.id.clone(),
                name: p.name.clone(),
                icon: provider_icon(p),
            })
            .collect(),
    }))
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub struct Registration {
    username: String,
    password: String,
    display_name: Option<String>,
}

/// User-entered display name: trimmed, `None` when empty, at most 64 characters, no
/// control characters.
pub fn validate_display_name(name: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(ApiError::invalid(format!(
            "display name: at most {MAX_DISPLAY_NAME_CHARS} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::invalid("display name: no control characters"));
    }
    Ok(Some(name.to_owned()))
}

/// A display name from an external provider, cleaned instead of rejected.
pub fn clean_display_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect();
    Some(cleaned.trim().to_owned()).filter(|n| !n.is_empty())
}

pub fn validate_username(username: &str) -> Result<(), ApiError> {
    let valid = (2..=39).contains(&username.len())
        && username.starts_with(|c: char| c.is_ascii_alphanumeric())
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "username: 2-39 characters from [A-Za-z0-9._-], starting with a letter or digit",
        ))
    }
}

/// How strong a password is, is up to its owner: the UI rates it and gives advice. This only
/// keeps out what can't work: an empty one, and one so long that hashing it is an attack.
fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.is_empty() {
        return Err(ApiError::invalid("password: can't be empty"));
    }
    if password.len() > secrets::MAX_PASSWORD_LEN {
        return Err(ApiError::invalid("password: too long"));
    }
    Ok(())
}

/// Creates a user. The first account becomes admin and takes over projects that have no
/// owner yet (created before accounts existed). A transaction-scoped advisory lock keeps
/// two simultaneous first sign-ups from both becoming admin.
pub async fn create_user(
    state: &AppState,
    username: &str,
    password_hash: Option<&str>,
    display_name: Option<&str>,
    email: Option<&str>,
) -> Result<i64, ApiError> {
    let mut tx = state.pg().begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('losscape.create_user'))")
        .execute(&mut *tx)
        .await?;
    let first: bool = !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users)")
        .fetch_one(&mut *tx)
        .await?;
    if !first && state.auth_config().signup == Signup::Closed {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "signup_closed",
            "sign-up is closed on this server",
        ));
    }
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, display_name, email, is_admin)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(username)
    .bind(password_hash)
    .bind(display_name)
    .bind(email)
    .bind(first)
    .fetch_optional(&mut *tx)
    .await?;
    let id = id.ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "username_taken",
            "this username is taken",
        )
    })?;
    if first {
        sqlx::query(
            "INSERT INTO project_members (project_id, user_id, role)
             SELECT p.id, $1, 'owner' FROM projects p
             WHERE NOT EXISTS (SELECT 1 FROM project_members m WHERE m.project_id = p.id)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    if first {
        tracing::info!(username, "first account created; it is admin");
    }
    Ok(id)
}

/// `POST /api/v0/auth/register` (public).
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<Registration>,
) -> Result<impl IntoResponse, UiError> {
    check_same_origin(&headers, state.auth_config())?;
    let username = body.username.trim();
    validate_username(username)?;
    let display_name = validate_display_name(body.display_name.as_deref())?;
    validate_password(&body.password)?;
    let hash = secrets::hash_password(body.password).await?;
    let id = create_user(&state, username, Some(&hash), display_name.as_deref(), None).await?;
    let cookie = start_session(&state, id, &headers).await?;
    Ok((
        StatusCode::CREATED,
        jar.add(cookie),
        Json(me_for(&state, id).await?),
    ))
}

/// `POST /api/v0/auth/login` (public). Same response and timing for unknown users and
/// wrong passwords; failed attempts are rate-limited per username and address.
pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> Result<impl IntoResponse, UiError> {
    check_same_origin(&headers, state.auth_config())?;
    let limiter = &state.auth_caches().login_limiter;
    let username = body.username.trim();
    limiter.check(username, ip)?;
    if body.password.len() > secrets::MAX_PASSWORD_LEN {
        return Err(ApiError::invalid("password: too long").into());
    }
    let row: Option<(i64, Option<String>)> = sqlx::query_as(
        "SELECT id, password_hash FROM users WHERE lower(username) = lower($1) AND disabled_at IS NULL",
    )
    .bind(username)
    .fetch_optional(state.pg())
    .await?;
    let (id, hash) = match row {
        Some((id, hash)) => (Some(id), hash),
        None => (None, None),
    };
    if !secrets::verify_password(body.password, hash).await {
        limiter.failed(username, ip);
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "wrong username or password",
        )
        .into());
    }
    limiter.succeeded(username);
    let id = id.expect("verified against a stored hash");
    let cookie = start_session(&state, id, &headers).await?;
    Ok((jar.add(cookie), Json(me_for(&state, id).await?)))
}

/// `POST /api/v0/auth/logout` (public; ends the current session if any).
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<impl IntoResponse, UiError> {
    check_same_origin(&headers, state.auth_config())?;
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(secrets::sha256(cookie.value()))
            .execute(state.pg())
            .await?;
    }
    Ok((
        jar.remove(removal_cookie(SESSION_COOKIE, "/")),
        StatusCode::NO_CONTENT,
    ))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Me {
    id: i64,
    username: String,
    display_name: Option<String>,
    email: Option<String>,
    /// Profile picture, if one was uploaded (otherwise the UI shows initials).
    avatar_url: Option<String>,
    is_admin: bool,
    has_password: bool,
    /// External providers linked to this account.
    #[sqlx(skip)]
    identities: Vec<String>,
}

async fn me_for(state: &AppState, user_id: i64) -> Result<Me, ApiError> {
    let sql = format!(
        "SELECT u.id, u.username, u.display_name, u.email, {AVATAR_URL_SQL} AS avatar_url, u.is_admin,
                u.password_hash IS NOT NULL AS has_password
         FROM users u LEFT JOIN user_avatars a ON a.user_id = u.id WHERE u.id = $1"
    );
    let mut me: Me = sqlx::query_as(&sql)
        .bind(user_id)
        .fetch_one(state.pg())
        .await?;
    me.identities = sqlx::query_scalar(
        "SELECT provider FROM user_identities WHERE user_id = $1 ORDER BY provider",
    )
    .bind(user_id)
    .fetch_all(state.pg())
    .await?;
    Ok(me)
}

#[derive(Deserialize)]
pub struct ProfileUpdate {
    /// Empty clears it (the username is shown instead).
    display_name: Option<String>,
}

/// `PATCH /api/v0/me`: the display name.
pub async fn update_profile(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<ProfileUpdate>,
) -> Result<Json<Me>, UiError> {
    if body.display_name.is_some() {
        let display_name = validate_display_name(body.display_name.as_deref())?;
        sqlx::query("UPDATE users SET display_name = $2 WHERE id = $1")
            .bind(user.id)
            .bind(display_name)
            .execute(state.pg())
            .await?;
    }
    Ok(Json(me_for(&state, user.id).await?))
}

/// `GET /api/v0/auth/me`.
pub async fn me(State(state): State<AppState>, user: AuthUser) -> Result<Json<Me>, UiError> {
    Ok(Json(me_for(&state, user.id).await?))
}

#[derive(Deserialize)]
pub struct PasswordChange {
    current: Option<String>,
    new: String,
}

/// `POST /api/v0/me/password`: sets or changes the password and signs out other sessions.
pub async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    jar: CookieJar,
    Json(body): Json<PasswordChange>,
) -> Result<StatusCode, UiError> {
    if user.via != Via::Session {
        return Err(super::forbidden("change the password from the web UI").into());
    }
    let current_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(state.pg())
            .await?;
    if current_hash.is_some() {
        let current = body.current.unwrap_or_default();
        if current.len() > secrets::MAX_PASSWORD_LEN
            || !secrets::verify_password(current, current_hash).await
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "invalid_credentials",
                "the current password is wrong",
            )
            .into());
        }
    }
    validate_password(&body.new)?;
    let hash = secrets::hash_password(body.new).await?;
    let keep = jar
        .get(SESSION_COOKIE)
        .map(|c| secrets::sha256(c.value()))
        .unwrap_or_default();
    let mut tx = state.pg().begin().await?;
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user.id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash <> $2")
        .bind(user.id)
        .bind(keep)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ApiKeyInfo {
    id: i64,
    name: String,
    prefix: String,
    created_at_ms: i64,
    last_used_at_ms: Option<i64>,
}

const API_KEY_COLUMNS: &str =
    "id, name, prefix, (extract(epoch from created_at) * 1000)::bigint AS created_at_ms,
     (extract(epoch from last_used_at) * 1000)::bigint AS last_used_at_ms";

/// `GET /api/v0/me/api-keys`: active keys (never the secrets).
pub async fn list_api_keys(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<ApiKeyInfo>>, UiError> {
    let sql = format!(
        "SELECT {API_KEY_COLUMNS} FROM api_keys WHERE user_id = $1 AND revoked_at IS NULL ORDER BY created_at DESC"
    );
    Ok(Json(
        sqlx::query_as(&sql)
            .bind(user.id)
            .fetch_all(state.pg())
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct NewKeyRequest {
    name: String,
}

#[derive(Serialize)]
pub struct NewKeyResponse {
    #[serde(flatten)]
    info: ApiKeyInfo,
    /// Shown once; only a hash is stored.
    secret: String,
}

/// `POST /api/v0/me/api-keys`.
pub async fn create_api_key(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<NewKeyRequest>,
) -> Result<(StatusCode, Json<NewKeyResponse>), UiError> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::invalid("name: 1-100 characters").into());
    }
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM api_keys WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user.id)
    .fetch_one(state.pg())
    .await?;
    if active >= MAX_API_KEYS {
        return Err(ApiError::invalid(format!(
            "at most {MAX_API_KEYS} active keys; revoke unused ones"
        ))
        .into());
    }
    let (info, secret) = issue_api_key(&state, user.id, name).await?;
    Ok((StatusCode::CREATED, Json(NewKeyResponse { info, secret })))
}

/// Creates an API key for a user; returns it with its secret (shown once).
pub async fn issue_api_key(
    state: &AppState,
    user_id: i64,
    name: &str,
) -> Result<(ApiKeyInfo, String), ApiError> {
    let key = secrets::new_api_key();
    let sql = format!(
        "INSERT INTO api_keys (user_id, name, prefix, key_hash) VALUES ($1, $2, $3, $4) RETURNING {API_KEY_COLUMNS}"
    );
    let info: ApiKeyInfo = sqlx::query_as(&sql)
        .bind(user_id)
        .bind(name)
        .bind(&key.prefix)
        .bind(&key.hash)
        .fetch_one(state.pg())
        .await?;
    Ok((info, key.secret))
}

/// `DELETE /api/v0/me/api-keys/{id}`.
pub async fn revoke_api_key(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, UiError> {
    let revoked = sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL")
        .bind(id)
        .bind(user.id)
        .execute(state.pg())
        .await?
        .rows_affected();
    if revoked == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown_key", "no such key").into());
    }
    state.auth_caches().forget_api_keys();
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        for ok in ["al", "alice", "alice.b", "a-b_c", "A1"] {
            assert!(validate_username(ok).is_ok(), "{ok}");
        }
        for bad in [
            "a",
            "-alice",
            ".a",
            "alice smith",
            "al'; DROP TABLE users;--",
            "<script>",
            &"x".repeat(40),
        ] {
            assert!(validate_username(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn passwords() {
        assert!(validate_password("").is_err());
        assert!(validate_password("x").is_ok());
        assert!(validate_password("correct horse").is_ok());
        assert!(validate_password(&"x".repeat(secrets::MAX_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"x".repeat(secrets::MAX_PASSWORD_LEN + 1)).is_err());
    }
}
