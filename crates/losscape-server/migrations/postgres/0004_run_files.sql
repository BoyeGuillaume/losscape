-- Text files captured with a run (the git diff of uncommitted changes), shown on the run page.
CREATE TABLE run_files (
    run_id      UUID NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    content     BYTEA NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, name)
);
