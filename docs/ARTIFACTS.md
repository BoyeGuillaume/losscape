# Checkpoints (design)

What training runs save every so many steps, uploaded to the server with the run, so a run can be
resumed from anywhere and its weights downloaded later. The server, the SDK and the web UI are
built. General artifacts (datasets, models with
versions and lineage) and logged media come later, on the same storage.

## Using them

```python
torch.save(model.state_dict(), "ckpt/model.pt")      # save however you like
torch.save(optimizer.state_dict(), "ckpt/optim.pt")
run.log_checkpoint("ckpt/", step=step)                # copies, then uploads in the background

# Resuming, after a preempted job:
run = lsc.init(id=run_id, resume=True)
ckpt = run.latest_checkpoint()                        # a local folder: kept, or downloaded
model.load_state_dict(torch.load(ckpt / "model.pt"))

# Later, from anywhere:
lsc.Api().run(run_id).checkpoints[-1].download(files="model.pt")
```

- `log_checkpoint` copies the folder into the run's local folder before returning: scripts often
  save to the same path every time, and the next save would change files mid-upload. On a disk that
  copies instantly (btrfs, XFS) it costs nothing; `move=True` hands the folder over instead.
- The upload runs in the background, on its own queue: metrics never wait behind a checkpoint.
  `run.finish()` waits for it as long as it progresses; an interrupted upload resumes, also
  with `losscape sync`. The local copy is deleted once the server has it all.
- A run's Checkpoints tab lists them (step, time, files, size), downloads one file or all of them
  as a tar archive, and deletes them; the runs table has a Checkpoints column (count and size).
- Each checkpoint has a link through the server that names it:
  `/ckpt/<project>/<run>:<step>` (all of it, a tar archive) and `…:<step>/<path>` (one file,
  with `Range`, `ETag` and `If-None-Match`), with `:latest` and `:best` for a run's last and
  best one. The run is its name (if no other run of the project has it; otherwise the link
  says so), `@<alias>`, or its id. `/ckpt/<checkpoint id>` names one checkpoint for good (and
  is cached for good). The same whatever the storage, checked like the API (session or API
  key), never compressed on the way. `GET /api/v0/checkpoints/resolve?ref=…` gives what a link
  names, for the SDK.
- Hashes are BLAKE3, taken by the server as the upload goes through it, never read back from
  storage, which isn't trusted to give back what it was given: every read checks each part
  against the hash taken when it arrived, and a download breaks off rather than give out
  something changed (the checkpoint is marked, and says what). Each file's BLAKE3 (as `b3sum`
  gives it) and the checkpoint's digest (BLAKE3 of its files listed as `b3sum` lists them,
  `<hash>  <path>`, by path) are shown and kept.
- A run can be thinned out at once (its Checkpoints tab, Clean up): about N evenly spaced by
  step, with its last, its best and its important ones; or all but the important ones; with a
  count of what goes before anything does. All of a run's checkpoints can be marked important,
  or none.
- Checkpoints are deleted by hand, on the run's Checkpoints tab or with `.delete()`, or by the
  project's retention: none by default; or so many days after upload, except those marked
  important and, as chosen, each run's last one and its best one by a metric (its latest value
  at the checkpoint's step, highest or lowest). The cleaner applies it every minute.
- The run's Checkpoints tab shows each one's favourite (important) metrics at its step, why
  it's kept or when it goes; its charts mark checkpoint steps (up to 100), hovered to say which,
  clicked to open it.
- Made for thousands per run (tested with 10,000): listed in one query (0.2 s with a metric's
  values and retention), drawn as a virtual list, followed while uploading by asking only for
  those in progress; the SDK uploads 8 at once (10,000 small ones in about 20 s).

## How they're stored

A checkpoint is a folder at a step of a run, stored file by file, each file in parts of 16 MiB
(before compression):

- **One object per part**, so that a single file downloads without the rest (a checkpoint is
  mostly optimizer state: Adam keeps two buffers the size of the weights, and evaluation only
  needs the weights), and so that an interrupted upload resumes at the part it stopped at. Bundling
  files under about 1 MB into one pack, for folders of many small files, comes later.
- **Compressed with zstd**, at its fastest level, part by part (each part a zstd frame: together,
  one zstd stream), when a sample shows it saves at least 5%. Measured on synthetic tensors: about
  7% on fp32 weights and optimizer state, 22% on bf16, half on mostly-zero tensors, 95% on JSON, at
  well over 1 GB/s, so it never slows an upload. (Grouping each number's bytes before compressing
  saves about 10 points more, in a format plain zstd can't read: later, if storage costs call for
  it.)
