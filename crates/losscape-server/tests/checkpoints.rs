//! Checkpoints end to end (docs/ARTIFACTS.md), against real Postgres and a storage folder.
//! Skipped when DATABASE_URL / CLICKHOUSE_URL are unset.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use losscape_core::PROTOCOL_HEADER;
use losscape_core::ids::RunId;
use losscape_core::proto::v1::RunInit;
use losscape_server::checkpoints::{self, PART_SIZE};
use prost::Message;
use std::collections::BTreeMap;

use serde_json::{Value, json};
use tower::ServiceExt;

mod common;

use common::{TestServer, error_of};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// BLAKE3, in hex: of parts as sent, of files as they are.
fn sha(bytes: &[u8]) -> String {
    hex(blake3::hash(bytes).as_bytes())
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
    let req = Request::post("/api/v1/runs/init")
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
            "/api/v1/checkpoints/{id}/files/{file}/parts/{part}"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", self.key))
        .header("x-losscape-blake3", hash)
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

/// Objects in the server's checkpoint storage.
async fn stored_files(server: &TestServer) -> u64 {
    server.state.storage().unwrap().count().await.unwrap()
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
        {"path": "model.pt", "size": model.len(), "blake3": sha(&model), "compression": "none", "parts": 2},
        {"path": "cfg/config.json", "size": config.len(), "blake3": sha(&config), "compression": "zstd", "parts": 1},
        {"path": "empty.txt", "size": 0, "blake3": sha(b""), "compression": "none", "parts": 0},
    ]});
    let path = format!("/api/v1/runs/{run}/checkpoints");
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
        .json("POST", &format!("/api/v1/checkpoints/{id}/commit"), None)
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
        .json("POST", &format!("/api/v1/checkpoints/{id}/commit"), None)
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
    let file = |n: usize| format!("/api/v1/checkpoints/{id}/files/{n}");
    let (status, headers, body) = admin.download(&file(0)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == model, "the file as it was");
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"model.pt\""
    );
    assert_eq!(headers["x-losscape-blake3"], sha(&model).as_str());
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let (_, _, body) = admin.download(&file(1)).await;
    assert_eq!(body, config, "decompressed");
    let (_, headers, body) = admin.download(&format!("{}?stored=true", file(1))).await;
    assert_eq!(body, config_zst, "as stored");
    assert_eq!(headers["x-losscape-compression"], "zstd");
    let (status, _, body) = admin.download(&file(2)).await;
    assert_eq!((status, body.len()), (StatusCode::OK, 0), "an empty file");

    // All of it as one tar archive, decompressed.
    let (status, headers, body) = admin
        .download(&format!("/api/v1/checkpoints/{id}/archive"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"ckpt-run-step-1000.tar\""
    );
    assert_eq!(
        headers[header::CONTENT_LENGTH],
        body.len().to_string().as_str()
    );
    let mut archive = tar::Archive::new(body.as_slice());
    let files: Vec<(String, Vec<u8>)> = archive
        .entries()
        .unwrap()
        .map(|e| {
            let mut e = e.unwrap();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut data).unwrap();
            (e.path().unwrap().to_string_lossy().into_owned(), data)
        })
        .collect();
    let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, ["model.pt", "cfg/config.json", "empty.txt"]);
    assert!(files[0].1 == model);
    assert_eq!(files[1].1, config);

    // The run list counts committed checkpoints.
    let (_, runs) = admin.json("GET", "/api/v1/projects/test/runs", None).await;
    let listed = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == run.as_str())
        .unwrap();
    assert_eq!(listed["checkpoints"], 1);
    assert_eq!(
        listed["checkpoint_bytes"],
        json!(model.len() + config.len())
    );

    assert_eq!(stored_files(&server).await, 3, "a stored object per part");
    let (status, _) = admin
        .json("DELETE", &format!("/api/v1/checkpoints/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = admin
        .json("GET", &format!("/api/v1/checkpoints/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        checkpoints::collect_garbage(&server.state).await.unwrap(),
        3
    );
    assert_eq!(
        stored_files(&server).await,
        0,
        "its files are gone from storage"
    );
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
    let path = format!("/api/v1/runs/{run}/checkpoints");
    let one = |p: &str| json!({"step": 5, "files": [{"path": p, "size": 3, "blake3": sha(b"abc"), "compression": "none", "parts": 1}]});
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
        .json("POST", &format!("/api/v1/checkpoints/{id}/commit"), None)
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
        {"path": "big.bin", "size": big.len(), "blake3": sha(&big), "compression": "zstd", "parts": 2}
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
            &format!("/api/v1/checkpoints/{big_id}/commit"),
            None,
        )
        .await;
    let (status, headers, body) = admin
        .download(&format!("/api/v1/checkpoints/{big_id}/files/0"))
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
        .download(&format!("/api/v1/checkpoints/{id}/files/0"))
        .await;
    assert_eq!(
        (status, body.as_slice()),
        (StatusCode::OK, b"abc".as_slice())
    );
    let (status, _) = viewer
        .json("DELETE", &format!("/api/v1/checkpoints/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "viewers don't delete");
    let (status, _) = viewer.json("POST", &path, Some(one("x.bin"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "viewers don't upload");
    for (method, url) in [
        ("GET", format!("/api/v1/checkpoints/{id}")),
        ("DELETE", format!("/api/v1/checkpoints/{id}")),
        ("GET", path.clone()),
    ] {
        let (status, _) = outsider.json(method, &url, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {url}");
    }
    let (status, _, _) = outsider
        .download(&format!("/api/v1/checkpoints/{id}/files/0"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Every way to read a checkpoint: viewers may; guests (runs and charts, not checkpoints)
    // are told no; outsiders can't tell it exists; nobody unsigned gets anything.
    let (guest_id, guest_key) = common::user(&server.state, "guest").await;
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role)
         SELECT id, $1, 'guest' FROM projects WHERE name = 'test'",
    )
    .bind(guest_id)
    .execute(server.state.pg())
    .await
    .unwrap();
    let guest = Client {
        server: &server,
        key: guest_key,
    };
    let reads = [
        path.clone(),
        format!("/api/v1/runs/{run}/checkpoints/brief"),
        format!("/api/v1/checkpoints/{id}"),
        format!("/api/v1/checkpoints/{id}/files/0"),
        format!("/api/v1/checkpoints/{id}/archive"),
        "/api/v1/checkpoints/resolve?ref=test/ckpt-run:5".to_owned(),
        format!("/ckpt/{id}"),
        format!("/ckpt/{id}/weights.bin"),
        "/ckpt/test/ckpt-run:5".to_owned(),
        "/ckpt/test/ckpt-run:5/weights.bin".to_owned(),
        "/ckpt/test/ckpt-run:latest".to_owned(),
    ];
    for url in &reads {
        let (status, _, _) = viewer.download(url).await;
        assert_eq!(status, StatusCode::OK, "a viewer reads {url}");
        let (status, _, body) = guest.download(url).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a guest doesn't read {url}");
        assert!(
            String::from_utf8_lossy(&body).contains("checkpoints"),
            "and is told why"
        );
        let (status, _, _) = outsider.download(url).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "an outsider can't tell {url} exists"
        );
        let anonymous = Request::get(url.as_str()).body(Body::empty()).unwrap();
        assert_eq!(
            server.call(anonymous).await.0,
            StatusCode::UNAUTHORIZED,
            "{url}"
        );
    }
    // A guest still reads the run itself.
    let (status, _, _) = guest.download(&format!("/api/v1/runs/{run}")).await;
    assert_eq!(status, StatusCode::OK);
    for (method, url) in [
        ("POST", format!("/api/v1/runs/{run}/checkpoints/prune")),
        ("POST", format!("/api/v1/runs/{run}/checkpoints/important")),
        ("PATCH", format!("/api/v1/checkpoints/{id}")),
    ] {
        let (status, _) = viewer
            .json(method, &url, Some(json!({"important": true})))
            .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "viewers don't {method} {url}"
        );
    }
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
                {"path": "w.bin", "size": 4, "blake3": sha(b"data"), "compression": "none", "parts": 1}
            ]});
            let (_, created) = admin
                .json(
                    "POST",
                    &format!("/api/v1/runs/{run}/checkpoints"),
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
                    .json("POST", &format!("/api/v1/checkpoints/{id}/commit"), None)
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
    assert_eq!(stored_files(&server).await, 4);

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
    assert_eq!(stored_files(&server).await, 2);

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
        stored_files(&server).await,
        1,
        "only the committed one of the run kept"
    );
    server.stop().await;
}

/// Creates, uploads and commits a checkpoint at `step`: `(path, content, compressed)`. Returns
/// its id.
async fn commit_checkpoint(
    admin: &Client<'_>,
    run: &str,
    step: u64,
    files: &[(&str, &[u8], bool)],
) -> String {
    let manifest = json!({"step": step, "files": files.iter().map(|(path, data, zstd)| json!({
        "path": path, "size": data.len(), "blake3": sha(data),
        "compression": if *zstd { "zstd" } else { "none" },
        "parts": (data.len() as u64).div_ceil(PART_SIZE),
    })).collect::<Vec<_>>()});
    let (status, created) = admin
        .json(
            "POST",
            &format!("/api/v1/runs/{run}/checkpoints"),
            Some(manifest),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    for (index, (_, data, zstd)) in files.iter().enumerate() {
        for (part, chunk) in data.chunks(PART_SIZE as usize).enumerate() {
            let body = if *zstd {
                zstd::encode_all(chunk, 1).unwrap()
            } else {
                chunk.to_vec()
            };
            let status = admin.part(&id, index, part, &body, &sha(&body)).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
    }
    let (status, _) = admin
        .json("POST", &format!("/api/v1/checkpoints/{id}/commit"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    id
}

#[tokio::test]
async fn short_links_serve_files_and_ranges() {
    let server = server!();
    let run = new_run(&server).await;
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };
    // Two compressed parts: a range across them decompresses from the second part's frame.
    let weights: Vec<u8> = (0..PART_SIZE + 5000).map(|i| (i / 7 % 251) as u8).collect();
    let id = commit_checkpoint(
        &admin,
        &run,
        10,
        &[
            ("model/weights.bin", &weights, true),
            ("notes.txt", b"hello", false),
        ],
    )
    .await;
    let (_, listed) = admin
        .json("GET", &format!("/api/v1/runs/{run}/checkpoints"), None)
        .await;
    // Its link names it; its permanent one is by id.
    assert_eq!(listed[0]["url"], "/ckpt/test/ckpt-run:10");
    assert_eq!(listed[0]["permanent_url"], format!("/ckpt/{id}"));

    let link = format!("/ckpt/{id}/model/weights.bin");
    let (status, headers, body) = admin.download(&link).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body == weights, "the whole file");
    assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert_eq!(etag, format!("\"{}\"", sha(&weights)));

    let ranged = |range: String, extra: Option<(header::HeaderName, String)>| {
        let mut req = Request::get(&link)
            .header(header::AUTHORIZATION, format!("Bearer {}", admin.key))
            .header(header::RANGE, range);
        if let Some((name, value)) = extra {
            req = req.header(name, value);
        }
        let router = server.router.clone();
        async move {
            let res = router
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
            (status, headers, body)
        }
    };
    let (start, end) = (PART_SIZE - 10, PART_SIZE + 9);
    let (status, headers, body) = ranged(format!("bytes={start}-{end}"), None).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        body,
        weights[start as usize..=end as usize],
        "across two parts"
    );
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes {start}-{end}/{}", weights.len()).as_str()
    );
    let (_, _, body) = ranged("bytes=-100".into(), None).await;
    assert_eq!(body, weights[weights.len() - 100..], "the end");
    let (status, _, _) = ranged(format!("bytes={}-", weights.len()), None).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    let (status, _, body) = ranged(
        "bytes=0-9".into(),
        Some((header::IF_RANGE, "\"other\"".into())),
    )
    .await;
    assert_eq!(
        (status, body.len()),
        (StatusCode::OK, weights.len()),
        "changed: all of it"
    );
    let (status, _, body) = ranged("bytes=0-9".into(), Some((header::IF_NONE_MATCH, etag))).await;
    assert_eq!((status, body.len()), (StatusCode::NOT_MODIFIED, 0));

    let (status, _, body) = admin.download(&format!("/ckpt/{id}/notes.txt")).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &b"hello"[..]));
    let (status, _, _) = admin.download(&format!("/ckpt/{id}/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, headers, body) = admin.download(&format!("/ckpt/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/x-tar");
    assert_eq!(
        tar::Archive::new(body.as_slice())
            .entries()
            .unwrap()
            .count(),
        2
    );

    let anonymous = Request::get(&link).body(Body::empty()).unwrap();
    assert_eq!(server.call(anonymous).await.0, StatusCode::UNAUTHORIZED);
    // Not compressed on the way, which would lose its length (and be slow).
    let req = Request::get(&link)
        .header(header::AUTHORIZATION, server.bearer())
        .header(header::ACCEPT_ENCODING, "gzip")
        .body(Body::empty())
        .unwrap();
    let res = server.router.clone().oneshot(req).await.unwrap();
    assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
    assert_eq!(
        res.headers()[header::CONTENT_LENGTH],
        weights.len().to_string().as_str()
    );

    // By name, step, :latest, an alias; ambiguous once another run has the name.
    let later = commit_checkpoint(&admin, &run, 20, &[("notes.txt", b"later", false)]).await;
    for (link, want) in [
        ("/ckpt/test/ckpt-run:10/notes.txt", &b"hello"[..]),
        ("/ckpt/test/ckpt-run:latest/notes.txt", b"later"),
        (
            &format!("/ckpt/test/{}:10/notes.txt", run.replace('-', "")),
            b"hello",
        ),
    ] {
        let (status, headers, body) = admin.download(link).await;
        assert_eq!((status, body.as_slice()), (StatusCode::OK, want), "{link}");
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "private, no-cache",
            "{link} can change"
        );
    }
    let (_, headers, _) = admin.download("/ckpt/test/ckpt-run:latest").await;
    assert_eq!(headers["x-losscape-checkpoint"], later.as_str());
    let (status, _, body) = admin.download("/ckpt/test/ckpt-run:best").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no best metric: {}",
        String::from_utf8_lossy(&body)
    );
    let (status, _, _) = admin.download("/ckpt/test/ckpt-run:30").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = admin.download("/ckpt/nope/ckpt-run:10").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, resolved) = admin
        .json(
            "GET",
            "/api/v1/checkpoints/resolve?ref=http://x/ckpt/test/ckpt-run:latest",
            None,
        )
        .await;
    assert_eq!(
        (status, resolved["id"].as_str()),
        (StatusCode::OK, Some(later.as_str()))
    );
    assert_eq!(resolved["files"][0]["path"], "notes.txt");
    new_run(&server).await; // another run named ckpt-run
    let (status, _, body) = admin.download("/ckpt/test/ckpt-run:10").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (_, listed) = admin
        .json("GET", &format!("/api/v1/runs/{run}/checkpoints"), None)
        .await;
    assert_eq!(
        listed[0]["url"],
        format!("/ckpt/test/{}:10", run.replace('-', "")),
        "by id, now that the name is shared"
    );

    // Stored where they can be found: project, run, step, then the files by path.
    let storage = server.state.storage().unwrap();
    let mut keys = storage.keys().await.unwrap();
    keys.sort();
    let tail = |id: &str| id.replace('-', "")[24..].to_owned();
    let place = format!("test/ckpt-run-{}/step-0000000010-{}", tail(&run), tail(&id));
    assert!(
        keys.contains(&format!("{place}/notes.txt.00000")),
        "{keys:?}"
    );
    assert!(
        keys.contains(&format!("{place}/model/weights.bin.00001.zst")),
        "{keys:?}"
    );

    // Hashed by the server as it arrived: each file's BLAKE3 (as b3sum gives it), and the
    // checkpoint's digest of them, which also names its archive.
    let (_, detail) = admin
        .json("GET", &format!("/api/v1/checkpoints/{id}"), None)
        .await;
    let files = detail["files"].as_array().unwrap();
    assert_eq!(files[0]["blake3"], sha(&weights));
    assert_eq!(files[1]["blake3"], sha(b"hello"));
    let listing = format!(
        "{}  model/weights.bin\n{}  notes.txt\n",
        sha(&weights),
        sha(b"hello")
    );
    assert_eq!(detail["digest"], sha(listing.as_bytes()));
    assert!(detail["corrupt"].is_null());
    let (_, headers, _) = admin.download(&format!("/ckpt/{id}")).await;
    assert_eq!(
        headers["x-losscape-digest"],
        sha(listing.as_bytes()).as_str()
    );

    // A part changed in storage: never given out, and the checkpoint is marked.
    let corrupt = commit_checkpoint(&admin, &run, 30, &[("a.bin", b"original", false)]).await;
    let stored: Vec<String> = storage
        .keys()
        .await
        .unwrap()
        .into_iter()
        .filter(|k| k.contains("step-0000000030"))
        .collect();
    storage
        .put_at(&stored[0], b"tampered".to_vec().into())
        .await
        .unwrap();
    let res = server
        .router
        .clone()
        .oneshot(
            Request::get(format!("/ckpt/{corrupt}/a.bin"))
                .header(header::AUTHORIZATION, server.bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = res.into_body().collect().await;
    assert!(
        body.is_err(),
        "the download breaks off rather than give what storage changed"
    );
    let mut marked = Value::Null;
    for _ in 0..50 {
        marked = admin
            .json("GET", &format!("/api/v1/checkpoints/{corrupt}"), None)
            .await
            .1;
        if !marked["corrupt"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        marked["corrupt"].as_str().unwrap().contains("a.bin.00000"),
        "{marked}"
    );
    server.stop().await;
}

/// Logs `metric` at each of `points` (step, value) for `run`.
async fn log_metric(server: &TestServer, run: &str, metric: &str, points: &[(u64, f64)]) {
    use losscape_core::proto::v1::{LogBatch, ScalarColumns};
    let id = uuid::Uuid::parse_str(run).unwrap();
    let batch = LogBatch {
        run_id: id.as_bytes().to_vec(),
        seq: 0,
        scalars: Some(ScalarColumns {
            keys: vec![metric.into()],
            key_idx: vec![0; points.len()],
            step: points.iter().map(|p| p.0).collect(),
            time_ms: points
                .iter()
                .map(|p| 1_700_000_000_000 + p.0 as i64)
                .collect(),
            value: points.iter().map(|p| p.1).collect(),
        }),
        ..Default::default()
    };
    let (status, body) = server.send("/api/v1/ingest", batch, false).await;
    assert_eq!(status, StatusCode::OK, "{}", error_of(&body));
}

#[tokio::test]
async fn retention_keeps_the_important_last_and_best() {
    let server = server!();
    let run = new_run(&server).await;
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };
    // val/acc peaks at step 200; NaN (never best) at 400.
    log_metric(
        &server,
        &run,
        "val/acc",
        &[
            (50, 0.5),
            (100, 0.7),
            (150, 0.8),
            (200, 0.9),
            (300, 0.85),
            (400, f64::NAN),
        ],
    )
    .await;
    let mut ids = BTreeMap::new();
    for step in [100, 200, 300, 400, 500] {
        ids.insert(
            step,
            commit_checkpoint(&admin, &run, step, &[("m.bin", b"w", false)]).await,
        );
    }
    let listing = format!("/api/v1/runs/{run}/checkpoints?metric=val/acc&metric=missing");
    // Metrics reach ClickHouse shortly after they're logged.
    let mut listed = Value::Null;
    for _ in 0..100 {
        listed = admin.json("GET", &listing, None).await.1;
        if listed[0]["metrics"]["val/acc"].is_object() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let at = |i: usize| listed[i]["metrics"]["val/acc"].clone();
    assert_eq!(at(0), json!({"step": 100, "value": 0.7}));
    assert_eq!(at(2), json!({"step": 300, "value": 0.85}));
    assert_eq!(
        at(3),
        json!({"step": 300, "value": 0.85}),
        "NaN skipped: the latest before"
    );
    assert!(listed[0]["metrics"].get("missing").is_none());
    assert_eq!(
        listed[0]["keep"],
        json!([]),
        "kept until deleted by default"
    );
    assert!(listed[0]["expires_at_ms"].is_null());

    let (status, marked) = admin
        .json(
            "PATCH",
            &format!("/api/v1/checkpoints/{}", ids[&100]),
            Some(json!({"important": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{marked}");
    assert_eq!(
        (marked["important"].clone(), marked["keep"].clone()),
        (json!(true), json!(["important"]))
    );

    let retention = json!({"days": 30, "keep_last": true, "keep_best": true, "best": {"metric": "val/acc", "goal": "max"}});
    let (status, _) = admin
        .json(
            "PATCH",
            "/api/v1/projects/test",
            Some(json!({"checkpoint_retention": retention})),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "project settings take a session, not a key"
    );
    let project_id: i64 = sqlx::query_scalar("SELECT id FROM projects WHERE name = 'test'")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    let policy = serde_json::from_value(retention.clone()).unwrap();
    losscape_server::retention::save(&server.state, project_id, &policy)
        .await
        .unwrap();
    let (_, info) = admin.json("GET", "/api/v1/projects/test", None).await;
    assert_eq!(info["checkpoint_retention"], retention);
    let (_, listed) = admin.json("GET", &listing, None).await;
    let keep: Vec<Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["keep"].clone())
        .collect();
    assert_eq!(
        keep,
        [
            json!(["important"]),
            json!(["best"]),
            json!([]),
            json!([]),
            json!(["last"])
        ]
    );
    assert!(
        listed[2]["expires_at_ms"].as_i64().unwrap()
            > listed[2]["committed_at_ms"].as_i64().unwrap()
    );
    assert!(listed[1]["expires_at_ms"].is_null());

    // A month later: only the unprotected go.
    sqlx::query("UPDATE checkpoints SET committed_at = now() - interval '31 days'")
        .execute(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        losscape_server::retention::apply(&server.state)
            .await
            .unwrap(),
        2
    );
    let (_, listed) = admin.json("GET", &listing, None).await;
    let steps: Vec<i64> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["step"].as_i64().unwrap())
        .collect();
    assert_eq!(steps, [100, 200, 500]);
    assert_eq!(
        losscape_server::retention::apply(&server.state)
            .await
            .unwrap(),
        0,
        "the rest stays"
    );

    server.stop().await;
}

#[tokio::test]
async fn ten_thousand_checkpoints_list_and_expire_quickly() {
    let server = server!();
    let run = new_run(&server).await;
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };
    let run_id = uuid::Uuid::parse_str(&run).unwrap();
    insert_checkpoints(&server, run_id, 10_000).await;
    // val/acc peaks at step 10,000.
    let points: Vec<(u64, f64)> = (0..20_000u64)
        .map(|s| (s, 1.0 - ((s as f64 - 10_000.0) / 10_000.0).powi(2)))
        .collect();
    log_metric(&server, &run, "val/acc", &points).await;
    let listing = format!("/api/v1/runs/{run}/checkpoints?metric=val/acc");
    let mut listed = Value::Null;
    for _ in 0..200 {
        listed = admin.json("GET", &listing, None).await.1;
        if listed[9999]["metrics"]["val/acc"]["step"] == 20_000 - 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(listed.as_array().map(Vec::len), Some(10_000), "{listed}");
    assert_eq!(listed[0]["file_count"], 3);
    assert_eq!(listed[0]["stored_size"], 900);

    let project_id: i64 = sqlx::query_scalar("SELECT id FROM projects WHERE name = 'test'")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    let policy = serde_json::from_value(json!({"days": 30, "keep_last": true, "keep_best": true,
        "best": {"metric": "val/acc", "goal": "max"}}))
    .unwrap();
    losscape_server::retention::save(&server.state, project_id, &policy)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let (status, listed) = admin.json("GET", &listing, None).await;
    let took = started.elapsed();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed[4999]["keep"], json!(["best"]), "step 10,000");
    assert_eq!(listed[9999]["keep"], json!(["last"]));
    eprintln!("listing 10,000 checkpoints with a metric and retention: {took:?}");
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");

    sqlx::query("UPDATE checkpoints SET committed_at = now() - interval '31 days'")
        .execute(server.state.pg())
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let deleted = losscape_server::retention::apply(&server.state)
        .await
        .unwrap();
    eprintln!("retention over 10,000 checkpoints: {:?}", started.elapsed());
    assert_eq!(deleted, 9_998, "all but the best and the last");
    server.stop().await;
}

/// `n` committed checkpoints of 3 files at steps 2, 4, … `2n`, written directly: uploading
/// thousands through the API is the SDK's test.
async fn insert_checkpoints(server: &TestServer, run_id: uuid::Uuid, n: i32) {
    for sql in [
        "INSERT INTO checkpoints (id, run_id, step, state, storage_prefix, size, committed_at)
         SELECT gen_random_uuid(), $1, g * 2, 'committed', 'none/' || $1 || '/' || g, 900, now()
         FROM generate_series(1, $2) g",
        "INSERT INTO checkpoint_files (checkpoint_id, file_index, path, size, blake3, compression, parts)
         SELECT c.id, i, 'f' || i, 300, '\\x00', 'none', 1
         FROM checkpoints c, generate_series(0, 2) i WHERE c.run_id = $1",
        "INSERT INTO checkpoint_parts (checkpoint_id, file_index, part, size, blake3, content)
         SELECT f.checkpoint_id, f.file_index, 0, 300, '\\x00', '\\x00'
         FROM checkpoint_files f JOIN checkpoints c ON c.id = f.checkpoint_id WHERE c.run_id = $1",
    ] {
        sqlx::query(sql)
            .bind(run_id)
            .bind(n)
            .execute(server.state.pg())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn pruning_keeps_spaced_last_best_and_important() {
    let server = server!();
    let run = new_run(&server).await;
    let admin = Client {
        server: &server,
        key: server.api_key.clone(),
    };
    let run_id = uuid::Uuid::parse_str(&run).unwrap();
    insert_checkpoints(&server, run_id, 1000).await; // steps 2..=2000
    // Best at step 777 (nearest checkpoint: 778), and one marked important.
    log_metric(
        &server,
        &run,
        "val/loss",
        &[(0, 5.0), (777, 0.1), (1500, 0.3)],
    )
    .await;
    let important: String = sqlx::query_scalar(
        "UPDATE checkpoints SET important = true WHERE run_id = $1 AND step = 1002
         RETURNING id::text",
    )
    .bind(run_id)
    .fetch_one(server.state.pg())
    .await
    .unwrap();
    let prune_url = format!("/api/v1/runs/{run}/checkpoints/prune");
    let prune = |body: Value| admin.json("POST", &prune_url, Some(body));
    let spec = json!({"spaced": 100, "last": true, "best": {"metric": "val/loss", "goal": "min"}});

    // Metrics reach ClickHouse shortly after they're logged: until then, no best.
    let mut preview;
    loop {
        let mut dry = spec.clone();
        dry["dry_run"] = json!(true);
        preview = prune(dry).await.1;
        if preview["kept"] == 102 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // 100 spaced (the last among them), the best (778) and the important one (1002).
    assert_eq!(preview["kept"], 102, "{preview}");
    assert_eq!(preview["deleted"], 898);
    assert_eq!(preview["deleted_bytes"], 898 * 900);
    assert_eq!(
        stored_steps(&server, run_id).await.len(),
        1000,
        "a dry run deletes nothing"
    );

    let (status, done) = prune(spec).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        (done["deleted"].clone(), done["kept"].clone()),
        (json!(898), json!(102))
    );
    let steps = stored_steps(&server, run_id).await;
    assert!(
        steps.contains(&2)
            && steps.contains(&2000)
            && steps.contains(&778)
            && steps.contains(&1002)
    );
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_garbage")
        .fetch_one(server.state.pg())
        .await
        .unwrap();
    assert_eq!(
        queued, 898,
        "their files queued for deletion, once per statement"
    );

    // Everything but the important ones.
    let (_, done) = prune(json!({})).await;
    assert_eq!(
        (done["deleted"].clone(), done["kept"].clone()),
        (json!(101), json!(1))
    );
    let (_, listed) = admin
        .json("GET", &format!("/api/v1/runs/{run}/checkpoints"), None)
        .await;
    assert_eq!(listed[0]["id"], important.as_str());

    // Marking all, then none (one left: the important one).
    let mark_url = format!("/api/v1/runs/{run}/checkpoints/important");
    let mark =
        |important: bool| admin.json("POST", &mark_url, Some(json!({"important": important})));
    assert_eq!(mark(false).await.1["changed"], 1);
    assert_eq!(mark(true).await.1["changed"], 1);
    assert_eq!(mark(true).await.1["changed"], 0, "already");
    server.stop().await;
}

async fn stored_steps(server: &TestServer, run_id: uuid::Uuid) -> Vec<i64> {
    sqlx::query_scalar("SELECT step FROM checkpoints WHERE run_id = $1 ORDER BY step")
        .bind(run_id)
        .fetch_all(server.state.pg())
        .await
        .unwrap()
}
