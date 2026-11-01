-- Console output, one row per line. A line the client sends again (a progress bar being
-- redrawn, at most once a second) replaces the earlier version: ReplacingMergeTree keeps
-- the row from the latest batch (seq). Readers use FINAL.
CREATE TABLE IF NOT EXISTS console_lines
(
    project_id  UInt64,
    run_id      UUID,
    line        UInt64        CODEC(DoubleDelta, ZSTD),
    ts          DateTime64(3) CODEC(DoubleDelta, ZSTD),  -- client clock
    stream      Enum8('stdout' = 1, 'stderr' = 2),
    text        String        CODEC(ZSTD(3)),
    seq         UInt64        CODEC(DoubleDelta, ZSTD)
)
ENGINE = ReplacingMergeTree(seq)
ORDER BY (project_id, run_id, line)
