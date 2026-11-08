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

use crate::config::{AuthConfig, IpNet};
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

impl AuthUser {
    /// For what controls an account or a project: what a leaked API key must not be able to
    /// do. Training code doesn't need any of it.
    pub fn require_session(&self) -> Result<(), ApiError> {
        match self.via {
            Via::Session => Ok(()),
            Via::ApiKey => Err(forbidden("API keys can't do this: use the web interface")),
        }
    }
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

/// Slows down password guessing: limited attempts per window, counted when they start, so
/// that sending them all at once gets no further. Per username and address together, so that
/// guessing someone's password doesn't lock them out; and much higher per username alone,
/// against guesses from many addresses.
#[derive(Default)]
pub struct LoginLimiter {
    attempts: Mutex<HashMap<String, (u32, Instant)>>,
}

const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_ATTEMPTS_PER_USER_AND_IP: u32 = 10;
const MAX_ATTEMPTS_PER_USER: u32 = 200;
const MAX_ATTEMPTS_PER_IP: u32 = 50;
const MAX_REGISTRATIONS_PER_IP: u32 = 30;
const MAX_ENTRIES: usize = 100_000;

/// IPv6 addresses by network: one machine commonly has a whole /64 to choose from.
pub fn address_key(ip: Option<IpAddr>) -> String {
    match ip.map(|ip| ip.to_canonical()) {
        Some(IpAddr::V6(ip)) => {
            let bits = u128::from(ip) & (u128::MAX << 64);
            std::net::Ipv6Addr::from(bits).to_string()
        }
        Some(ip) => ip.to_string(),
        None => String::new(),
    }
}

fn too_many_attempts() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
        "too many failed sign-ins; try again in a few minutes",
    )
}

impl LoginLimiter {
    fn keys(username: &str, ip: Option<IpAddr>) -> [(String, u32); 3] {
        let user = username.to_lowercase();
        let address = address_key(ip);
        [
            (
                format!("both:{address}:{user}"),
                MAX_ATTEMPTS_PER_USER_AND_IP,
            ),
            (format!("user:{user}"), MAX_ATTEMPTS_PER_USER),
            (format!("ip:{address}"), MAX_ATTEMPTS_PER_IP),
        ]
    }

    /// Counts these, unless any of them is over its limit.
    fn count(&self, keys: &[(String, u32)]) -> Result<(), ApiError> {
        let mut attempts = self.attempts.lock().expect("not poisoned");
        if attempts.len() >= MAX_ENTRIES {
            attempts.retain(|_, (_, since)| since.elapsed() < LOGIN_WINDOW);
            // Someone with that many addresses wins a fresh start, rather than a lockout
            // of everyone.
            if attempts.len() >= MAX_ENTRIES {
                attempts.clear();
            }
        }
        let over = keys.iter().any(|(key, max)| {
            attempts
                .get(key)
                .is_some_and(|(n, since)| since.elapsed() < LOGIN_WINDOW && n >= max)
        });
        if over {
            return Err(too_many_attempts());
        }
        for (key, _) in keys {
            let entry = attempts.entry(key.clone()).or_insert((0, Instant::now()));
            if entry.1.elapsed() >= LOGIN_WINDOW {
                *entry = (0, Instant::now());
            }
            entry.0 += 1;
        }
        Ok(())
    }

    /// A sign-in is starting: to be followed by [`Self::succeeded`] if it turns out right.
    pub fn attempt(&self, username: &str, ip: Option<IpAddr>) -> Result<(), ApiError> {
        self.count(&Self::keys(username, ip))
    }

    /// Takes back an attempt that said nothing about the password, like a busy server.
    pub fn forgive(&self, username: &str, ip: Option<IpAddr>) {
        let mut attempts = self.attempts.lock().expect("not poisoned");
        for (key, _) in Self::keys(username, ip) {
            if let Some(entry) = attempts.get_mut(&key) {
                entry.0 = entry.0.saturating_sub(1);
            }
        }
    }

    /// Whoever knows the password starts over, from there.
    pub fn succeeded(&self, username: &str, ip: Option<IpAddr>) {
        self.forgive(username, ip);
        let [(both, _), ..] = Self::keys(username, ip);
        self.attempts.lock().expect("not poisoned").remove(&both);
    }

    /// Accounts are expensive to refuse: see [`secrets::hash_password`].
    pub fn registration(&self, ip: Option<IpAddr>) -> Result<(), ApiError> {
        // Without an address to tell people apart, everyone would share this limit.
        let Some(ip) = ip else { return Ok(()) };
        let key = format!("register:{}", address_key(Some(ip)));
        self.count(&[(key, MAX_REGISTRATIONS_PER_IP)])
    }
}

/// Client address when the server runs with connect info (absent in in-process tests).
pub struct ClientIp(pub Option<IpAddr>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        let forwarded: Vec<&str> = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        let trusted = &state.auth_config().trusted_proxies;
        Ok(ClientIp(peer.map(|peer| {
            client_address(peer, &forwarded.join(","), trusted)
        })))
    }
}

