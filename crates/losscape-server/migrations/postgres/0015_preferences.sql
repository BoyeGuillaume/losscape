-- A user's own settings that follow them from browser to browser (appearance: theme, chart
-- size...). Read by the UI, which checks each one; the server only bounds the size.
ALTER TABLE users ADD COLUMN preferences JSONB NOT NULL DEFAULT '{}'::jsonb;
