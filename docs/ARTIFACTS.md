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
- Checkpoints are deleted by hand, on the run's Checkpoints tab or with `.delete()`. Rules to keep
  only some (the last few, the best by a metric) come later.

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
- **Checked:** each part against its SHA-256 as it arrives; each file has the SHA-256 of its
  original content, given back with it.

Storage goes through the [`object_store`](https://crates.io/crates/object_store) crate
(MIT/Apache-2.0), at `LOSSCAPE_STORAGE`: a folder (the `checkpoints` volume of `compose.yaml`, or a
temporary folder in tests), or an S3-compatible store (`s3://bucket/prefix`, credentials in the
`AWS_*` variables). Keys are `<project id>/<run id>/<checkpoint id>/<file index>/<part>`: ids and
numbers only, so no path anyone sends reaches outside its checkpoint. Everything goes through the
server, which checks who asks: clients never reach the storage, or its credentials.

## Protocol

1. The SDK lists the folder, decides on compression, and hashes each file.
2. `POST /api/v0/runs/{run}/checkpoints` `{"step", "files": [{"path", "size", "sha256",
   "compression", "parts"}]}` (editors): a pending checkpoint, with the parts each file has. The
   same manifest again gives back the same checkpoint, to resume; another one at the step of a
   pending checkpoint replaces it; at the step of a committed one, it's refused (delete first).
3. `PUT /api/v0/checkpoints/{id}/files/{file}/parts/{part}`, the part as declared (compressed or
   not), its SHA-256 in `x-losscape-sha256`: several at a time, again if they fail.
4. `POST /api/v0/checkpoints/{id}/commit`, once all parts are there. Pending checkpoints never
   committed expire after a day, with what was uploaded.

Reading: `GET /api/v0/runs/{run}/checkpoints` (committed ones, by step; `?pending=true` for the
others), `GET /api/v0/checkpoints/{id}` (with its files), and
`GET /api/v0/checkpoints/{id}/files/{file}`: decompressed, or as stored with `?stored=true`,
always as a download; `GET /api/v0/checkpoints/{id}/archive`: all of its files, decompressed, as
one tar archive (pax headers: long paths and files over 8 GiB), streamed with its length known
up front. `DELETE /api/v0/checkpoints/{id}` (editors).

## Data (Postgres)

`checkpoints` (run, step, state, where in storage, size), `checkpoint_files` (path, size, SHA-256,
compression, parts) and `checkpoint_parts` (those received, with their stored size and SHA-256):
crates/losscape-server/migrations/postgres/0016_checkpoints.sql. A checkpoint deleted any way (by
itself, with its run or its project) leaves its place in storage in `storage_garbage`, through a
trigger; the server deletes those files within a minute.

## Rules

- Editors of a project upload and delete; viewers list and download. Access to a file is checked
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

Later: markers at checkpoint steps on charts with that step's metrics, rules for which to keep,
shards uploaded from every GPU, general artifacts with versions and lineage, logged images and
media.
