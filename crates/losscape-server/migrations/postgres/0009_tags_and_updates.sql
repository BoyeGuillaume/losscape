-- When a run last changed (its row, its aliases or its metric renames), so the run list
-- can be refreshed with only the runs that changed.
ALTER TABLE runs ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE INDEX runs_project_updated ON runs (project_id, updated_at);

CREATE FUNCTION touch_run() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := now();
    RETURN NEW;
END $$;
CREATE TRIGGER runs_touch BEFORE UPDATE ON runs FOR EACH ROW EXECUTE FUNCTION touch_run();

-- Rows that belong to a run (aliases, renames) mark it changed. Not when the run itself is
-- being deleted: then the delete cascades here, one trigger level down.
CREATE FUNCTION touch_owning_run() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF pg_trigger_depth() > 1 THEN
        RETURN NULL;
    END IF;
    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        UPDATE runs SET updated_at = now() WHERE id = OLD.run_id;
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') THEN
        UPDATE runs SET updated_at = now() WHERE id = NEW.run_id;
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER run_aliases_touch AFTER INSERT OR UPDATE OR DELETE ON run_aliases
    FOR EACH ROW EXECUTE FUNCTION touch_owning_run();
CREATE TRIGGER run_metric_renames_touch AFTER INSERT OR UPDATE OR DELETE ON run_metric_renames
    FOR EACH ROW EXECUTE FUNCTION touch_owning_run();

-- How a project shows its tags (runs hold the tags themselves): a color from the UI's
-- palette (none: one derived from the name) and a description.
CREATE TABLE project_tags (
    project_id   BIGINT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    color        TEXT,
    description  TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, name)
);
