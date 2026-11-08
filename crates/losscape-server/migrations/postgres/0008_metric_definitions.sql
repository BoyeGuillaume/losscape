-- Custom metrics, per project: descriptions of logged metrics (unit, bounds, which way is
-- better, summary), and virtual metrics computed when reading (merges of differently named
-- metrics, families like val/pck-{k}, formulas). Nothing logged is ever rewritten.
CREATE TABLE metric_definitions (
    project_id  BIGINT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    position    INT NOT NULL,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('describe', 'merge', 'family', 'formula')),
    spec        JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (project_id, name)
);

-- A metric renamed within one run (logged as pck-8, shown as pck-8px).
CREATE TABLE run_metric_renames (
    run_id    UUID NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    from_key  TEXT NOT NULL,
    to_key    TEXT NOT NULL,
    PRIMARY KEY (run_id, from_key)
);

-- Lowest and highest value of each metric per run (system metrics aside), kept by ingest,
-- so the best value of any metric is known without scanning its points.
ALTER TABLE runs ADD COLUMN metric_min JSONB NOT NULL DEFAULT '{}',
                 ADD COLUMN metric_max JSONB NOT NULL DEFAULT '{}';

-- Every metric name logged in a project, kept by ingest (for pickers and definitions).
CREATE TABLE project_metric_keys (
    project_id  BIGINT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    PRIMARY KEY (project_id, key)
);
