//! Sign-in with external providers: any OpenID Connect provider, and GitHub.
//!
//! Authorization code flow with PKCE (S256). The `state` value is bound to the browser
//! that started the flow by a short-lived cookie, so a callback link from someone else
//! can't sign the victim into the attacker's account. For OIDC the ID token's signature,
//! issuer, audience, expiry and nonce are verified. State, nonce and PKCE verifier come
//! from the OS CSPRNG.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::url::{Url, form_urlencoded};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, reqwest,
};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use super::{Via, account, authenticate, secrets, start_session};
use crate::config::{OAuthKind, OAuthProvider};
use crate::error::ApiError;
use crate::read::UiError;
use crate::state::AppState;

const STATE_COOKIE: &str = "losscape_oauth";
const CALLBACK_PATH: &str = "/api/v0/auth/oauth";
const STATE_TTL_MINUTES: i64 = 10;
const METADATA_TTL: Duration = Duration::from_secs(3600);

/// Shared HTTP client (no redirects followed, as the OAuth spec requires) and cached
/// OIDC discovery documents.
pub struct OAuthRuntime {
    http: reqwest::Client,
    metadata: Mutex<HashMap<String, (CoreProviderMetadata, Instant)>>,
}

impl Default for OAuthRuntime {
    fn default() -> Self {
        Self {
            http: reqwest::ClientBuilder::new()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
                .expect("HTTP client"),
            metadata: Mutex::default(),
        }
    }
}

/// What we learn about the person from the provider.
struct Identity {
    subject: String,
    /// Only when the provider says it is verified.
    email: Option<String>,
    username_hint: Option<String>,
    display_name: Option<String>,
}

fn provider<'a>(state: &'a AppState, id: &str) -> Result<&'a OAuthProvider, ApiError> {
    state
        .auth_config()
        .oauth
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "unknown_provider",
                "no such sign-in provider",
            )
        })
}

fn redirect_uri(state: &AppState, provider: &OAuthProvider) -> String {
    let base = state
        .auth_config()
        .public_url
        .as_deref()
        .unwrap_or_default();
    format!("{base}{CALLBACK_PATH}/{}/callback", provider.id)
}

async fn oidc_metadata(
    state: &AppState,
    provider: &OAuthProvider,
    issuer: &str,
) -> Result<CoreProviderMetadata, ApiError> {
    let runtime = state.oauth();
    if let Some((metadata, at)) = runtime
        .metadata
        .lock()
        .expect("not poisoned")
        .get(&provider.id)
        && at.elapsed() < METADATA_TTL
    {
        return Ok(metadata.clone());
    }
    let issuer = IssuerUrl::new(issuer.to_owned()).map_err(|e| oauth_error("issuer url", e))?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &runtime.http)
        .await
        .map_err(|e| oauth_error("OIDC discovery", e))?;
    runtime
        .metadata
        .lock()
        .expect("not poisoned")
        .insert(provider.id.clone(), (metadata.clone(), Instant::now()));
    Ok(metadata)
}

fn oauth_error(what: &str, err: impl std::fmt::Display) -> ApiError {
    tracing::warn!(%err, "OAuth: {what} failed");
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "oauth_failed",
        "the sign-in provider could not be reached",
    )
}

/// Only local paths, so the sign-in can't be turned into an open redirect.
fn safe_return_to(path: Option<String>) -> String {
    match path {
        Some(p)
            if p.starts_with('/')
                && !p.starts_with("//")
                && !p.starts_with("/\\")
                && p.len() <= 512
                && !p.chars().any(char::is_control) =>
        {
            p
        }
        _ => "/".into(),
    }
}

#[derive(Deserialize)]
pub struct StartParams {
    return_to: Option<String>,
}

