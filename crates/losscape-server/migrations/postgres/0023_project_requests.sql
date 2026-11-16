-- Projects asked for, when creating one takes an administrator's approval
-- (LOSSCAPE_PROJECTS=approval). Accepted: the project is created, owned by who asked, and the
-- request goes. Rejected: kept, with why, until who asked dismisses it.
CREATE TABLE project_requests (
    id            BIGSERIAL PRIMARY KEY,
    name          TEXT NOT NULL,
    requested_by  BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- What it's for, from who asked.
    note          TEXT NOT NULL DEFAULT '',
    state         TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'rejected')),
    reason        TEXT NOT NULL DEFAULT '',
    decided_by    BIGINT REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at    TIMESTAMPTZ
);
-- One pending request per name.
CREATE UNIQUE INDEX project_requests_pending_name ON project_requests (lower(name))
    WHERE state = 'pending';
CREATE INDEX project_requests_user ON project_requests (requested_by);