/// Who is really asking. Behind a reverse proxy every request comes from the proxy, which
/// would make all users share one login rate limit, so a trusted one is believed about who
/// it forwards. Anyone else's `X-Forwarded-For` is ignored, since clients can write anything
/// there; and in a list, only the part added by trusted proxies counts, read from the right.
fn client_address(peer: IpAddr, forwarded: &str, trusted: &[IpNet]) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|net| net.contains(ip));
    let mut client = peer.to_canonical();
    if !is_trusted(client) {
        return client;
    }
    for hop in forwarded.rsplit(',') {
        let Ok(ip) = hop.trim().parse::<IpAddr>() else {
            break;
        };
        client = ip.to_canonical();
        if !is_trusted(client) {
            break;
        }
    }
    client
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
    fn believes_only_trusted_proxies_about_clients() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let proxies: Vec<IpNet> = vec!["172.30.42.10".parse().unwrap()];
        let address = |peer, forwarded| client_address(ip(peer), forwarded, &proxies);
        // Directly connected: the header is whatever the client wants it to be.
        assert_eq!(address("8.8.8.8", "1.1.1.1"), ip("8.8.8.8"));
        assert_eq!(client_address(ip("8.8.8.8"), "1.1.1.1", &[]), ip("8.8.8.8"));
        // Through the proxy.
        assert_eq!(address("172.30.42.10", "1.1.1.1"), ip("1.1.1.1"));
        assert_eq!(address("::ffff:172.30.42.10", " 1.1.1.1 "), ip("1.1.1.1"));
        // A client's own forged entries come first: the proxy's is the last one.
        assert_eq!(address("172.30.42.10", "6.6.6.6, 1.1.1.1"), ip("1.1.1.1"));
        // Further trusted proxies on the way are skipped.
        assert_eq!(
            address("172.30.42.10", "1.1.1.1, 172.30.42.10"),
            ip("1.1.1.1")
        );
        // Nothing usable: the proxy itself rather than a guess.
        assert_eq!(address("172.30.42.10", ""), ip("172.30.42.10"));
        assert_eq!(address("172.30.42.10", "garbage"), ip("172.30.42.10"));
        assert_eq!(
            address("172.30.42.10", "1.1.1.1, garbage"),
            ip("172.30.42.10")
        );
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
    fn login_limiter() {
        let (attacker, victim) = (
            Some("198.51.100.1".parse().unwrap()),
            Some("198.51.100.2".parse().unwrap()),
        );
        let limiter = LoginLimiter::default();
        for _ in 0..MAX_ATTEMPTS_PER_USER_AND_IP {
            assert!(limiter.attempt("Alice", attacker).is_ok());
        }
        assert!(
            limiter.attempt("alice", attacker).is_err(),
            "case-insensitive"
        );
        // Being refused isn't counted, or waiting would never help.
        assert!(limiter.attempt("bob", attacker).is_ok());
        // Someone guessing doesn't lock the owner out.
        assert!(limiter.attempt("alice", victim).is_ok());
        limiter.succeeded("alice", attacker);
        assert!(limiter.attempt("alice", attacker).is_ok());
    }

    #[test]
    fn login_limits_on_many_addresses_and_many_usernames() {
        let limiter = LoginLimiter::default();
        let many = |i: u32| Some(IpAddr::from([198, 51, (i >> 8) as u8, i as u8]));
        for i in 0..MAX_ATTEMPTS_PER_USER {
            assert!(limiter.attempt("alice", many(i)).is_ok());
        }
        assert!(limiter.attempt("alice", many(9999)).is_err());

        let one = many(0);
        let limiter = LoginLimiter::default();
        for i in 0..MAX_ATTEMPTS_PER_IP {
            assert!(limiter.attempt(&format!("user{i}"), one).is_ok());
        }
        assert!(limiter.attempt("someone-else", one).is_err());
    }

    #[test]
    fn ipv6_addresses_count_by_network() {
        let ip = |s: &str| Some(s.parse().unwrap());
        assert_eq!(
            address_key(ip("2001:db8:1:2::1")),
            address_key(ip("2001:db8:1:2:ffff::9"))
        );
        assert_ne!(
            address_key(ip("2001:db8:1:2::1")),
            address_key(ip("2001:db8:1:3::1"))
        );
        assert_eq!(address_key(ip("::ffff:192.0.2.1")), "192.0.2.1");
    }

    #[test]
    fn registrations_are_limited_per_address() {
        let limiter = LoginLimiter::default();
        let ip = Some("198.51.100.1".parse().unwrap());
        for _ in 0..MAX_REGISTRATIONS_PER_IP {
            assert!(limiter.registration(ip).is_ok());
        }
        assert!(limiter.registration(ip).is_err());
        assert!(limiter.registration(None).is_ok());
    }
}
