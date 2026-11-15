-- Guests read a project's runs, charts and metrics, but not its checkpoints (their files can
-- be what's worth protecting: weights).
ALTER TABLE project_members DROP CONSTRAINT project_members_role_check;
ALTER TABLE project_members ADD CONSTRAINT project_members_role_check
    CHECK (role IN ('owner', 'editor', 'viewer', 'guest'));
