-- Checkpoints: folders a run uploads at a step (docs/ARTIFACTS.md). Their files are in the
-- configured storage (LOSSCAPE_STORAGE), under `storage_prefix`, one object per part.
CREATE TABLE checkpoints (
    id              UUID PRIMARY KEY,
    run_id          UUID NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    step            BIGINT NOT NULL CHECK (step >= 0),
    state           TEXT NOT NULL CHECK (state IN ('pending', 'committed')),
    storage_prefix  TEXT NOT NULL,
    size            BIGINT NOT NULL,
    created_by      BIGINT REFERENCES users (id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    committed_at    TIMESTAMPTZ,
    UNIQUE (run_id, step)
);
-- Pending ones that are never committed expire.
CREATE INDEX checkpoints_pending ON checkpoints (created_at) WHERE state = 'pending';

CREATE TABLE checkpoint_files (
    checkpoint_id  UUID NOT NULL REFERENCES checkpoints (id) ON DELETE CASCADE,
    file_index     INT NOT NULL,
    path           TEXT NOT NULL,
    size           BIGINT NOT NULL,
    -- Of the file as it was, before any compression.
    sha256         BYTEA NOT NULL,
    compression    TEXT NOT NULL CHECK (compression IN ('none', 'zstd')),
    parts          INT NOT NULL,
    PRIMARY KEY (checkpoint_id, file_index),
    UNIQUE (checkpoint_id, path)
);

-- Parts received: of each file's parts, as stored (compressed or not).
CREATE TABLE checkpoint_parts (
    checkpoint_id  UUID NOT NULL,
    file_index     INT NOT NULL,
    part           INT NOT NULL,
    size           BIGINT NOT NULL,
    sha256         BYTEA NOT NULL,
    PRIMARY KEY (checkpoint_id, file_index, part),
    FOREIGN KEY (checkpoint_id, file_index)
        REFERENCES checkpoint_files (checkpoint_id, file_index) ON DELETE CASCADE
);

-- Where deleted checkpoints' files are in storage, for the cleaner to delete them. Checkpoints
-- go with their runs and projects too (cascades): the trigger catches every way.
CREATE TABLE storage_garbage (
    prefix     TEXT PRIMARY KEY,
    queued_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE FUNCTION queue_checkpoint_garbage() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO storage_garbage (prefix) VALUES (OLD.storage_prefix) ON CONFLICT DO NOTHING;
    RETURN OLD;
END
$$;

CREATE TRIGGER checkpoint_garbage AFTER DELETE ON checkpoints
    FOR EACH ROW EXECUTE FUNCTION queue_checkpoint_garbage();
