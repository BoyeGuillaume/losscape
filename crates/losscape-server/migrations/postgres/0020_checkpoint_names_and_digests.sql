-- A project's best checkpoint metric is its own setting: it names each run's best checkpoint
-- (the `:best` link, the mark in the UI) whether or not retention keeps it.
ALTER TABLE projects ADD COLUMN checkpoint_keep_best BOOLEAN NOT NULL DEFAULT false;
UPDATE projects SET checkpoint_keep_best = true WHERE checkpoint_best_metric IS NOT NULL;

-- Where a file's parts are under its checkpoint's place: `<name>.<part>`, `.zst` added when
-- compressed. Null for files stored before, at `<index>/<part>`.
ALTER TABLE checkpoint_files ADD COLUMN storage_name TEXT;

-- The checkpoint as one tar archive (its download): its SHA-256, computed by the server after
-- the upload by reading everything back, which checks each file against its SHA-256 too.
ALTER TABLE checkpoints
    ADD COLUMN archive_sha256 BYTEA,
    ADD COLUMN verified_at TIMESTAMPTZ,
    ADD COLUMN verify_error TEXT;
CREATE INDEX checkpoints_unverified ON checkpoints (committed_at)
    WHERE state = 'committed' AND verified_at IS NULL;
