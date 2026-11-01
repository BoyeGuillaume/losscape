-- Archived runs are hidden from run lists and deleted for good after their project's
-- `archive_days` (set by its owners).
ALTER TABLE runs ADD COLUMN archived_at TIMESTAMPTZ;
CREATE INDEX runs_archived ON runs (archived_at) WHERE archived_at IS NOT NULL;
ALTER TABLE projects ADD COLUMN archive_days INT NOT NULL DEFAULT 30
    CHECK (archive_days BETWEEN 1 AND 3650);
