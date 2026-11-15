-- Deleting thousands of checkpoints at once (pruning a run, retention): once per statement,
-- not once per row, queue their files for deletion and mark their runs changed.
DROP TRIGGER checkpoint_garbage ON checkpoints;
DROP TRIGGER checkpoints_touch ON checkpoints;

CREATE FUNCTION queue_checkpoints_garbage() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO storage_garbage (prefix) SELECT storage_prefix FROM gone ON CONFLICT DO NOTHING;
    -- Not when their run is being deleted: it isn't there to mark.
    IF pg_trigger_depth() = 1 THEN
        UPDATE runs SET updated_at = now() WHERE id IN (SELECT DISTINCT run_id FROM gone);
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER checkpoints_deleted AFTER DELETE ON checkpoints
    REFERENCING OLD TABLE AS gone
    FOR EACH STATEMENT EXECUTE FUNCTION queue_checkpoints_garbage();

-- Committing changes the run's checkpoint count too (one at a time: per row is fine).
CREATE TRIGGER checkpoints_touch AFTER UPDATE OF state ON checkpoints
    FOR EACH ROW EXECUTE FUNCTION touch_owning_run();
