-- Highest step logged (system metrics aside), kept up to date by ingest so run lists can
-- show the number of steps without scanning metrics. Older runs are filled in at startup.
ALTER TABLE runs ADD COLUMN last_step BIGINT;
