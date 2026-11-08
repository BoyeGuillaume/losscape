//! Accounts, sessions, API keys, permissions and OAuth, end to end.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Form, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::routing::{get, post};
use base64::Engine;
use common::{TestServer, error_of, user};
use http_body_util::BodyExt;
use losscape_core::ids::RunId;
use losscape_core::proto::v0::{LogBatch, RunInit, ScalarColumns};
use losscape_core::{PROTOCOL_HEADER, PROTOCOL_VERSION};
use losscape_server::config::{AuthConfig, OAuthKind, OAuthProvider, Signup};
use prost::Message;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

/// A browser-like JSON request. `site` is the `Sec-Fetch-Site` header browsers send.
struct Call<'a> {
    method: Method,
    uri: String,
    body: Option<Value>,
    cookie: Option<&'a str>,
    bearer: Option<&'a str>,
    site: Option<&'a str>,
    origin: Option<&'a str>,
}

impl<'a> Call<'a> {
    fn new(method: Method, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            body: None,
            cookie: None,
            bearer: None,
            site: Some("same-origin"),
            origin: None,
        }
    }
    fn get(uri: impl Into<String>) -> Self {
        Self::new(Method::GET, uri)
    }
    fn post(uri: impl Into<String>, body: Value) -> Self {
        Self {
            body: Some(body),
            ..Self::new(Method::POST, uri)
        }
    }
    fn put(uri: impl Into<String>, body: Value) -> Self {
        Self {
            body: Some(body),
            ..Self::new(Method::PUT, uri)
        }
    }
    fn cookie(self, cookie: &'a str) -> Self {
        Self {
            cookie: Some(cookie),
            ..self
        }
    }
    fn bearer(self, key: &'a str) -> Self {
        Self {
            bearer: Some(key),
            ..self
        }
    }
    fn site(self, site: Option<&'a str>) -> Self {
        Self { site, ..self }
    }

    async fn send(self, server: &TestServer) -> Response {
        let mut req = Request::builder().method(self.method).uri(self.uri);
        if let Some(cookie) = self.cookie {
            req = req.header(header::COOKIE, cookie);
        }
        if let Some(key) = self.bearer {
            req = req.header(header::AUTHORIZATION, format!("Bearer {key}"));
        }
        if let Some(site) = self.site {
            req = req.header("sec-fetch-site", site);
        }
        if let Some(origin) = self.origin {
            req = req
                .header(header::ORIGIN, origin)
                .header(header::HOST, "localhost:8080");
        }
        let body = match self.body {
            Some(json) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let res = server
            .router
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        Response {
            status,
            headers,
            bytes,
        }
    }
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Bytes,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
    fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_owned()
    }
    fn set_cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&format!("{name}=")))
            .map(str::to_owned)
    }
    /// `name=value` for a later Cookie header.
    fn cookie(&self, name: &str) -> String {
        self.set_cookie(name)
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .to_owned()
    }
    fn location(&self) -> String {
        self.headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    }
}

async fn register(server: &TestServer, username: &str, password: &str) -> Response {
    Call::post(
        "/api/v0/auth/register",
        json!({"username": username, "password": password}),
    )
    .send(server)
    .await
}

async fn login(server: &TestServer, username: &str, password: &str) -> Response {
    Call::post(
        "/api/v0/auth/login",
        json!({"username": username, "password": password}),
    )
    .send(server)
    .await
}

/// SDK call with an API key.
async fn sdk(
    server: &TestServer,
    key: &str,
    path: &str,
    body: impl Message,
) -> (StatusCode, Bytes) {
    let req = Request::post(path)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(PROTOCOL_HEADER, PROTOCOL_VERSION.to_string())
        .body(Body::from(body.encode_to_vec()))
        .unwrap();
    let res = server.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    (status, res.into_body().collect().await.unwrap().to_bytes())
}

async fn create_project(server: &TestServer, key: &str, name: &str) -> Response {
    Call::post("/api/v0/projects", json!({ "name": name }))
        .bearer(key)
        .send(server)
        .await
}

async fn init_run(server: &TestServer, key: &str, project: &str) -> (StatusCode, RunId, String) {
    let run = RunId::new();
    let init = RunInit {
        run_id: run.as_bytes().to_vec(),
        project: project.into(),
        ..Default::default()
    };
    let (status, body) = sdk(server, key, "/api/v0/runs/init", init).await;
    (status, run, error_of(&body))
}

async fn log_point(server: &TestServer, key: &str, run: RunId, seq: u64) -> (StatusCode, String) {
    let batch = LogBatch {
        run_id: run.as_bytes().to_vec(),
        seq,
        scalars: Some(ScalarColumns {
            keys: vec!["loss".into()],
            key_idx: vec![0],
            step: vec![seq],
            time_ms: vec![0],
            value: vec![1.0],
        }),
        ..Default::default()
    };
    let (status, body) = sdk(server, key, "/api/v0/ingest", batch).await;
    (status, error_of(&body))
}

fn run_path(run: RunId) -> String {
    uuid::Uuid::from_bytes(*run.as_bytes()).simple().to_string()
}

macro_rules! server_with {
    ($e:expr) => {
        match $e.await {
            Some(server) => server,
            None => return,
        }
    };
}

