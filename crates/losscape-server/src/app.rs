use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, patch, post, put};
use axum::{Json, Router, middleware};
use base64::Engine;
use sha2::{Digest, Sha256};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::auth::{account, avatar, members, oauth};
use crate::config::BrandingImage;
use crate::error::ApiError;
use crate::read::UiError;
use crate::state::AppState;
use crate::{auth, board, ingest, live, metric_defs, projects, read, runs, tags};

pub fn router(state: AppState, max_body_bytes: usize, web_dir: Option<&Path>) -> Router {
    // Signed-in users: session cookie (web UI) or API key (SDK).
    let user_api = Router::new()
        // SDK (protobuf)
        .route("/runs/init", post(runs::init))
        .route("/runs/heartbeat", post(runs::heartbeat))
        .route("/runs/finish", post(runs::finish))
        .route("/ingest", post(ingest::ingest))
        // Web UI (JSON, binary tiles, SSE)
        .route("/projects", get(read::projects).post(projects::create))
        .route(
            "/projects/{project}",
            get(projects::project_info)
                .patch(projects::update_project)
                .delete(projects::delete_project),
        )
        .route("/projects/{project}/transfer", post(members::transfer))
        .route("/projects/{project}/board", get(board::get).put(board::put))
        .route(
            "/projects/{project}/metric-definitions",
            get(metric_defs::list).put(metric_defs::replace),
        )
        .route("/projects/{project}/metric-keys", get(metric_defs::keys))
        .route(
            "/runs/{run}/metric-renames/{*from}",
            put(metric_defs::rename).delete(metric_defs::unrename),
        )
        .route(
            "/projects/{project}/runs/archive",
            post(projects::archive_runs),
        )
        .route(
            "/projects/{project}/runs/restore",
            post(projects::restore_runs),
        )
        .route("/projects/{project}/runs/abort", post(projects::abort_runs))
        .route("/projects/{project}/runs", get(read::project_runs))
        .route("/projects/{project}/runs/tags", post(tags::change))
        .route(
            "/projects/{project}/runs/metric-renames",
            post(metric_defs::rename_many),
        )
        .route("/projects/{project}/tags", get(tags::list))
        .route("/projects/{project}/tags/{*name}", put(tags::describe))
        .route("/projects/{project}/members", get(members::list))
        .route(
            "/projects/{project}/members/{username}",
            put(members::set_role).delete(members::remove),
        )
        .route("/projects/{project}/metrics", post(read::project_metrics))
        .route("/projects/{project}/live", get(live::live))
        .route(
            "/projects/{project}/aliases/{alias}",
            get(projects::get_alias)
                .put(projects::set_alias)
                .delete(projects::remove_alias),
        )
        .route("/runs/{run}", get(read::run_detail))
        .route("/runs/{run}/logs", get(read::run_logs))
        .route("/runs/{run}/models", get(read::run_models))
        .route("/runs/{run}/logs.txt", get(read::run_logs_file))
        .route("/runs/{run}/files/{name}", get(read::run_file))
        .route("/series", post(read::series))
        .route("/auth/me", get(account::me))
        .route("/me", patch(account::update_profile))
        .route(
            "/me/avatar",
            put(avatar::upload)
                .delete(avatar::remove)
                .layer(DefaultBodyLimit::max(avatar::MAX_UPLOAD_BYTES)),
        )
        .route("/users/{username}/avatar", get(avatar::serve))
        .route("/me/password", post(account::change_password))
        .route(
            "/me/api-keys",
            get(account::list_api_keys).post(account::create_api_key),
        )
        .route("/me/api-keys/{id}", delete(account::revoke_api_key))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_user,
        ));

    let public_api = Router::new()
        .route("/version", get(version))
        .route("/app", get(app_info))
        .route("/auth/settings", get(account::settings))
        .route("/auth/register", post(account::register))
        .route("/auth/login", post(account::login))
        .route("/auth/logout", post(account::logout))
        .route("/auth/oauth/{provider}/start", get(oauth::start))
        .route("/auth/oauth/{provider}/callback", get(oauth::callback));

    let mut router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/branding/favicon", get(favicon))
        .route("/branding/auth-background", get(auth_background))
        .nest("/api/v0", user_api.merge(public_api))
        .route("/api/{*rest}", any(unknown_endpoint))
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(state);
    if let Some(dir) = web_dir {
        // Unknown paths get index.html so client-side routes work on reload. Not under `_app`:
        // a tab open since before an upgrade must see that the code it asks for is gone.
        let app = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
        router = router
            .nest_service("/_app", ServeDir::new(dir.join("_app")))
            .fallback_service(app);
    }
    let policy = Arc::new(Policy::new(web_dir));
    router
        // A bug in one request gets an answer, rather than looking like a lost connection.
        .layer(CatchPanicLayer::custom(panicked))
        .layer(middleware::map_response(move |response| {
            security_headers(response, policy.current())
        }))
        // Run lists and tiles shrink ~10x; event streams and small bodies are left alone.
        .layer(CompressionLayer::new())
}

