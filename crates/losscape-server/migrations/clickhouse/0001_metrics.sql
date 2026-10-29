-- One row per logged scalar. ReplacingMergeTree keyed on (run, key, step) keeps the row
-- with the highest seq: a batch re-sent after a server crash collapses into one row, and
-- logging the same step twice keeps the last value.
CREATE TABLE IF NOT EXISTS metrics
(
    project_id   UInt64,
    run_id       UUID,
    key          LowCardinality(String),
    step         UInt64        CODEC(DoubleDelta, ZSTD),
    ts           DateTime64(3) CODEC(DoubleDelta, ZSTD),  -- client clock
    value        Float64       CODEC(Gorilla, ZSTD),
    seq          UInt64        CODEC(DoubleDelta, ZSTD),
    ingested_at  DateTime64(3) DEFAULT now64(3) CODEC(DoubleDelta, ZSTD)  -- server clock
)
ENGINE = ReplacingMergeTree(seq)
ORDER BY (project_id, run_id, key, step)