#[tokio::test]
async fn register_login_logout() {
    let server = server_with!(TestServer::start());
    let settings = Call::get("/api/v0/auth/settings")
        .send(&server)
        .await
        .json();
    assert_eq!(
        (settings["signup"].clone(), settings["first_user"].clone()),
        (json!(true), json!(false))
    );

    let res = register(&server, "alice", "correct horse battery").await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.json());
    assert_eq!(res.json()["username"], "alice");
    assert_eq!(
        res.json()["is_admin"],
        false,
        "only the first account is admin"
    );
    let set_cookie = res.set_cookie("losscape_session").expect("session cookie");
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age="] {
        assert!(set_cookie.contains(flag), "{flag} missing in {set_cookie}");
    }
    let cookie = res.cookie("losscape_session");
    let token = cookie.trim_start_matches("losscape_session=");

    // Stored as salted Argon2id; the session as a hash of the token.
    let hash: String =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE username = 'alice'")
            .fetch_one(server.state.pg())
            .await
            .unwrap();
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{hash}"
    );
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(server.state.pg())
        .await
        .unwrap();
    assert_eq!(stored, [Sha256::digest(token.as_bytes()).to_vec()]);

    let me = Call::get("/api/v0/auth/me")
        .cookie(&cookie)
        .send(&server)
        .await;
    assert_eq!(
        (me.status, me.json()["username"].clone()),
        (StatusCode::OK, json!("alice"))
    );

    assert_eq!(
        register(&server, "ALICE", "another password").await.code(),
        "username_taken"
    );
    // How strong a password is, is up to its owner; only an empty one can't work.
    assert_eq!(
        register(&server, "bob", "").await.status,
        StatusCode::BAD_REQUEST
    );
    assert!(register(&server, "bob", "x").await.status.is_success());
    assert!(
        register(&server, "carol", "carol")
            .await
            .status
            .is_success()
    );

    // Wrong password and unknown user look the same.
    let wrong = login(&server, "alice", "wrong password!").await;
    let unknown = login(&server, "nobody", "wrong password!").await;
    assert_eq!(
        (wrong.status, wrong.bytes.clone()),
        (unknown.status, unknown.bytes.clone())
    );
    assert_eq!(wrong.code(), "invalid_credentials");

    let ok = login(&server, "Alice", "correct horse battery").await;
    assert_eq!(ok.status, StatusCode::OK, "usernames are case-insensitive");
    let second = ok.cookie("losscape_session");

    let out = Call::post("/api/v0/auth/logout", json!({}))
        .cookie(&cookie)
        .send(&server)
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(
        out.set_cookie("losscape_session")
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        Call::get("/api/v0/auth/me")
            .cookie(&cookie)
            .send(&server)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        Call::get("/api/v0/auth/me")
            .cookie(&second)
            .send(&server)
            .await
            .status,
        StatusCode::OK
    );
    server.stop().await;
}

#[tokio::test]
async fn login_attempts_are_rate_limited() {
    let server = server_with!(TestServer::start());
    register(&server, "alice", "correct horse battery").await;
    for _ in 0..10 {
        assert_eq!(
            login(&server, "alice", "wrong password!").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let blocked = login(&server, "alice", "correct horse battery").await;
    assert_eq!(
        (blocked.status, blocked.code()),
        (StatusCode::TOO_MANY_REQUESTS, "too_many_attempts".into())
    );
    server.stop().await;
}

#[tokio::test]
async fn first_user_is_admin_and_adopts_projects() {
    let auth = AuthConfig {
        signup: Signup::Closed,
        ..Default::default()
    };
    let server = server_with!(TestServer::start_bare(auth));
    // A project from before accounts existed.
    sqlx::query("INSERT INTO projects (name) VALUES ('legacy')")
        .execute(server.state.pg())
        .await
        .unwrap();
    let settings = Call::get("/api/v0/auth/settings")
        .send(&server)
        .await
        .json();
    assert_eq!(
        (settings["signup"].clone(), settings["first_user"].clone()),
        (json!(true), json!(true))
    );

    let first = register(&server, "root", "correct horse battery").await;
    assert_eq!(
        (first.status, first.json()["is_admin"].clone()),
        (StatusCode::CREATED, json!(true))
    );
    let role: String = sqlx::query_scalar(
        "SELECT m.role FROM project_members m JOIN projects p ON p.id = m.project_id WHERE p.name = 'legacy'",
    )
    .fetch_one(server.state.pg())
    .await
    .unwrap();
    assert_eq!(role, "owner");

    assert_eq!(
        Call::get("/api/v0/auth/settings")
            .send(&server)
            .await
            .json()["signup"],
        false
    );
    assert_eq!(
        register(&server, "second", "correct horse battery")
            .await
            .code(),
        "signup_closed"
    );
    server.stop().await;
}

#[tokio::test]
async fn cross_site_requests_and_injection_are_rejected() {
    let server = server_with!(TestServer::start());
    let cookie = register(&server, "alice", "correct horse battery")
        .await
        .cookie("losscape_session");
    let new_key = |site| {
        Call::post("/api/v0/me/api-keys", json!({"name": "ci"}))
            .cookie(&cookie)
            .site(site)
    };

    assert_eq!(
        new_key(Some("cross-site")).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        new_key(Some("same-site")).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    let mut evil = new_key(None);
    evil.origin = Some("https://evil.example");
    assert_eq!(evil.send(&server).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        new_key(Some("same-origin")).send(&server).await.status,
        StatusCode::CREATED
    );
    let cross_login = Call::post(
        "/api/v0/auth/login",
        json!({"username": "alice", "password": "correct horse battery"}),
    )
    .site(Some("cross-site"))
    .send(&server)
    .await;
    assert_eq!(cross_login.status, StatusCode::FORBIDDEN);

    // SQL injection attempts are data: rejected or not found, never executed.
    for name in [
        "x' OR '1'='1",
        "a'; DROP TABLE users; --",
        "<script>alert(1)</script>",
    ] {
        assert_eq!(
            register(&server, name, "correct horse battery")
                .await
                .status,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            login(&server, name, "' OR 1=1 --").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    create_project(&server, &server.api_key, "proj").await;
    let (_, run, _) = init_run(&server, &server.api_key, "proj").await;
    let res = Call::new(
        Method::PUT,
        "/api/v0/projects/proj/members/a'%3B%20DROP%20TABLE%20users%3B--",
    )
    .bearer(&server.api_key)
    .send(&server)
    .await;
    assert!(
        res.status.is_client_error(),
        "missing body is rejected before any query: {}",
        res.status
    );
    let res = Call {
        body: Some(json!({"role": "viewer"})),
        ..Call::new(
            Method::PUT,
            "/api/v0/projects/proj/members/a'%3B%20DROP%20TABLE%20users%3B--",
        )
    }
    .bearer(&server.api_key)
    .send(&server)
    .await;
    assert_eq!(res.code(), "unknown_user");
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    assert_eq!(users, 2, "admin + alice, table intact");

    // Hostile strings come back as JSON data, with headers that stop browsers from
    // sniffing or framing them.
    let hostile = r#"</script><img src=x onerror=alert(1)>"#;
    let batch = LogBatch {
        run_id: run.as_bytes().to_vec(),
        config_json: json!({ "note": hostile }).to_string(),
        ..Default::default()
    };
    assert_eq!(
        sdk(&server, &server.api_key, "/api/v0/ingest", batch)
            .await
            .0,
        StatusCode::OK
    );
    let detail = Call::get(format!("/api/v0/runs/{}", run_path(run)))
        .bearer(&server.api_key)
        .send(&server)
        .await;
    assert_eq!(detail.json()["config"]["note"], hostile);
    assert_eq!(detail.headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(detail.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(detail.headers[header::X_FRAME_OPTIONS], "DENY");
    let csp = detail.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(
        csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    server.stop().await;
}

#[tokio::test]
async fn api_keys_are_created_used_and_revoked() {
    let server = server_with!(TestServer::start());
    let cookie = register(&server, "alice", "correct horse battery")
        .await
        .cookie("losscape_session");

    let created = Call::post("/api/v0/me/api-keys", json!({"name": "laptop"}))
        .cookie(&cookie)
        .send(&server)
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let key = created.json()["secret"].as_str().unwrap().to_owned();
    assert!(key.starts_with("df_") && key.len() == 46);

    let list = Call::get("/api/v0/me/api-keys")
        .cookie(&cookie)
        .send(&server)
        .await
        .json();
    assert_eq!(list[0]["name"], "laptop");
    assert_eq!(list[0]["prefix"], &key[..10]);
    assert!(list[0].get("secret").is_none(), "secrets are never listed");
    let stored: Vec<u8> = sqlx::query_scalar("SELECT key_hash FROM api_keys WHERE name = 'laptop'")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        stored,
        Sha256::digest(key.as_bytes()).to_vec(),
        "only the hash is stored"
    );

    create_project(&server, &key, "alice-proj").await;
    assert_eq!(
        init_run(&server, &key, "alice-proj").await.0,
        StatusCode::OK
    );
    // An API key can't change the password (that needs the web session).
    let res = Call::post(
        "/api/v0/me/password",
        json!({"current": "correct horse battery", "new": "another long one"}),
    )
    .bearer(&key)
    .send(&server)
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);

    let id = list[0]["id"].as_i64().unwrap();
    let revoke = Call::new(Method::DELETE, format!("/api/v0/me/api-keys/{id}"))
        .cookie(&cookie)
        .send(&server)
        .await;
    assert_eq!(revoke.status, StatusCode::NO_CONTENT);
    assert_eq!(
        init_run(&server, &key, "alice-proj").await.0,
        StatusCode::UNAUTHORIZED,
        "revoked at once"
    );
    assert_eq!(
        init_run(&server, "df_not-a-real-key", "x").await.0,
        StatusCode::UNAUTHORIZED
    );
    server.stop().await;
}

#[tokio::test]
async fn password_change_signs_out_other_sessions() {
    let server = server_with!(TestServer::start());
    let a = register(&server, "alice", "correct horse battery")
        .await
        .cookie("losscape_session");
    let b = login(&server, "alice", "correct horse battery")
        .await
        .cookie("losscape_session");
    let wrong = Call::post(
        "/api/v0/me/password",
        json!({"current": "nope nope nope", "new": "brand new password"}),
    )
    .cookie(&a)
    .send(&server)
    .await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    let ok = Call::post(
        "/api/v0/me/password",
        json!({"current": "correct horse battery", "new": "brand new password"}),
    )
    .cookie(&a)
    .send(&server)
    .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);
    assert_eq!(
        Call::get("/api/v0/auth/me")
            .cookie(&a)
            .send(&server)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        Call::get("/api/v0/auth/me")
            .cookie(&b)
            .send(&server)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&server, "alice", "brand new password").await.status,
        StatusCode::OK
    );
    server.stop().await;
}

fn transfer_call<'a>(key: &'a str, to: &str, then: &str) -> Call<'a> {
    Call::post(
        "/api/v0/projects/p/transfer",
        json!({ "username": to, "then": then }),
    )
    .bearer(key)
}

fn role_call<'a>(key: &'a str, who: &str, role: &str) -> Call<'a> {
    Call {
        body: Some(json!({ "role": role })),
        ..Call::new(Method::PUT, format!("/api/v0/projects/p/members/{who}"))
    }
    .bearer(key)
}