- **Hashed by the server, once, as the upload goes through it:** each part's BLAKE3 as sent
  (checked against the client's, then kept: storage is checked against it on every read), and
  its content's share of its file's BLAKE3. Parts are 16 MiB, a power of two, so each is a
  subtree of the file's BLAKE3 tree: whatever order they arrive in, the server merges their
  hashes into the file's own BLAKE3 at commit, without reading anything back, and refuses the
  commit if it isn't the one declared (crates/losscape-server/src/hashing.rs). A compressed part
  is decompressed on arrival for this, which also checks it's exactly its share of the file.

In storage, each checkpoint has a place that reads as what it is (in an S3 console, say):
`<project>/<run name>-<run id end>/step-<step>-<checkpoint id end>/<file path>.<part>`, `.zst`
added to compressed parts (`cat model.pt.*` rebuilds an uncompressed file; `zstd -dc
config.json.*.zst` a compressed one). The ids' random ends keep apart two runs of one name, and
a checkpoint from one deleted at its step whose files may still be going.

Storage goes through the [`object_store`](https://crates.io/crates/object_store) crate
(MIT/Apache-2.0), at `LOSSCAPE_STORAGE`: a folder (the `checkpoints` volume of `compose.yaml`, or a
temporary folder in tests), or an S3-compatible store (`s3://bucket/prefix`, credentials in the
`AWS_*` variables). Names in keys are reduced to letters, digits, `.`, `_` and `-`, and paths
are checked on upload (relative, no `..`), so nothing anyone sends reaches outside its
checkpoint. Everything goes through the
server, which checks who asks: clients never reach the storage, or its credentials.

## Protocol

1. The SDK lists the folder, decides on compression, and hashes each file.
2. `POST /api/v0/runs/{run}/checkpoints` `{"step", "files": [{"path", "size", "blake3",
   "compression", "parts"}]}` (editors): a pending checkpoint, with the parts each file has. The
   same manifest again gives back the same checkpoint, to resume; another one at the step of a
   pending checkpoint replaces it; at the step of a committed one, it's refused (delete first).
3. `PUT /api/v0/checkpoints/{id}/files/{file}/parts/{part}`, the part as declared (compressed or
   not), its BLAKE3 in `x-losscape-blake3`: several at a time, again if they fail.
4. `POST /api/v0/checkpoints/{id}/commit`, once all parts are there: each file's BLAKE3, from
   its parts', must be the one declared (else `hash_mismatch`). Pending checkpoints never
   committed expire after a day, with what was uploaded.

Reading: `GET /api/v0/runs/{run}/checkpoints` (committed ones, by step; `?pending=true` for the
others; `&metric=<name>` repeated for metrics' latest values at their steps), `GET /api/v0/checkpoints/{id}` (with its files), and
`GET /api/v0/checkpoints/{id}/files/{file}`: decompressed, or as stored with `?stored=true`,
always as a download; `GET /api/v0/checkpoints/{id}/archive`: all of its files, decompressed, as
one tar archive (pax headers: long paths and files over 8 GiB), streamed with its length known
up front. `DELETE /api/v0/checkpoints/{id}` (editors).

## Data (Postgres)

`checkpoints` (run, step, state, where in storage, size, digest), `checkpoint_files` (path, size,
BLAKE3, compression, parts) and `checkpoint_parts` (those received, with their stored size, the
BLAKE3 of what's stored, and their share of their file's BLAKE3):
crates/losscape-server/migrations/postgres/0016_checkpoints.sql. A checkpoint deleted any way (by
itself, with its run or its project) leaves its place in storage in `storage_garbage`, through a
trigger; the server deletes those files within a minute.

## Rules

- Editors of a project upload and delete; viewers list and download; guests (who read runs and
  charts) don't see checkpoints, and are told so. Access to a file is checked
  through its checkpoint, run and project.
- Paths are relative, normalized, without `..`; names with control characters are refused.
- Files are always served as downloads (`Content-Disposition: attachment`, `nosniff`): an uploaded
  HTML file never runs in the app.
- A configurable largest checkpoint (`LOSSCAPE_MAX_CHECKPOINT_GB`), and later per-project quotas
  with the storage dashboard.

## Work

1. **Server (done):** schema, storage on `object_store` (a folder or S3), the endpoints above,
   expiry of pending checkpoints, deletion with runs and projects. Tested end to end in
   crates/losscape-server/tests/checkpoints.rs: full cycle, resuming, corrupt parts, permissions,
   paths, clean-up; on a folder, and on S3 with `make test-s3` (a local S3 mock, moto).
2. **SDK (done):** `log_checkpoint` (a copy or a move into the run's local log, offline too),
   the uploader in the Rust core (beside the metrics, 4 parts at a time, compression decided
   once per file, resuming), `finish()` and `sync`, `latest_checkpoint()` and `checkpoint(step)`
   (a local copy, linked, or a download), and the query API (`checkpoints`, `download`,
   `delete`). Tested end to end in crates/losscape-client/tests/e2e.rs (a run cut short
   mid-upload, offline, refused) and sdk/python/tests.
3. **Web UI (done):** a Checkpoints tab on the run page (step, time, files, size, uploads in
   progress; a file or all of them as a tar archive; delete), checkpoint count and size in the
   runs table (a column, shown by default when a run has some).
4. **Docs (done):** the SDK README, storage in docs/DEPLOYMENT.md (the volume or S3 settings,
   backing it up with the databases, sizing the disk).

`PATCH /api/v0/checkpoints/{id}` `{"important": bool}` (editors); a project's retention in
`PATCH /api/v0/projects/{project}` `{"checkpoint_retention": {"days", "keep_last", "best":
{"metric", "goal"}}}` (owners, signed in).

Later: keeping the last few, shards uploaded from every GPU, general artifacts with versions and lineage, logged images and
media.
