//! Checkpoints end to end (docs/ARTIFACTS.md), against real Postgres and a storage folder.
//! Skipped when DATABASE_URL / CLICKHOUSE_URL are unset.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use losscape_core::PROTOCOL_HEADER;
use losscape_core::ids::RunId;
use losscape_core::proto::v0::RunInit;
use losscape_server::checkpoints::{self, PART_SIZE};
use prost::Message;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

mod common;

use common::{TestServer, error_of};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A run in a project of its own ("test"), as the admin.
async fn new_run(server: &TestServer) -> String {
    assert!(matches!(
        server.create_project("test").await,
        StatusCode::CREATED | StatusCode::CONFLICT
    ));
    let run = RunId::new();
    let init = RunInit {
        run_id: run.as_bytes().to_vec(),
        project: "test".into(),
        name: "ckpt-run".into(),
        ..Default::default()
    };
    let req = Request::post("/api/v0/runs/init")
        .header(header::AUTHORIZATION, server.bearer())
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header(PROTOCOL_HEADER, losscape_core::PROTOCOL_VERSION.to_string())
        .body(Body::from(init.encode_to_vec()))
        .unwrap();
    let (status, body) = server.call(req).await;
    assert_eq!(status, StatusCode::OK, "{}", error_of(&body));
    uuid::Uuid::from_bytes(*run.as_bytes()).to_string()
}

struct Client<'a> {
    server: &'a TestServer,
    key: String,
}

impl Client<'_> {
    async fn json(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.key))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let (status, bytes) = self.server.call(req).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn part(
        &self,
        id: &str,
        file: usize,
        part: usize,
        data: &[u8],
        hash: &str,
    ) -> StatusCode {
        let req = Request::put(format!(
            "/api/v0/checkpoints/{id}/files/{file}/parts/{part}"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", self.key))
        .header("x-losscape-sha256", hash)
        .body(Body::from(data.to_vec()))
        .unwrap();
        self.server.call(req).await.0
    }

    async fn download(&self, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let req = Request::get(path)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.key))
            .body(Body::empty())
            .unwrap();
        let res = self.server.router.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
        (status, headers, body)
    }
}