async fn role_in_p(server: &TestServer, key: &str) -> Value {
    Call::get("/api/v0/projects/p")
        .bearer(key)
        .send(server)
        .await
        .json()["role"]
        .clone()
}

#[tokio::test]
async fn owners_transfer_ownership() {
    let server = server_with!(TestServer::start());
    let (_, alice) = user(&server.state, "alice").await;
    let (_, bob) = user(&server.state, "bob").await;
    let (_, carol) = user(&server.state, "carol").await;
    create_project(&server, &alice, "p").await;
    role_call(&alice, "bob", "editor").send(&server).await;

    // Only owners, only to members, not to oneself, and the old owner steps down.
    assert_eq!(
        transfer_call(&bob, "bob", "editor")
            .send(&server)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        transfer_call(&alice, "carol", "editor")
            .send(&server)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        transfer_call(&alice, "alice", "editor")
            .send(&server)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        transfer_call(&alice, "bob", "owner")
            .send(&server)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        transfer_call(&alice, "bob", "viewer")
            .send(&server)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(role_in_p(&server, &bob).await, "owner");
    assert_eq!(role_in_p(&server, &alice).await, "viewer");

    // An admin can transfer too: the owner (not the admin) steps down.
    let admin = server.api_key.clone();
    assert_eq!(
        transfer_call(&admin, "alice", "editor")
            .send(&server)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(role_in_p(&server, &alice).await, "owner");
    assert_eq!(role_in_p(&server, &bob).await, "editor");
    transfer_call(&alice, "bob", "viewer").send(&server).await;

    // Bob hands it on to carol and leaves.
    role_call(&bob, "carol", "viewer").send(&server).await;
    assert_eq!(
        transfer_call(&bob, "carol", "leave")
            .send(&server)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(role_in_p(&server, &carol).await, "owner");
    assert_eq!(
        Call::get("/api/v0/projects/p")
            .bearer(&bob)
            .send(&server)
            .await
            .status,
        StatusCode::NOT_FOUND,
        "bob left"
    );
    server.stop().await;
}

async fn list_runs(server: &TestServer, key: &str, archived: bool) -> Response {
    Call::get(format!("/api/v0/projects/p/runs?archived={archived}"))
        .bearer(key)
        .send(server)
        .await
}

fn archive_call<'a>(key: &'a str, action: &str, run: RunId) -> Call<'a> {
    Call::post(
        format!("/api/v0/projects/p/runs/{action}"),
        json!({ "runs": [run_path(run)] }),
    )
    .bearer(key)
}

fn patch<'a>(key: &'a str, days: i64) -> Call<'a> {
    Call {
        body: Some(json!({ "archive_days": days })),
        ..Call::new(Method::PATCH, "/api/v0/projects/p")
    }
    .bearer(key)
}

