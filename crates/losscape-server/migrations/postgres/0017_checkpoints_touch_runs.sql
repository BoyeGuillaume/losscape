-- A checkpoint committed or deleted changes its run's checkpoint count and size in the run
-- list, which refreshes the runs that changed.
CREATE TRIGGER checkpoints_touch AFTER UPDATE OF state OR DELETE ON checkpoints
    FOR EACH ROW EXECUTE FUNCTION touch_owning_run();
