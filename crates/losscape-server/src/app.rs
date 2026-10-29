use axum::{Json, Router, routing::get};

pub fn router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v0/version", get(version))
}

async fn healthz() -> &'static str {
    "ok"
}

// Becomes a real check (Postgres + ClickHouse reachable) once storage is wired in.
async fn readyz() -> &'static str {
    "ok"
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

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    async fn get(uri: &str) -> (StatusCode, String) {
        let res = router()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn health_endpoints() {
        assert_eq!(get("/healthz").await, (StatusCode::OK, "ok".into()));
        assert_eq!(get("/readyz").await, (StatusCode::OK, "ok".into()));
    }

    #[tokio::test]
    async fn version_reports_protocol() {
        let (status, body) = get("/api/v0/version").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""protocol":0"#), "{body}");
    }
}
