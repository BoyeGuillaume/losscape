-- Important checkpoints are never deleted by retention.
ALTER TABLE checkpoints ADD COLUMN important BOOLEAN NOT NULL DEFAULT false;

-- How long a project keeps checkpoints: `checkpoint_days` after they're uploaded (null: until
-- deleted by hand), except the important ones, each run's last one (`checkpoint_keep_last`) and
-- each run's best one by a metric (`checkpoint_best_metric`, lowest or highest value).
ALTER TABLE projects
    ADD COLUMN checkpoint_days INT CHECK (checkpoint_days BETWEEN 1 AND 3650),
    ADD COLUMN checkpoint_keep_last BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN checkpoint_best_metric TEXT,
    ADD COLUMN checkpoint_best_goal TEXT NOT NULL DEFAULT 'max'
        CHECK (checkpoint_best_goal IN ('min', 'max'));
