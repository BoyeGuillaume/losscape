-- Checkpoint hashes are BLAKE3, computed by the server itself as parts arrive (any order),
-- never read back from storage: storage is checked against them instead, on every read.
-- Checkpoints from before (SHA-256, not released) are deleted, their files with them.
DELETE FROM checkpoints;

-- A file's BLAKE3: declared by the client, then the server's own once committed (they match).
ALTER TABLE checkpoint_files RENAME COLUMN sha256 TO blake3;

-- A part as stored: BLAKE3 of its bytes (checked on every read); `content`: what it
-- contributes to its file's BLAKE3 (its subtree's chaining value, or the file's hash if it's
-- the only part), from its content (decompressed).
ALTER TABLE checkpoint_parts RENAME COLUMN sha256 TO blake3;
ALTER TABLE checkpoint_parts ADD COLUMN content BYTEA NOT NULL;

-- `digest`: BLAKE3 of its files listed as b3sum lists them, by path; `corrupt`: what storage
-- gave back that didn't match, if it ever did.
ALTER TABLE checkpoints
    DROP COLUMN archive_sha256,
    DROP COLUMN verified_at,
    DROP COLUMN verify_error,
    ADD COLUMN digest BYTEA,
    ADD COLUMN corrupt TEXT;