#[tokio::test]
async fn archived_runs_are_hidden_restorable_then_purged() {
    let server = server_with!(TestServer::start());
    let (_, alice) = user(&server.state, "alice").await;
    let (bob_id, bob) = user(&server.state, "bob").await;
    assert_eq!(
        create_project(&server, &alice, "p").await.status,
        StatusCode::CREATED
    );
    let project_id: i64 = sqlx::query_scalar("SELECT id FROM projects WHERE name = 'p'")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role) VALUES ($1, $2, 'viewer')",
    )
    .bind(project_id)
    .bind(bob_id)
    .execute(server.state.pg())
    .await
    .unwrap();
    let (_, keep, _) = init_run(&server, &alice, "p").await;
    let (_, old, _) = init_run(&server, &alice, "p").await;
    for run in [keep, old] {
        assert_eq!(log_point(&server, &alice, run, 0).await.0, StatusCode::OK);
    }
    let names = |runs: &Value| -> Vec<String> {
        runs.get("runs")
            .unwrap_or(runs)
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().replace('-', ""))
            .collect()
    };
    // Viewers can't archive; editors can.
    let (alice, bob) = (alice.as_str(), bob.as_str());
    let list = |archived: bool| list_runs(&server, alice, archived);
    assert_eq!(
        archive_call(bob, "archive", old).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    // Nor change tags; they can read them.
    let tag = |user| {
        Call::post(
            "/api/v0/projects/p/runs/tags",
            json!({"runs": [run_path(old)], "add": ["x"]}),
        )
        .bearer(user)
    };
    assert_eq!(tag(bob).send(&server).await.status, StatusCode::FORBIDDEN);
    let rename = Call::post(
        "/api/v0/projects/p/runs/metric-renames",
        json!({"runs": [run_path(old)], "from": "loss", "to": "train/loss"}),
    )
    .bearer(bob);
    assert_eq!(rename.send(&server).await.status, StatusCode::FORBIDDEN);
    let abort = Call::post(
        "/api/v0/projects/p/runs/abort",
        json!({"runs": [run_path(old)]}),
    )
    .bearer(bob);
    assert_eq!(abort.send(&server).await.status, StatusCode::FORBIDDEN);
    let style = Call::put("/api/v0/projects/p/tags/x", json!({"color": "red"})).bearer(bob);
    assert_eq!(style.send(&server).await.status, StatusCode::FORBIDDEN);
    let read = Call::get("/api/v0/projects/p/tags").bearer(bob);
    assert_eq!(read.send(&server).await.status, StatusCode::OK);
    assert_eq!(tag(alice).send(&server).await.json()["changed"], 1);
    let res = archive_call(alice, "archive", old).send(&server).await;
    assert_eq!(res.json()["changed"], 1);
    assert_eq!(names(&list(false).await.json()), [run_path(keep)]);
    let archived = list(true).await.json()["runs"].clone();
    assert_eq!(names(&archived), [run_path(old)]);
    assert!(archived[0]["archived_at_ms"].as_i64().is_some());
    let info = Call::get("/api/v0/projects/p")
        .bearer(alice)
        .send(&server)
        .await
        .json();
    assert_eq!(
        (info["runs"].clone(), info["archived_runs"].clone()),
        (json!(1), json!(1))
    );

    // Restore, archive again.
    assert_eq!(
        archive_call(alice, "restore", old)
            .send(&server)
            .await
            .json()["changed"],
        1
    );
    assert_eq!(names(&list(true).await.json()), Vec::<String>::new());
    archive_call(alice, "archive", old).send(&server).await;

    // Retention: owners only, 1-3650 days.
    assert_eq!(
        patch(bob, 5).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        patch(alice, 0).send(&server).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        patch(alice, 7).send(&server).await.status,
        StatusCode::NO_CONTENT
    );
    let info = Call::get("/api/v0/projects/p")
        .bearer(alice)
        .send(&server)
        .await
        .json();
    assert_eq!(info["archive_days"], 7);

    // Purged once older than the project's retention, metrics included.
    assert_eq!(
        losscape_server::purge::purge_archived(&server.state)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE runs SET archived_at = now() - interval '8 days' WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(*old.as_bytes()))
        .execute(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        losscape_server::purge::purge_archived(&server.state)
            .await
            .unwrap(),
        1
    );
    assert_eq!(names(&list(true).await.json()), Vec::<String>::new());
    let points: u64 = server
        .state
        .ch()
        .query("SELECT count() FROM metrics WHERE run_id = toUUID(?)")
        .bind(run_path(old))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(points, 0);
    assert_eq!(
        Call::get(format!("/api/v0/runs/{}", run_path(old)))
            .bearer(alice)
            .send(&server)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // Deleting the project: owners only; runs and memberships go with it.
    let delete = |key| Call::new(Method::DELETE, "/api/v0/projects/p").bearer(key);
    assert_eq!(
        delete(bob).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        delete(alice).send(&server).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        Call::get("/api/v0/projects")
            .bearer(bob)
            .send(&server)
            .await
            .json(),
        json!([])
    );
    assert_eq!(
        Call::get(format!("/api/v0/runs/{}", run_path(keep)))
            .bearer(alice)
            .send(&server)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        log_point(&server, alice, keep, 1).await.0,
        StatusCode::NOT_FOUND,
        "no logging into a deleted project's run"
    );
    // The name is free again.
    assert_eq!(
        create_project(&server, alice, "p").await.status,
        StatusCode::CREATED
    );
    server.stop().await;
}

#[tokio::test]
async fn project_roles_control_access() {
    let server = server_with!(TestServer::start());
    let (_, alice) = user(&server.state, "alice").await;
    let (_, bob) = user(&server.state, "bob").await;
    let (_, carol) = user(&server.state, "carol").await;

    // The SDK doesn't create projects: they're created explicitly, and the creator owns it.
    let (status, _, code) = init_run(&server, &alice, "alice-proj").await;
    assert_eq!(
        (status, code.starts_with("unknown_project")),
        (StatusCode::NOT_FOUND, true),
        "{code}"
    );
    assert_eq!(
        create_project(&server, &alice, "alice-proj").await.status,
        StatusCode::CREATED
    );
    let taken = create_project(&server, &bob, "alice-proj").await;
    assert_eq!(
        (taken.status, taken.json()["code"].clone()),
        (StatusCode::CONFLICT, json!("project_exists"))
    );
    for bad in ["", "a/b", "../x", "<script>"] {
        assert_eq!(
            create_project(&server, &alice, bad).await.status,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    // Others can't log into it.
    let (status, run, _) = init_run(&server, &alice, "alice-proj").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(log_point(&server, &alice, run, 0).await.0, StatusCode::OK);
    let (status, _, code) = init_run(&server, &bob, "alice-proj").await;
    assert_eq!(
        (status, code.starts_with("forbidden")),
        (StatusCode::FORBIDDEN, true)
    );
    assert_eq!(
        log_point(&server, &bob, run, 1).await.0,
        StatusCode::NOT_FOUND,
        "run hidden from non-members"
    );

    // Non-members don't see the project or its runs.
    let projects = Call::get("/api/v0/projects")
        .bearer(&bob)
        .send(&server)
        .await
        .json();
    assert_eq!(projects, json!([]));
    let hidden = [
        Call::get("/api/v0/projects/alice-proj/runs"),
        Call::get(format!("/api/v0/runs/{}", run_path(run))),
        Call::get(format!(
            "/api/v0/projects/alice-proj/live?runs={}",
            run_path(run)
        )),
        Call::post(
            "/api/v0/projects/alice-proj/metrics",
            json!({ "runs": [run_path(run)] }),
        ),
        Call::get("/api/v0/projects/alice-proj/members"),
    ];
    for call in hidden {
        assert_eq!(
            call.bearer(&bob).send(&server).await.status,
            StatusCode::NOT_FOUND
        );
    }
    let series = json!({"ranges": [{"run": run_path(run), "key": "loss", "level": 0, "from_tile": 0, "to_tile": 1}]});
    assert_eq!(
        Call::post("/api/v0/series?format=json", series.clone())
            .bearer(&bob)
            .send(&server)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // Viewer: reads, can't log, can't manage members or change the board.
    let set = |who: &str, role: &str| Call {
        body: Some(json!({ "role": role })),
        ..Call::new(
            Method::PUT,
            format!("/api/v0/projects/alice-proj/members/{who}"),
        )
    };
    assert_eq!(
        set("bob", "viewer")
            .bearer(&alice)
            .send(&server)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let put_board = || Call {
        body: Some(
            json!({"version": 0, "query": "", "sort": {"id": "created", "desc": true}, "limit": 5}),
        ),
        ..Call::new(Method::PUT, "/api/v0/projects/alice-proj/board")
    };
    assert_eq!(
        put_board().bearer(&bob).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        put_board().bearer(&alice).send(&server).await.status,
        StatusCode::OK
    );
    assert_eq!(
        Call::get("/api/v0/projects/alice-proj/board")
            .bearer(&bob)
            .send(&server)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        Call::get("/api/v0/projects/alice-proj/runs")
            .bearer(&bob)
            .send(&server)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        Call::post("/api/v0/series?format=json", series)
            .bearer(&bob)
            .send(&server)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        log_point(&server, &bob, run, 1).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        set("carol", "viewer")
            .bearer(&bob)
            .send(&server)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let projects = Call::get("/api/v0/projects")
        .bearer(&bob)
        .send(&server)
        .await
        .json();
    assert_eq!(
        (projects[0]["name"].clone(), projects[0]["role"].clone()),
        (json!("alice-proj"), json!("viewer"))
    );

    // Editor: logs.
    assert_eq!(
        set("bob", "editor")
            .bearer(&alice)
            .send(&server)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        log_point(&server, &bob, run, 1).await.0,
        StatusCode::OK,
        "role change applies at once"
    );
    assert_eq!(
        init_run(&server, &bob, "alice-proj").await.0,
        StatusCode::OK
    );

    // One owner, fixed outside of transfers; members can leave; only owners remove others.
    let leave = |who: &str| {
        Call::new(
            Method::DELETE,
            format!("/api/v0/projects/alice-proj/members/{who}"),
        )
    };
    assert_eq!(
        leave("alice").bearer(&alice).send(&server).await.code(),
        "owner_fixed"
    );
    assert_eq!(
        set("alice", "viewer")
            .bearer(&alice)
            .send(&server)
            .await
            .code(),
        "owner_fixed"
    );
    assert_eq!(
        set("bob", "owner")
            .bearer(&alice)
            .send(&server)
            .await
            .status,
        StatusCode::BAD_REQUEST,
        "no second owner"
    );
    assert_eq!(
        leave("alice")
            .bearer(&server.api_key)
            .send(&server)
            .await
            .code(),
        "owner_fixed",
        "not even an admin removes the owner"
    );
    assert_eq!(
        leave("alice").bearer(&bob).send(&server).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        set("carol", "viewer")
            .bearer(&alice)
            .send(&server)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        leave("carol").bearer(&carol).send(&server).await.status,
        StatusCode::NO_CONTENT
    );
    let members = Call::get("/api/v0/projects/alice-proj/members")
        .bearer(&alice)
        .send(&server)
        .await
        .json();
    let names: Vec<_> = members["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["username"].clone())
        .collect();
    assert_eq!(names, [json!("alice"), json!("bob")], "owners first");

    // The admin sees every project.
    let all = Call::get("/api/v0/projects")
        .bearer(&server.api_key)
        .send(&server)
        .await
        .json();
    assert_eq!(all[0]["name"], "alice-proj");
    server.stop().await;
}

// ---- OAuth: fake providers -------------------------------------------------------

/// Throwaway key generated for these tests only.
const TEST_RSA_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDsz+VAAA6dHitf
xFVFiH9xyLm1Y/2RJaEdVcwLwyo1nvMFX6i75R5wItkjfM871U2Vmai2VwuDVted
5fNSSFCxgMQC5AdSiRtDD6JPnkDs2hNFM8XESoi/g3jDiW8dZtK0UWBB57WQ6A1b
N8FakT+iXMtMRTlARMP52jWe1lRqGUCxoozWLoSHSziT2VWel6SyvrPzd5vgKZQ5
0Iojm2dUQOgJZMYP7wXYAY6Xs1d1PSpAVlMJH9gNGXN4R1JH4vPRHhl75N5XQX0I
cj8XMvgSCqPNlfSASkKCRvg6OOX7rk+4DwStN5rCCkoxciBokS+QtdxGaDpF2y7x
IjEfr+wjAgMBAAECggEAFh/S2GNeT3yG6zK8ZWYmw0/QVdY1Ku8xObyPIdbb0IL8
oRcPpxpKo98Fmq9UQc/1qHvgy270yDz/9k8KUrjR3SYyvzMO92g6z/hGdETMkQmg
vO7k06gZADd9ZrCmMmuC/6NjUv+dsso98TSTem173KSUaRhF4doVgHghWVPHMtri
GyBkF1pImdBwPPOWBp4V8MU4OBnoXWIcMn2X4pyEdwicCq5RWxcCPt/l3VRUzYGf
9uDpVAXxtGy9U3VdvOfkB7DEz9fA+Z52FHq2ubAwi6jwpR+9hcDWd/K0fIGmkzMg
dHttJTdyxpYwZbWqUrYWNSBqMB+4KoiYAH1tt5ZVjQKBgQD7KD9vU+HX2aLu6YhI
S1wCFgtcefvAkq+odkqouv8iHd0WEMP2QvCD5w1f9aFM0mIAIKEdT+XEzO8H1KFH
dZ8gcMQf95o9OM+JY42op/sE4i3qFs1e8yIp0DQ+2mRhjtSaQyE7vjGGPQaPATF1
uI+AeNRhl0LR1ggTZokdYR7oXQKBgQDxYNZ/KyoM3BuAf2e/znjp0JaBtfqGC6Ac
lmWmTr9jXqoZ0VbFODPWSQdwny1msHDOaXQEsHTElRJXSHjp0HnDoZk4YafTenA1
UTlw3Vn872XNlBCFNeynPLzx+iljvhD2UIYN3dl9RlzZ9VwO8NoOygDS+Qsf8L56
v3MY7aLefwKBgEVKsF2lldx4ZfRmJ7xSVjsZG99SpDtX/PCMWkK8/KDL/nAVBguO
Ik8x0sXTvcG1FWaVmf29SlYGeDf/2NMfR1yq3+E17a7QWqm79pw+ZAnP4P0Bd32n
tJg2ogNpO2KTBmLc/HFAIue3NHUFzkB9P9IhA+88zSmZVDcOBew+HEndAoGAcmwg
Yjo4/c2db+V1bhRXdyc9IZH9h2udY8/bseGkklt+sSxuQKFq2UrXLwkWAZbkcsTE
jbCEOegcwKmHP/dmsVk6g+KvJu3koTG21AzqKp/ll3V3+13bqE9/lgmrcy/XZJw5
X/tprRwcyemo0SBVjisuBtSt9F8wPIni6s9DpMsCgYEA1ZyBRBNReYWlNTAGWW7q
vHjYc9/M6Tk8LFCivQoaDXS/uaF1yE4ApyAyLn7zbB14Je0o/S3MfNd80TZzT5CB
epM42uRKdJLfgQ+NQTZH/1b2PxAD+COQUszy1lnX793gKjX1xaEJAnUNwMDKdSUy
+4j0ynzZbb17t5LjR8hcrKs=
-----END PRIVATE KEY-----"#;
const TEST_RSA_N: &str = "7M_lQAAOnR4rX8RVRYh_cci5tWP9kSWhHVXMC8MqNZ7zBV-ou-UecCLZI3zPO9VNlZmotlcLg1bXneXzUkhQsYDEAuQHUokbQw-iT55A7NoTRTPFxEqIv4N4w4lvHWbStFFgQee1kOgNWzfBWpE_olzLTEU5QETD-do1ntZUahlAsaKM1i6Eh0s4k9lVnpeksr6z83eb4CmUOdCKI5tnVEDoCWTGD-8F2AGOl7NXdT0qQFZTCR_YDRlzeEdSR-Lz0R4Ze-TeV0F9CHI_FzL4EgqjzZX0gEpCgkb4Ojjl-65PuA8ErTeawgpKMXIgaJEvkLXcRmg6Rdsu8SIxH6_sIw";

#[derive(Default)]
struct Provider {
    url: String,
    /// code -> (nonce to put in the ID token, PKCE challenge, subject)
    codes: Mutex<HashMap<String, (String, String, String)>>,
}

type Shared = Arc<Provider>;

/// An OpenID Connect provider: discovery, JWKS, and a token endpoint that checks the
/// client secret and PKCE verifier and signs an RS256 ID token.
async fn fake_oidc() -> Shared {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let shared = Arc::new(Provider {
        url,
        ..Default::default()
    });

    async fn discovery(State(p): State<Shared>) -> axum::Json<Value> {
        axum::Json(json!({
            "issuer": p.url,
            "authorization_endpoint": format!("{}/authorize", p.url),
            "token_endpoint": format!("{}/token", p.url),
            "jwks_uri": format!("{}/jwks", p.url),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        }))
    }
    async fn jwks() -> axum::Json<Value> {
        axum::Json(
            json!({"keys": [{"kty": "RSA", "use": "sig", "alg": "RS256", "kid": "test", "n": TEST_RSA_N, "e": "AQAB"}]}),
        )
    }
    async fn token(
        State(p): State<Shared>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> (StatusCode, axum::Json<Value>) {
        let basic = base64::engine::general_purpose::STANDARD.encode("client:secret");
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some(&format!("Basic {basic}"))
        {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({"error": "invalid_client"})),
            );
        }
        let Some((nonce, challenge, subject)) = p.codes.lock().unwrap().remove(&form["code"])
        else {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"error": "invalid_grant"})),
            );
        };
        if pkce(&form["code_verifier"]) != challenge {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"error": "invalid_grant", "error_description": "PKCE"})),
            );
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = json!({
            "iss": p.url, "sub": subject, "aud": "client", "iat": now, "exp": now + 300, "nonce": nonce,
            "email": "jane@example.com", "email_verified": true, "preferred_username": "jane", "name": "Jane Doe",
        });
        let mut jwt_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        jwt_header.kid = Some("test".into());
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_RSA_KEY.as_bytes()).unwrap();
        let id_token = jsonwebtoken::encode(&jwt_header, &claims, &key).unwrap();
        (
            StatusCode::OK,
            axum::Json(
                json!({"access_token": "at", "token_type": "Bearer", "expires_in": 300, "id_token": id_token}),
            ),
        )
    }

    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(shared.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    shared
}

