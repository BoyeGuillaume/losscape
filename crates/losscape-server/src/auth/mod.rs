//! Users, sessions, API keys and project permissions.
//!
//! - Browser: a session cookie (`HttpOnly`, `SameSite=Lax`, `Secure` over HTTPS) holding a
//!   random token; the server stores only its SHA-256.
//! - SDK: `Authorization: Bearer df_...` API keys created by each user; also stored as
//!   SHA-256 only.
//! - Projects: members are `owner` (manages members), `editor` (logs runs) or `viewer`.

pub mod account;
pub mod avatar;
pub mod members;
pub mod oauth;
pub mod secrets;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};

use crate::config::AuthConfig;
use crate::error::ApiError;
use crate::read::UiError;
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "losscape_session";
pub const SESSION_DAYS: i64 = 30;
const API_KEY_CACHE_TTL: Duration = Duration::from_secs(30);
const ROLE_CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Role::Viewer),
            "editor" => Some(Role::Editor),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Session,
    ApiKey,
}

/// The authenticated caller, put in request extensions by [`require_user`].
#[derive(Clone, Debug)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
    pub via: Via,
}

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or_else(unauthorized)
    }
}

pub fn unauthorized() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "sign in, or send an API key (Authorization: Bearer df_...)",
    )
}

pub fn forbidden(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "forbidden", message)
}

/// Caches in front of the database for the hot paths (every SDK request is authenticated
/// and authorized). Entries expire quickly; changes also clear them explicitly.
type CachedRole = (Option<Role>, Instant);

#[derive(Default)]
pub struct AuthCaches {
    api_keys: Mutex<HashMap<Vec<u8>, (AuthUser, Instant)>>,
    /// (user id, project id) -> role.
    roles: Mutex<HashMap<(i64, i64), CachedRole>>,
    pub login_limiter: LoginLimiter,
}

impl AuthCaches {
    pub fn forget_api_keys(&self) {
        self.api_keys.lock().expect("not poisoned").clear();
    }

    pub fn forget_project(&self, project_id: i64) {
        self.roles
            .lock()
            .expect("not poisoned")
            .retain(|(_, p), _| *p != project_id);
    }
}

/// Requires a signed-in user (session cookie or API key) and blocks cross-site
/// state-changing requests made with the cookie.
pub async fn require_user(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    // Owned copies: the request body isn't Sync, so it can't be borrowed across awaits.
    let (headers, method) = (req.headers().clone(), req.method().clone());
    let checked = async {
        let user = authenticate(&state, &headers)
            .await?
            .ok_or_else(unauthorized)?;
        if user.via == Via::Session && !is_safe(&method) {
            check_same_origin(&headers, state.auth_config())?;
        }
        Ok::<_, ApiError>(user)
    }
    .await;
    match checked {
        Ok(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        // The SDK speaks protobuf; everything else gets JSON errors.
        Err(err) if is_protobuf(&headers) => err.into_response(),
        Err(err) => UiError::from(err).into_response(),
    }
}

fn is_protobuf(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with(crate::proto_body::CONTENT_TYPE))
}

/// The caller, if any. A present but invalid API key is an error, not "anonymous".
pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<AuthUser>, ApiError> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if let Some(key) = bearer {
        return user_for_api_key(state, key.trim()).await.map(Some);
    }
    match CookieJar::from_headers(headers).get(SESSION_COOKIE) {
        Some(cookie) => user_for_session(state, cookie.value()).await,
        None => Ok(None),
    }
}

async fn user_for_api_key(state: &AppState, key: &str) -> Result<AuthUser, ApiError> {
    if !key.starts_with(secrets::API_KEY_PREFIX) || key.len() > 128 {
        return Err(unauthorized());
    }
    let hash = secrets::sha256(key);
    let caches = state.auth_caches();
    if let Some((user, at)) = caches.api_keys.lock().expect("not poisoned").get(&hash)
        && at.elapsed() < API_KEY_CACHE_TTL
    {
        return Ok(user.clone());
    }
    let row: Option<(i64, String, bool)> = sqlx::query_as(
        "UPDATE api_keys k SET last_used_at = now()
         FROM users u
         WHERE k.key_hash = $1 AND k.revoked_at IS NULL AND u.id = k.user_id AND u.disabled_at IS NULL
         RETURNING u.id, u.username, u.is_admin",
    )
    .bind(&hash)
    .fetch_optional(state.pg())
    .await?;
    let (id, username, is_admin) = row.ok_or_else(unauthorized)?;
    let user = AuthUser {
        id,
        username,
        is_admin,
        via: Via::ApiKey,
    };
    caches
        .api_keys
        .lock()
        .expect("not poisoned")
        .insert(hash, (user.clone(), Instant::now()));
    Ok(user)
}