/// Files under the server's storage folder.
fn stored_files(server: &TestServer) -> usize {
    fn count(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .flatten()
                .map(|e| {
                    if e.path().is_dir() {
                        count(&e.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
    }
    count(&server.storage_dir)
}

#[tokio::test]
async fn upload_resume_commit_download_delete() {
    let server = server!();
    let run = new_run(&server).await;
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };

    // Two parts uncompressed, one compressed, one empty.
    let model: Vec<u8> = (0..PART_SIZE + 10).map(|i| (i % 251) as u8).collect();
    let config = br#"{"layers": 12, "hidden": 768, "vocab": 50257}"#.repeat(100);
    let config_zst = zstd::encode_all(config.as_slice(), 1).unwrap();
    let manifest = json!({"step": 1000, "files": [
        {"path": "model.pt", "size": model.len(), "sha256": sha(&model), "compression": "none", "parts": 2},
        {"path": "cfg/config.json", "size": config.len(), "sha256": sha(&config), "compression": "zstd", "parts": 1},
        {"path": "empty.txt", "size": 0, "sha256": sha(b""), "compression": "none", "parts": 0},
    ]});
    let path = format!("/api/v0/runs/{run}/checkpoints");
    let (status, created) = admin.json("POST", &path, Some(manifest.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["state"], "pending");
    assert_eq!(created["files"][0]["uploaded"], json!([]));

    let first = &model[..PART_SIZE as usize];
    assert_eq!(
        admin.part(&id, 0, 0, first, &sha(first)).await,
        StatusCode::NO_CONTENT
    );
    // Resuming: the same manifest gives the same checkpoint, with what it has.
    let (status, again) = admin.json("POST", &path, Some(manifest.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["id"], created["id"]);
    assert_eq!(again["files"][0]["uploaded"], json!([0]));

    let (status, body) = admin
        .json("POST", &format!("/api/v0/checkpoints/{id}/commit"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "parts are missing: {body}");
    assert_eq!(body["code"], "incomplete");

    let rest = &model[PART_SIZE as usize..];
    assert_eq!(
        admin.part(&id, 0, 1, rest, &sha(b"something else")).await,
        StatusCode::BAD_REQUEST,
        "a part that doesn't match its hash"
    );
    assert_eq!(
        admin.part(&id, 0, 1, &rest[..5], &sha(&rest[..5])).await,
        StatusCode::BAD_REQUEST,
        "an uncompressed part of the wrong size"
    );
    assert_eq!(
        admin.part(&id, 0, 2, rest, &sha(rest)).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        admin.part(&id, 9, 0, rest, &sha(rest)).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        admin.part(&id, 0, 1, rest, &sha(rest)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        admin.part(&id, 1, 0, &config_zst, &sha(&config_zst)).await,
        StatusCode::NO_CONTENT
    );

    let (status, done) = admin
        .json("POST", &format!("/api/v0/checkpoints/{id}/commit"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["state"], "committed");
    assert_eq!(done["size"], json!(model.len() + config.len()));
    assert_eq!(done["stored_size"], json!(model.len() + config_zst.len()));
    assert!(
        done["files"][0].get("uploaded").is_none(),
        "only while pending"
    );
    assert_eq!(
        admin.part(&id, 0, 1, rest, &sha(rest)).await,
        StatusCode::CONFLICT,
        "nothing more once committed"
    );
    let (status, body) = admin.json("POST", &path, Some(manifest)).await;
    assert_eq!(status, StatusCode::CONFLICT, "one checkpoint per step");
    assert_eq!(body["code"], "checkpoint_exists");

    let (status, list) = admin.json("GET", &path, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["step"], 1000);
    assert_eq!(list[0]["file_count"], 3);

    // Downloads: as uploaded, or as stored for the SDK.
    let file = |n: usize| format!("/api/v0/checkpoints/{id}/files/{n}");
    let (status, headers, body) = admin.download(&file(0)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == model, "the file as it was");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"model.pt\""
    );
    assert_eq!(headers["x-losscape-sha256"], sha(&model).as_str());
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let (_, _, body) = admin.download(&file(1)).await;
    assert_eq!(body, config, "decompressed");
    let (_, headers, body) = admin.download(&format!("{}?stored=true", file(1))).await;
    assert_eq!(body, config_zst, "as stored");
    assert_eq!(headers["x-losscape-compression"], "zstd");
    let (status, _, body) = admin.download(&file(2)).await;
    assert_eq!((status, body.len()), (StatusCode::OK, 0), "an empty file");

    assert_eq!(stored_files(&server), 3, "a stored object per part");
    let (status, _) = admin
        .json("DELETE", &format!("/api/v0/checkpoints/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = admin
        .json("GET", &format!("/api/v0/checkpoints/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        checkpoints::collect_garbage(&server.state).await.unwrap(),
        3
    );
    assert_eq!(stored_files(&server), 0, "its files are gone from storage");
    server.stop().await;
}

#[tokio::test]
async fn manifests_and_access_are_checked() {
    let server = server!();
    let run = new_run(&server).await;
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };
    let path = format!("/api/v0/runs/{run}/checkpoints");
    let one = |p: &str| json!({"step": 5, "files": [{"path": p, "size": 3, "sha256": sha(b"abc"), "compression": "none", "parts": 1}]});
    for bad in ["../../etc/passwd", "/abs", "a/./b"] {
        let (status, body) = admin.json("POST", &path, Some(one(bad))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    let (status, created) = admin.json("POST", &path, Some(one("weights.bin"))).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(
        admin.part(&id, 0, 0, b"abc", &sha(b"abc")).await,
        StatusCode::NO_CONTENT
    );
    let (status, _) = admin
        .json("POST", &format!("/api/v0/checkpoints/{id}/commit"), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    // A compressed file of two parts: one zstd frame each, downloaded as one file.
    let big: Vec<u8> = (0..PART_SIZE + 4096).map(|i| (i % 7) as u8).collect();
    let halves = [&big[..PART_SIZE as usize], &big[PART_SIZE as usize..]];
    let frames: Vec<Vec<u8>> = halves
        .iter()
        .map(|h| zstd::encode_all(*h, 1).unwrap())
        .collect();
    let manifest = json!({"step": 6, "files": [
        {"path": "big.bin", "size": big.len(), "sha256": sha(&big), "compression": "zstd", "parts": 2}
    ]});
    let (_, created) = admin.json("POST", &path, Some(manifest)).await;
    let big_id = created["id"].as_str().unwrap().to_owned();
    for (n, frame) in frames.iter().enumerate() {
        assert_eq!(
            admin.part(&big_id, 0, n, frame, &sha(frame)).await,
            StatusCode::NO_CONTENT
        );
    }
    admin
        .json(
            "POST",
            &format!("/api/v0/checkpoints/{big_id}/commit"),
            None,
        )
        .await;
    let (status, headers, body) = admin
        .download(&format!("/api/v0/checkpoints/{big_id}/files/0"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == big, "both frames, decompressed, in order");
    assert_eq!(
        headers[header::CONTENT_LENGTH],
        big.len().to_string().as_str()
    );

    // A viewer reads; an outsider can't tell it exists.
    let (viewer_id, viewer_key) = common::user(&server.state, "viewer").await;
    let (_, outsider_key) = common::user(&server.state, "outsider").await;
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role)
         SELECT id, $1, 'viewer' FROM projects WHERE name = 'test'",
    )
    .bind(viewer_id)
    .execute(server.state.pg())
    .await
    .unwrap();
    let viewer = Client {
        server: &server,
        key: viewer_key,
    };
    let outsider = Client {
        server: &server,
        key: outsider_key,
    };
    let (status, _, body) = viewer
        .download(&format!("/api/v0/checkpoints/{id}/files/0"))
        .await;
    assert_eq!(
        (status, body.as_slice()),
        (StatusCode::OK, b"abc".as_slice())
    );
    let (status, _) = viewer
        .json("DELETE", &format!("/api/v0/checkpoints/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "viewers don't delete");
    let (status, _) = viewer.json("POST", &path, Some(one("x.bin"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "viewers don't upload");
    for (method, url) in [
        ("GET", format!("/api/v0/checkpoints/{id}")),
        ("DELETE", format!("/api/v0/checkpoints/{id}")),
        ("GET", path.clone()),
    ] {
        let (status, _) = outsider.json(method, &url, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {url}");
    }
    let (status, _, _) = outsider
        .download(&format!("/api/v0/checkpoints/{id}/files/0"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    server.stop().await;
}

#[tokio::test]
async fn files_go_with_runs_and_abandoned_uploads() {
    let server = server!();
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };
    let upload = |run: String, step: u64, commit: bool| {
        let admin = &admin;
        async move {
            let manifest = json!({"step": step, "files": [
                {"path": "w.bin", "size": 4, "sha256": sha(b"data"), "compression": "none", "parts": 1}
            ]});
            let (_, created) = admin
                .json(
                    "POST",
                    &format!("/api/v0/runs/{run}/checkpoints"),
                    Some(manifest),
                )
                .await;
            let id = created["id"].as_str().unwrap().to_owned();
            assert_eq!(
                admin.part(&id, 0, 0, b"data", &sha(b"data")).await,
                StatusCode::NO_CONTENT
            );
            if commit {
                admin
                    .json("POST", &format!("/api/v0/checkpoints/{id}/commit"), None)
                    .await;
            }
            id
        }
    };
    let kept = new_run(&server).await;
    let deleted = new_run(&server).await;
    upload(kept.clone(), 1, true).await;
    upload(deleted.clone(), 1, true).await;
    upload(deleted.clone(), 2, true).await;
    let abandoned = upload(kept.clone(), 2, false).await;
    assert_eq!(stored_files(&server), 4);

    // A run deleted any way (purged, its project deleted): its checkpoints' files follow.
    sqlx::query("DELETE FROM runs WHERE id = $1::uuid")
        .bind(&deleted)
        .execute(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        checkpoints::collect_garbage(&server.state).await.unwrap(),
        2
    );
    assert_eq!(stored_files(&server), 2);

    // Uploads left unfinished go after a day.
    assert_eq!(
        checkpoints::expire_pending(&server.state, 24)
            .await
            .unwrap(),
        0,
        "not yet"
    );
    sqlx::query(
        "UPDATE checkpoints SET created_at = now() - interval '2 days' WHERE id = $1::uuid",
    )
    .bind(&abandoned)
    .execute(server.state.pg())
    .await
    .unwrap();
    assert_eq!(
        checkpoints::expire_pending(&server.state, 24)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        checkpoints::collect_garbage(&server.state).await.unwrap(),
        1
    );
    assert_eq!(
        stored_files(&server),
        1,
        "only the committed one of the run kept"
    );
    server.stop().await;
}