/// GitHub's OAuth endpoints and user API.
async fn fake_github() -> Shared {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let shared = Arc::new(Provider {
        url,
        ..Default::default()
    });

    async fn token(
        State(p): State<Shared>,
        Form(form): Form<HashMap<String, String>>,
    ) -> axum::Json<Value> {
        let ok = form.get("client_secret").map(String::as_str) == Some("gh-secret")
            && p.codes
                .lock()
                .unwrap()
                .remove(&form["code"])
                .is_some_and(|(_, challenge, _)| pkce(&form["code_verifier"]) == challenge);
        axum::Json(if ok {
            json!({"access_token": "gh-token", "token_type": "bearer"})
        } else {
            json!({"error": "bad_verification_code"})
        })
    }
    async fn me(headers: HeaderMap) -> (StatusCode, axum::Json<Value>) {
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some("Bearer gh-token")
        {
            return (StatusCode::UNAUTHORIZED, axum::Json(json!({})));
        }
        (
            StatusCode::OK,
            axum::Json(
                json!({"id": 583231, "login": "octocat", "name": "The Octocat", "email": "public@unverified.example"}),
            ),
        )
    }
    async fn emails() -> axum::Json<Value> {
        axum::Json(json!([
            {"email": "old@example.com", "primary": false, "verified": true},
            {"email": "octo@example.com", "primary": true, "verified": true},
        ]))
    }
    let app = Router::new()
        .route("/login/oauth/access_token", post(token))
        .route("/user", get(me))
        .route("/user/emails", get(emails))
        .with_state(shared.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    shared
}

fn pkce(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn query(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

fn auth_config(oidc: &Provider, github: &Provider) -> AuthConfig {
    AuthConfig {
        trusted_proxies: Vec::new(),
        public_url: Some("http://losscape.test".into()),
        signup: Signup::Open,
        oauth: vec![
            OAuthProvider {
                id: "corp".into(),
                name: "Corp SSO".into(),
                kind: OAuthKind::Oidc {
                    issuer: oidc.url.clone(),
                },
                client_id: "client".into(),
                client_secret: "secret".into(),
            },
            OAuthProvider {
                id: "github".into(),
                name: "GitHub".into(),
                kind: OAuthKind::GitHub {
                    web_url: github.url.clone(),
                    api_url: github.url.clone(),
                },
                client_id: "gh-client".into(),
                client_secret: "gh-secret".into(),
            },
        ],
    }
}

/// Starts a sign-in; returns (provider authorize URL params, state cookie).
async fn start_oauth(
    server: &TestServer,
    provider: &str,
    session: Option<&str>,
) -> (HashMap<String, String>, String) {
    let mut call = Call::get(format!(
        "/api/v0/auth/oauth/{provider}/start?return_to=/p/demo"
    ));
    call.cookie = session;
    let res = call.send(server).await;
    assert_eq!(
        res.status,
        StatusCode::SEE_OTHER,
        "{}",
        String::from_utf8_lossy(&res.bytes)
    );
    let cookie = res.set_cookie("losscape_oauth").expect("state cookie");
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/api/v0/auth/oauth"] {
        assert!(cookie.contains(flag), "{cookie}");
    }
    (query(&res.location()), res.cookie("losscape_oauth"))
}

async fn callback(
    server: &TestServer,
    provider: &str,
    code: &str,
    state: &str,
    cookie: &str,
) -> Response {
    Call::get(format!(
        "/api/v0/auth/oauth/{provider}/callback?code={code}&state={state}"
    ))
    .cookie(cookie)
    .site(Some("cross-site"))
    .send(server)
    .await
}

#[tokio::test]
async fn oidc_sign_in() {
    let (oidc, github) = (fake_oidc().await, fake_github().await);
    let server = server_with!(TestServer::start_with(auth_config(&oidc, &github)));
    let providers = Call::get("/api/v0/auth/settings")
        .send(&server)
        .await
        .json()["providers"]
        .clone();
    assert_eq!(
        providers,
        json!([
            {"id": "corp", "name": "Corp SSO", "icon": "key"},
            {"id": "github", "name": "GitHub", "icon": "github"},
        ])
    );

    let (params, state_cookie) = start_oauth(&server, "corp", None).await;
    assert_eq!(params["client_id"], "client");
    assert_eq!(
        params["redirect_uri"],
        "http://losscape.test/api/v0/auth/oauth/corp/callback"
    );
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(params["scope"].contains("openid") && params["scope"].contains("email"));
    oidc.codes.lock().unwrap().insert(
        "c1".into(),
        (
            params["nonce"].clone(),
            params["code_challenge"].clone(),
            "sub-1".into(),
        ),
    );

    // The provider's redirect is a cross-site top-level navigation.
    let res = callback(&server, "corp", "c1", &params["state"], &state_cookie).await;
    assert_eq!(
        (res.status, res.location()),
        (StatusCode::SEE_OTHER, "/p/demo".to_owned())
    );
    let session = res.cookie("losscape_session");
    let me = Call::get("/api/v0/auth/me")
        .cookie(&session)
        .send(&server)
        .await
        .json();
    assert_eq!(
        (me["username"].clone(), me["email"].clone()),
        (json!("jane"), json!("jane@example.com"))
    );
    assert_eq!(
        (me["has_password"].clone(), me["identities"].clone()),
        (json!(false), json!(["corp"]))
    );

    // The state works once.
    let replay = callback(&server, "corp", "c1", &params["state"], &state_cookie).await;
    assert_eq!(replay.location(), "/login?error=oauth_state");

    // Signing in again finds the same account.
    let (params, state_cookie) = start_oauth(&server, "corp", None).await;
    oidc.codes.lock().unwrap().insert(
        "c2".into(),
        (
            params["nonce"].clone(),
            params["code_challenge"].clone(),
            "sub-1".into(),
        ),
    );
    let again = callback(&server, "corp", "c2", &params["state"], &state_cookie).await;
    let me = Call::get("/api/v0/auth/me")
        .cookie(&again.cookie("losscape_session"))
        .send(&server)
        .await
        .json();
    assert_eq!(me["username"], "jane");
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    assert_eq!(users, 2, "admin + jane");

    // A callback link started in another browser (no matching state cookie) is refused.
    let (params, _) = start_oauth(&server, "corp", None).await;
    let (_, other_cookie) = start_oauth(&server, "corp", None).await;
    oidc.codes.lock().unwrap().insert(
        "c3".into(),
        (
            params["nonce"].clone(),
            params["code_challenge"].clone(),
            "sub-attacker".into(),
        ),
    );
    let forged = callback(&server, "corp", "c3", &params["state"], &other_cookie).await;
    assert_eq!(forged.location(), "/login?error=oauth_state");
    assert!(forged.set_cookie("losscape_session").is_none());

    // An ID token with the wrong nonce is rejected.
    let (params, state_cookie) = start_oauth(&server, "corp", None).await;
    oidc.codes.lock().unwrap().insert(
        "c4".into(),
        (
            "wrong-nonce".into(),
            params["code_challenge"].clone(),
            "sub-2".into(),
        ),
    );
    let bad = callback(&server, "corp", "c4", &params["state"], &state_cookie).await;
    assert_eq!(bad.location(), "/login?error=oauth_failed");

    // Signed in, the flow links the identity to the current account instead.
    let alice = register(&server, "alice", "correct horse battery")
        .await
        .cookie("losscape_session");
    let (params, state_cookie) = start_oauth(&server, "corp", Some(&alice)).await;
    oidc.codes.lock().unwrap().insert(
        "c5".into(),
        (
            params["nonce"].clone(),
            params["code_challenge"].clone(),
            "sub-alice".into(),
        ),
    );
    callback(&server, "corp", "c5", &params["state"], &state_cookie).await;
    let me = Call::get("/api/v0/auth/me")
        .cookie(&alice)
        .send(&server)
        .await
        .json();
    assert_eq!(me["identities"], json!(["corp"]));
    server.stop().await;
}

#[tokio::test]
async fn github_sign_in() {
    let (oidc, github) = (fake_oidc().await, fake_github().await);
    let server = server_with!(TestServer::start_with(auth_config(&oidc, &github)));
    let (params, state_cookie) = start_oauth(&server, "github", None).await;
    assert_eq!(params["client_id"], "gh-client");
    assert_eq!(params["scope"], "read:user user:email");
    github.codes.lock().unwrap().insert(
        "g1".into(),
        (
            String::new(),
            params["code_challenge"].clone(),
            String::new(),
        ),
    );

    let res = callback(&server, "github", "g1", &params["state"], &state_cookie).await;
    assert_eq!(res.location(), "/p/demo");
    let me = Call::get("/api/v0/auth/me")
        .cookie(&res.cookie("losscape_session"))
        .send(&server)
        .await
        .json();
    assert_eq!(me["username"], "octocat");
    assert_eq!(
        me["email"], "octo@example.com",
        "primary verified email, not the public profile one"
    );
    assert_eq!(me["display_name"], "The Octocat");

    let (params, state_cookie) = start_oauth(&server, "github", None).await;
    let unknown_code = callback(
        &server,
        "github",
        "never-issued",
        &params["state"],
        &state_cookie,
    )
    .await;
    assert_eq!(unknown_code.location(), "/login?error=oauth_failed");
    server.stop().await;
}

#[tokio::test]
async fn profiles_have_display_names_and_pictures() {
    let server = server_with!(TestServer::start());
    let res = Call::post(
        "/api/v0/auth/register",
        json!({"username": "alice", "password": "correct horse battery", "display_name": "  Alice Martin  "}),
    )
    .send(&server)
    .await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.json()["display_name"], "Alice Martin", "trimmed");
    assert_eq!(res.json()["avatar_url"], Value::Null);
    let cookie = res.cookie("losscape_session");

    for bad in ["x\u{0}y", "line\nbreak", &"x".repeat(65)] {
        let res = Call::post(
            "/api/v0/auth/register",
            json!({"username": "bob", "password": "correct horse battery", "display_name": bad}),
        )
        .send(&server)
        .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{bad:?}");
    }
    let patch = |body: Value| {
        Call {
            body: Some(body),
            ..Call::new(Method::PATCH, "/api/v0/me")
        }
        .cookie(&cookie)
    };
    assert_eq!(
        patch(json!({"display_name": "Al"}))
            .send(&server)
            .await
            .json()["display_name"],
        "Al"
    );
    assert_eq!(
        patch(json!({"display_name": ""}))
            .send(&server)
            .await
            .json()["display_name"],
        Value::Null
    );

    // Upload: any PNG/JPEG/WebP comes back as a 256x256 PNG.
    let upload = |bytes: Vec<u8>| async {
        let req = Request::put("/api/v0/me/avatar")
            .header(header::COOKIE, &cookie)
            .header("sec-fetch-site", "same-origin")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(bytes))
            .unwrap();
        let res = server.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        (
            status,
            String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes())
                .into_owned(),
        )
    };
    let photo = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(900, 300, |x, y| {
        image::Rgb([x as u8, y as u8, 128])
    }));
    let mut jpeg = Vec::new();
    photo
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    assert_eq!(upload(jpeg).await.0, StatusCode::NO_CONTENT);

    let me = Call::get("/api/v0/auth/me")
        .cookie(&cookie)
        .send(&server)
        .await
        .json();
    let url = me["avatar_url"].as_str().expect("avatar url").to_owned();
    assert!(url.starts_with("/api/v0/users/alice/avatar?v="), "{url}");
    let res = Call::get(url.clone()).cookie(&cookie).send(&server).await;
    assert_eq!(res.headers[header::CONTENT_TYPE], "image/png");
    let stored = image::load_from_memory(&res.bytes).unwrap();
    assert_eq!((stored.width(), stored.height()), (256, 256));
    assert_eq!(
        Call::get(url.clone()).send(&server).await.status,
        StatusCode::UNAUTHORIZED,
        "signed-in users only"
    );

    // Anything else is refused, scripts included.
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>"#.to_vec();
    let (status, body) = upload(svg).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        upload(b"<html><script>alert(1)</script></html>".to_vec())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut huge = Vec::new();
    image::DynamicImage::new_luma8(5000, 2)
        .write_to(
            &mut std::io::Cursor::new(&mut huge),
            image::ImageFormat::Png,
        )
        .unwrap();
    assert_eq!(upload(huge).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        upload(vec![0u8; 3 * 1024 * 1024]).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        Call::get("/api/v0/auth/me")
            .cookie(&cookie)
            .send(&server)
            .await
            .json()["avatar_url"],
        url.as_str(),
        "a refused upload keeps the current picture"
    );

    let key = Call::post("/api/v0/me/api-keys", json!({"name": "k"}))
        .cookie(&cookie)
        .send(&server)
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    create_project(&server, &key, "alice-proj").await;
    init_run(&server, &key, "alice-proj").await;
    let members = Call::get("/api/v0/projects/alice-proj/members")
        .cookie(&cookie)
        .send(&server)
        .await
        .json();
    assert_eq!(members["members"][0]["avatar_url"], url.as_str());

    let res = Call::new(Method::DELETE, "/api/v0/me/avatar")
        .cookie(&cookie)
        .send(&server)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(
        Call::get("/api/v0/auth/me")
            .cookie(&cookie)
            .send(&server)
            .await
            .json()["avatar_url"],
        Value::Null
    );
    assert_eq!(
        Call::get(url).cookie(&cookie).send(&server).await.status,
        StatusCode::NOT_FOUND
    );
    server.stop().await;
}