async fn user_for_session(state: &AppState, token: &str) -> Result<Option<AuthUser>, ApiError> {
    if token.len() > 128 {
        return Ok(None);
    }
    let row: Option<(i64, String, bool)> = sqlx::query_as(
        "SELECT u.id, u.username, u.is_admin FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now() AND u.disabled_at IS NULL",
    )
    .bind(secrets::sha256(token))
    .fetch_optional(state.pg())
    .await?;
    Ok(row.map(|(id, username, is_admin)| AuthUser {
        id,
        username,
        is_admin,
        via: Via::Session,
    }))
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Cross-site request forgery guard for cookie-authenticated requests, on top of
/// `SameSite=Lax`: browsers send `Sec-Fetch-Site`; older ones send `Origin`, which must be
/// this server. Requests with neither come from non-browser clients, which can't ride on
/// a victim's cookie.
pub fn check_same_origin(headers: &HeaderMap, config: &AuthConfig) -> Result<(), ApiError> {
    let header_str = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(site) = header_str("sec-fetch-site") {
        return match site {
            "same-origin" | "none" => Ok(()),
            _ => Err(forbidden("cross-site request blocked")),
        };
    }
    if let Some(origin) = header_str("origin") {
        let host = header_str("host").unwrap_or_default();
        let same = config.public_url.as_deref() == Some(origin)
            || origin
                .split_once("://")
                .is_some_and(|(_, rest)| rest == host);
        if !same {
            return Err(forbidden("cross-site request blocked"));
        }
    }
    Ok(())
}

pub fn session_cookie(token: String, config: &AuthConfig) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .secure(config.cookie_secure())
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(SESSION_DAYS))
        .build()
}

pub fn removal_cookie(name: &'static str, path: &'static str) -> Cookie<'static> {
    let mut cookie = Cookie::build((name, "")).path(path).build();
    cookie.make_removal();
    cookie
}

/// Creates a session and returns its cookie.
pub async fn start_session(
    state: &AppState,
    user_id: i64,
    headers: &HeaderMap,
) -> Result<Cookie<'static>, ApiError> {
    let token = secrets::random_token();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(256).collect::<String>());
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, expires_at, user_agent)
         VALUES ($1, $2, now() + make_interval(days => $3), $4)",
    )
    .bind(secrets::sha256(&token))
    .bind(user_id)
    .bind(SESSION_DAYS as i32)
    .bind(user_agent)
    .execute(state.pg())
    .await?;
    // Opportunistic cleanup; sessions are few.
    sqlx::query("DELETE FROM sessions WHERE expires_at < now()")
        .execute(state.pg())
        .await
        .ok();
    Ok(session_cookie(token, state.auth_config()))
}

/// The caller's role in a project (admins own everything), cached briefly.
pub async fn project_role(
    state: &AppState,
    user: &AuthUser,
    project_id: i64,
) -> Result<Option<Role>, ApiError> {
    if user.is_admin {
        return Ok(Some(Role::Owner));
    }
    let key = (user.id, project_id);
    if let Some((role, at)) = state
        .auth_caches()
        .roles
        .lock()
        .expect("not poisoned")
        .get(&key)
        && at.elapsed() < ROLE_CACHE_TTL
    {
        return Ok(*role);
    }
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM project_members WHERE project_id = $1 AND user_id = $2",
    )
    .bind(project_id)
    .bind(user.id)
    .fetch_optional(state.pg())
    .await?;
    let role = role.as_deref().and_then(Role::parse);
    state
        .auth_caches()
        .roles
        .lock()
        .expect("not poisoned")
        .insert(key, (role, Instant::now()));
    Ok(role)
}

