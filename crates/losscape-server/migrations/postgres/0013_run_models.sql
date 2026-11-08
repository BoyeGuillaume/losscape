-- What runs recorded about their models, by model name: the latest description of each.
ALTER TABLE runs ADD COLUMN models JSONB NOT NULL DEFAULT '{}';
