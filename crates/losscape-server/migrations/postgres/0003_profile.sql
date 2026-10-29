-- Profile pictures: always a server-encoded 256x256 PNG (uploads are decoded and
-- re-encoded, never stored as sent).
CREATE TABLE user_avatars (
    user_id     BIGINT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    image       BYTEA NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
