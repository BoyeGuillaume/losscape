CREATE TABLE users (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username       TEXT NOT NULL,
    display_name   TEXT,
    email          TEXT,
    -- Argon2id PHC string (algorithm, parameters and salt included). NULL for accounts
    -- that only sign in through an external provider.
    password_hash  TEXT,
    is_admin       BOOLEAN NOT NULL DEFAULT false,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at    TIMESTAMPTZ
);
CREATE UNIQUE INDEX users_username ON users (lower(username));

-- Accounts at external providers (OIDC / GitHub) linked to a user.
CREATE TABLE user_identities (
    provider    TEXT NOT NULL,
    subject     TEXT NOT NULL,
    user_id     BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX user_identities_user ON user_identities (user_id);

-- Browser sessions. The cookie holds a random token; only its SHA-256 is stored, so a
-- leaked table can't be replayed.
CREATE TABLE sessions (
    token_hash  BYTEA PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,
    user_agent  TEXT
);
CREATE INDEX sessions_user ON sessions (user_id);

-- API keys for the SDK. Keys are 256-bit random values, so a fast hash is enough (a slow
-- password hash would add latency to every SDK request for no security gain).
CREATE TABLE api_keys (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id       BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    prefix        TEXT NOT NULL,  -- first characters, shown in the UI to tell keys apart
    key_hash      BYTEA NOT NULL UNIQUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at  TIMESTAMPTZ,
    revoked_at    TIMESTAMPTZ
);
CREATE INDEX api_keys_user ON api_keys (user_id);

-- In-flight OAuth sign-ins (state -> PKCE verifier and nonce), valid for a few minutes.
CREATE TABLE oauth_states (
    state_hash     BYTEA PRIMARY KEY,
    provider       TEXT NOT NULL,
    pkce_verifier  TEXT NOT NULL,
    nonce          TEXT NOT NULL,
    link_user_id   BIGINT REFERENCES users (id) ON DELETE CASCADE,
    return_to      TEXT NOT NULL DEFAULT '/',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE project_members (
    project_id  BIGINT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    user_id     BIGINT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);
CREATE INDEX project_members_user ON project_members (user_id);
