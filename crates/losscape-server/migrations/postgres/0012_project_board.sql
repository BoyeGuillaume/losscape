-- A project's board (Overview tab): the query, sort and limit choosing its runs. NULL: defaults.
ALTER TABLE projects ADD COLUMN board JSONB;