/// The Content-Security-Policy, recomputed when the built app's `index.html` changes (its
/// inline bootstrap script, and so its hash, changes with every build).
struct Policy {
    index: Option<PathBuf>,
    cached: Mutex<(Option<SystemTime>, Arc<str>)>,
}

impl Policy {
    fn new(web_dir: Option<&Path>) -> Self {
        let index = web_dir.map(|dir| dir.join("index.html"));
        let policy = content_security_policy(index.as_deref());
        let modified = index.as_deref().and_then(modified);
        Self {
            index,
            cached: Mutex::new((modified, policy.into())),
        }
    }

    fn current(&self) -> Arc<str> {
        let modified = self.index.as_deref().and_then(modified);
        let mut cached = self.cached.lock().expect("not poisoned");
        if cached.0 != modified {
            *cached = (
                modified,
                content_security_policy(self.index.as_deref()).into(),
            );
        }
        cached.1.clone()
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Defense in depth against XSS and clickjacking. Scripts may only come from this server,
/// plus the exact inline bootstrap script of the built app (allowed by its hash).
fn content_security_policy(index: Option<&Path>) -> String {
    let mut script_src = String::from("'self'");
    let html = index
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    for inline in inline_scripts(&html) {
        let hash =
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(inline.as_bytes()));
        script_src.push_str(&format!(" 'sha256-{hash}'"));
    }
    format!(
        "default-src 'self'; script-src {script_src}; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
         font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; \
         frame-ancestors 'none'"
    )
}

/// Bodies of `<script>` elements without a `src`.
fn inline_scripts(html: &str) -> Vec<&str> {
    let mut scripts = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        let Some(tag_end) = rest[open..].find('>') else {
            break;
        };
        let attrs = &rest[open + "<script".len()..open + tag_end];
        let body_start = open + tag_end + 1;
        let Some(close) = rest[body_start..].find("</script>") else {
            break;
        };
        if !attrs.contains("src=") {
            scripts.push(&rest[body_start..body_start + close]);
        }
        rest = &rest[body_start + close + "</script>".len()..];
    }
    scripts
}

async fn security_headers(mut response: Response, policy: Arc<str>) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    if let Ok(value) = HeaderValue::from_str(&policy) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

async fn unknown_endpoint() -> UiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_endpoint",
        "no such endpoint",
    )
    .into()
}

fn panicked(_: Box<dyn std::any::Any + Send>) -> Response {
    // The panic itself has been logged already, by the panic hook.
    UiError::from(ApiError::internal()).into_response()
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> (StatusCode, &'static str) {
    let pg = sqlx::query("SELECT 1").execute(state.pg()).await;
    let ch = state.ch().query("SELECT 1").fetch_one::<u8>().await;
    match (pg, ch) {
        (Ok(_), Ok(_)) => (StatusCode::OK, "ok"),
        (Err(_), _) => (StatusCode::SERVICE_UNAVAILABLE, "postgres unavailable"),
        (_, Err(_)) => (StatusCode::SERVICE_UNAVAILABLE, "clickhouse unavailable"),
    }
}

#[derive(serde::Serialize)]
struct VersionInfo {
    version: &'static str,
    protocol: u32,
}

async fn version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: losscape_core::VERSION,
        protocol: losscape_core::PROTOCOL_VERSION,
    })
}

/// What this deployment calls itself and how it looks: public, for the sign-in and About pages.
#[derive(serde::Serialize)]
struct AppInfo {
    name: String,
    version: &'static str,
    protocol: u32,
    favicon: Option<String>,
    auth_background: Option<String>,
}

async fn app_info(State(state): State<AppState>) -> Json<AppInfo> {
    let branding = state.branding();
    let url = |path: &str, image: &Option<BrandingImage>| {
        image
            .as_ref()
            .map(|i| format!("/branding/{path}?v={}", i.hash))
    };
    Json(AppInfo {
        name: branding.app_name.clone(),
        version: losscape_core::VERSION,
        protocol: losscape_core::PROTOCOL_VERSION,
        favicon: url("favicon", &branding.favicon),
        auth_background: url("auth-background", &branding.auth_background),
    })
}

fn image_response(image: Option<&BrandingImage>) -> Response {
    let Some(image) = image else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, image.content_type),
            // Its URL changes with its content.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        image.bytes.clone(),
    )
        .into_response()
}

async fn favicon(State(state): State<AppState>) -> Response {
    image_response(state.branding().favicon.as_ref())
}

async fn auth_background(State(state): State<AppState>) -> Response {
    image_response(state.branding().auth_background.as_ref())
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;
    use tower_http::catch_panic::CatchPanicLayer;

    use super::{inline_scripts, panicked};

    #[tokio::test]
    async fn a_panic_gets_an_answer() {
        async fn broken() -> &'static str {
            panic!("a bug");
        }
        let app = Router::new()
            .route("/", get(broken))
            .layer(CatchPanicLayer::custom(panicked));
        let request = Request::get("/").body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "internal");
    }

    #[test]
    fn finds_inline_scripts_only() {
        let html = r#"<head><script src="/a.js"></script><script type="module">boot()</script></head>
                      <body><script>
x()
</script></body>"#;
        assert_eq!(inline_scripts(html), ["boot()", "\nx()\n"]);
    }
}