/// Requires at least `min` in the project. Users without any role get `not_found`, so
/// project and run ids don't reveal what exists.
pub async fn require_role(
    state: &AppState,
    user: &AuthUser,
    project_id: i64,
    min: Role,
    not_found: fn() -> ApiError,
) -> Result<Role, ApiError> {
    match project_role(state, user, project_id).await? {
        None => Err(not_found()),
        Some(role) if role < min => Err(forbidden(format!(
            "this needs the {} role in the project",
            min.as_str()
        ))),
        Some(role) => Ok(role),
    }
}

/// The run's project, if the caller has at least `min` in it (unknown run otherwise).
pub async fn require_run(
    state: &AppState,
    user: &AuthUser,
    run_id: uuid::Uuid,
    min: Role,
) -> Result<i64, ApiError> {
    let project_id = state.project_of(run_id).await?;
    require_role(state, user, project_id, min, ApiError::unknown_run).await?;
    Ok(project_id)
}

/// Slows down password guessing: a username, and an address, get a limited number of
/// failed attempts per window.
#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_FAILURES_PER_USER: u32 = 10;
const MAX_FAILURES_PER_IP: u32 = 50;

impl LoginLimiter {
    fn keys(username: &str, ip: Option<IpAddr>) -> [(String, u32); 2] {
        [
            (
                format!("user:{}", username.to_lowercase()),
                MAX_FAILURES_PER_USER,
            ),
            (
                format!("ip:{}", ip.map(|ip| ip.to_string()).unwrap_or_default()),
                MAX_FAILURES_PER_IP,
            ),
        ]
    }

    pub fn check(&self, username: &str, ip: Option<IpAddr>) -> Result<(), ApiError> {
        let failures = self.failures.lock().expect("not poisoned");
        for (key, max) in Self::keys(username, ip) {
            if let Some((count, since)) = failures.get(&key)
                && since.elapsed() < LOGIN_WINDOW
                && *count >= max
            {
                return Err(ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too_many_attempts",
                    "too many failed sign-ins; try again in a few minutes",
                ));
            }
        }
        Ok(())
    }

    pub fn failed(&self, username: &str, ip: Option<IpAddr>) {
        let mut failures = self.failures.lock().expect("not poisoned");
        failures.retain(|_, (_, since)| since.elapsed() < LOGIN_WINDOW);
        for (key, _) in Self::keys(username, ip) {
            let entry = failures.entry(key).or_insert((0, Instant::now()));
            entry.0 += 1;
        }
    }

    pub fn succeeded(&self, username: &str) {
        self.failures
            .lock()
            .expect("not poisoned")
            .remove(&format!("user:{}", username.to_lowercase()));
    }
}

/// Client address when the server runs with connect info (absent in in-process tests).
pub struct ClientIp(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn same_origin_check() {
        let config = AuthConfig::default();
        assert!(check_same_origin(&headers(&[("sec-fetch-site", "same-origin")]), &config).is_ok());
        assert!(check_same_origin(&headers(&[("sec-fetch-site", "cross-site")]), &config).is_err());
        assert!(check_same_origin(&headers(&[("sec-fetch-site", "same-site")]), &config).is_err());
        let ok = headers(&[
            ("origin", "http://localhost:8080"),
            ("host", "localhost:8080"),
        ]);
        assert!(check_same_origin(&ok, &config).is_ok());
        let evil = headers(&[
            ("origin", "https://evil.example"),
            ("host", "localhost:8080"),
        ]);
        assert!(check_same_origin(&evil, &config).is_err());
        assert!(
            check_same_origin(&HeaderMap::new(), &config).is_ok(),
            "non-browser client"
        );
    }

    #[test]
    fn roles_are_ordered() {
        assert!(Role::Owner > Role::Editor && Role::Editor > Role::Viewer);
        assert_eq!(Role::parse("editor"), Some(Role::Editor));
        assert_eq!(Role::parse("admin"), None);
    }

    #[test]
    fn limiter_blocks_after_repeated_failures() {
        let limiter = LoginLimiter::default();
        for _ in 0..MAX_FAILURES_PER_USER {
            assert!(limiter.check("alice", None).is_ok());
            limiter.failed("Alice", None);
        }
        assert!(limiter.check("alice", None).is_err(), "case-insensitive");
        assert!(limiter.check("bob", None).is_ok());
        limiter.succeeded("alice");
        assert!(limiter.check("alice", None).is_ok());
    }
}
