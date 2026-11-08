-- Saved as a whole, so each save says which version it's based on: a stale tab or a second
-- editor gets refused instead of silently overwriting what happened since.
ALTER TABLE projects
    ADD COLUMN metric_definitions_version BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN board_version BIGINT NOT NULL DEFAULT 0;