/// `GET /api/v0/auth/oauth/{provider}/start`: redirects to the provider. When already
/// signed in, the external account is linked to the current user instead.
pub async fn start(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Query(params): Query<StartParams>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Response, UiError> {
    let provider = provider(&state, &provider_id)?;
    let link_user = authenticate(&state, &headers)
        .await
        .ok()
        .flatten()
        .filter(|u| u.via == Via::Session)
        .map(|u| u.id);
    let (state_token, verifier, nonce) = (
        secrets::random_token(),
        secrets::random_token(),
        secrets::random_token(),
    );

    sqlx::query("DELETE FROM oauth_states WHERE created_at < now() - make_interval(mins => $1)")
        .bind(STATE_TTL_MINUTES as i32)
        .execute(state.pg())
        .await?;
    sqlx::query(
        "INSERT INTO oauth_states (state_hash, provider, pkce_verifier, nonce, link_user_id, return_to)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(secrets::sha256(&state_token))
    .bind(&provider.id)
    .bind(&verifier)
    .bind(&nonce)
    .bind(link_user)
    .bind(safe_return_to(params.return_to))
    .execute(state.pg())
    .await?;

    let redirect = redirect_uri(&state, provider);
    let url = match &provider.kind {
        OAuthKind::Oidc { issuer } => {
            let metadata = oidc_metadata(&state, provider, issuer).await?;
            let client = CoreClient::from_provider_metadata(
                metadata,
                ClientId::new(provider.client_id.clone()),
                Some(ClientSecret::new(provider.client_secret.clone())),
            )
            .set_redirect_uri(
                RedirectUrl::new(redirect).map_err(|e| oauth_error("redirect url", e))?,
            );
            let challenge =
                PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(verifier));
            let (csrf, nonce) = (state_token.clone(), nonce.clone());
            let (url, _, _) = client
                .authorize_url(
                    CoreAuthenticationFlow::AuthorizationCode,
                    move || CsrfToken::new(csrf),
                    move || Nonce::new(nonce),
                )
                .add_scope(Scope::new("email".into()))
                .add_scope(Scope::new("profile".into()))
                .set_pkce_challenge(challenge)
                .url();
            url
        }
        OAuthKind::GitHub { web_url, .. } => {
            let mut url = Url::parse(&format!("{web_url}/login/oauth/authorize"))
                .map_err(|e| oauth_error("GitHub url", e))?;
            url.query_pairs_mut()
                .append_pair("client_id", &provider.client_id)
                .append_pair("redirect_uri", &redirect)
                .append_pair("scope", "read:user user:email")
                .append_pair("state", &state_token)
                .append_pair("code_challenge", &secrets::pkce_challenge(&verifier))
                .append_pair("code_challenge_method", "S256")
                .append_pair("allow_signup", "false");
            url
        }
    };

    let cookie = Cookie::build((STATE_COOKIE, state_token))
        .path(CALLBACK_PATH)
        .http_only(true)
        .secure(state.auth_config().cookie_secure())
        // Lax: the provider's redirect back is a top-level navigation, which carries it.
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(STATE_TTL_MINUTES))
        .build();
    Ok((jar.add(cookie), Redirect::to(url.as_str())).into_response())
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /api/v0/auth/oauth/{provider}/callback`: finishes sign-in and redirects into the
/// app (or to `/login?error=...`, with a fixed error code).
pub async fn callback(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Query(params): Query<CallbackParams>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let cookie_state = jar.get(STATE_COOKIE).map(|c| c.value().to_owned());
    let jar = jar.remove(super::removal_cookie(STATE_COOKIE, CALLBACK_PATH));
    match finish(&state, &provider_id, params, cookie_state, &headers).await {
        Ok((session, return_to)) => (jar.add(session), Redirect::to(&return_to)).into_response(),
        Err(code) => (jar, Redirect::to(&format!("/login?error={code}"))).into_response(),
    }
}

async fn finish(
    state: &AppState,
    provider_id: &str,
    params: CallbackParams,
    cookie_state: Option<String>,
    headers: &HeaderMap,
) -> Result<(Cookie<'static>, String), &'static str> {
    let provider = provider(state, provider_id).map_err(|_| "oauth_failed")?;
    if params.error.is_some() {
        return Err("oauth_denied");
    }
    let (Some(code), Some(state_token), Some(cookie_state)) =
        (params.code, params.state, cookie_state)
    else {
        return Err("oauth_state");
    };
    if !bool::from(state_token.as_bytes().ct_eq(cookie_state.as_bytes())) {
        return Err("oauth_state");
    }
    let row: Option<(String, String, Option<i64>, String)> = sqlx::query_as(
        "DELETE FROM oauth_states
         WHERE state_hash = $1 AND provider = $2 AND created_at > now() - make_interval(mins => $3)
         RETURNING pkce_verifier, nonce, link_user_id, return_to",
    )
    .bind(secrets::sha256(&state_token))
    .bind(&provider.id)
    .bind(STATE_TTL_MINUTES as i32)
    .fetch_optional(state.pg())
    .await
    .map_err(|_| "oauth_failed")?;
    let (verifier, nonce, link_user, return_to) = row.ok_or("oauth_state")?;

    let identity = match &provider.kind {
        OAuthKind::Oidc { issuer } => {
            oidc_identity(state, provider, issuer, code, verifier, nonce).await
        }
        OAuthKind::GitHub { web_url, api_url } => {
            github_identity(state, provider, web_url, api_url, code, verifier).await
        }
    }
    .map_err(|err| {
        tracing::warn!(provider = provider.id, %err, "OAuth sign-in failed");
        "oauth_failed"
    })?;

    let user_id = resolve_user(state, provider, &identity, link_user).await?;
    let session = start_session(state, user_id, headers)
        .await
        .map_err(|_| "oauth_failed")?;
    Ok((session, return_to))
}

/// Links, finds, or creates the user for an external identity.
async fn resolve_user(
    state: &AppState,
    provider: &OAuthProvider,
    identity: &Identity,
    link_user: Option<i64>,
) -> Result<i64, &'static str> {
    let existing: Option<(i64, bool)> = sqlx::query_as(
        "SELECT u.id, u.disabled_at IS NOT NULL FROM user_identities i JOIN users u ON u.id = i.user_id
         WHERE i.provider = $1 AND i.subject = $2",
    )
    .bind(&provider.id)
    .bind(&identity.subject)
    .fetch_optional(state.pg())
    .await
    .map_err(|_| "oauth_failed")?;

    let user_id = match (existing, link_user) {
        (Some((_, true)), _) => return Err("account_disabled"),
        (Some((id, false)), Some(link)) if id != link => return Err("identity_in_use"),
        (Some((id, false)), _) => return Ok(id),
        (None, Some(link)) => link,
        (None, None) => create_oauth_user(state, identity).await?,
    };
    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email) VALUES ($1, $2, $3, $4)",
    )
    .bind(&provider.id)
    .bind(&identity.subject)
    .bind(user_id)
    .bind(&identity.email)
    .execute(state.pg())
    .await
    .map_err(|_| "oauth_failed")?;
    Ok(user_id)
}

async fn create_oauth_user(state: &AppState, identity: &Identity) -> Result<i64, &'static str> {
    let hint = identity
        .username_hint
        .clone()
        .or_else(|| {
            identity
                .email
                .as_deref()
                .and_then(|e| e.split('@').next())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let base = username_from_hint(&hint);
    for n in 1..=50 {
        let candidate = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        match account::create_user(
            state,
            &candidate,
            None,
            identity
                .display_name
                .as_deref()
                .and_then(account::clean_display_name)
                .as_deref(),
            identity.email.as_deref(),
            None,
        )
        .await
        {
            Ok(id) => return Ok(id),
            Err(err) if err.code == "setup_token_required" => return Err("setup_token_required"),
            Err(err) if err.code == "username_taken" => continue,
            Err(err) if err.code == "signup_closed" => return Err("signup_closed"),
            Err(_) => return Err("oauth_failed"),
        }
    }
    Err("oauth_failed")
}

/// A valid username from whatever the provider gave (`Jane Doe` -> `Jane-Doe`).
fn username_from_hint(hint: &str) -> String {
    let mut name: String = hint
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    name = name
        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(32)
        .collect();
    if account::validate_username(&name).is_ok() {
        name
    } else {
        "user".into()
    }
}

async fn oidc_identity(
    state: &AppState,
    provider: &OAuthProvider,
    issuer: &str,
    code: String,
    verifier: String,
    nonce: String,
) -> Result<Identity, String> {
    let metadata = oidc_metadata(state, provider, issuer)
        .await
        .map_err(|e| e.message)?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(provider.client_id.clone()),
        Some(ClientSecret::new(provider.client_secret.clone())),
    )
    .set_redirect_uri(RedirectUrl::new(redirect_uri(state, provider)).map_err(|e| e.to_string())?);
    let token = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| e.to_string())?
        .set_pkce_verifier(PkceCodeVerifier::new(verifier))
        .request_async(&state.oauth().http)
        .await
        .map_err(|e| format!("token exchange: {e}"))?;
    let id_token = token.id_token().ok_or("no ID token in the response")?;
    let claims = id_token
        .claims(&client.id_token_verifier(), &Nonce::new(nonce))
        .map_err(|e| format!("ID token: {e}"))?;
    let email = claims
        .email()
        .filter(|_| claims.email_verified() == Some(true))
        .map(|e| e.to_string());
    Ok(Identity {
        subject: claims.subject().to_string(),
        username_hint: claims.preferred_username().map(|u| u.to_string()),
        display_name: claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.to_string()),
        email,
    })
}

async fn github_identity(
    state: &AppState,
    provider: &OAuthProvider,
    web_url: &str,
    api_url: &str,
    code: String,
    verifier: String,
) -> Result<Identity, String> {
    let http = &state.oauth().http;
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &provider.client_id)
        .append_pair("client_secret", &provider.client_secret)
        .append_pair("code", &code)
        .append_pair("redirect_uri", &redirect_uri(state, provider))
        .append_pair("code_verifier", &verifier)
        .finish();
    let token: serde_json::Value = json(
        http.post(format!("{web_url}/login/oauth/access_token"))
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(form),
    )
    .await?;
    let access = token["access_token"]
        .as_str()
        .ok_or("no access token in the response")?
        .to_owned();
    let api = |path: &str| {
        http.get(format!("{api_url}{path}"))
            .bearer_auth(&access)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header(header::USER_AGENT, "losscape")
    };
    let user: serde_json::Value = json(api("/user")).await?;
    let subject = user["id"].as_u64().ok_or("no user id")?.to_string();
    // GitHub's profile email is public and unverified; take the primary verified one.
    let emails: serde_json::Value = json(api("/user/emails")).await.unwrap_or_default();
    let email = emails.as_array().and_then(|list| {
        list.iter()
            .find(|e| e["primary"] == true && e["verified"] == true)
            .and_then(|e| e["email"].as_str())
            .map(str::to_owned)
    });
    Ok(Identity {
        subject,
        email,
        username_hint: user["login"].as_str().map(str::to_owned),
        display_name: user["name"].as_str().map(str::to_owned),
    })
}

async fn json(request: reqwest::RequestBuilder) -> Result<serde_json::Value, String> {
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_stays_local() {
        assert_eq!(safe_return_to(Some("/p/demo".into())), "/p/demo");
        for bad in [
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "javascript:alert(1)",
            "/a\nb",
        ] {
            assert_eq!(safe_return_to(Some(bad.into())), "/", "{bad}");
        }
        assert_eq!(safe_return_to(None), "/");
    }

    #[test]
    fn usernames_from_provider_hints() {
        assert_eq!(username_from_hint("jane.doe"), "jane.doe");
        assert_eq!(username_from_hint("Jane Doe"), "Jane-Doe");
        assert_eq!(username_from_hint("--x"), "user");
        assert_eq!(
            username_from_hint("<script>alert(1)</script>"),
            "script-alert-1---script-"
        );
        assert_eq!(username_from_hint(""), "user");
    }
}
