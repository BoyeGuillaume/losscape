-- A renamed metric can also be rescaled (val/pck-8-fine logged in %, shown as val/pck-8 in
-- 0-1). Renames replace merges: they say exactly which runs a name applies to.
ALTER TABLE run_metric_renames ADD COLUMN scale DOUBLE PRECISION NOT NULL DEFAULT 1
    CHECK (scale > 0);

DELETE FROM metric_definitions WHERE kind = 'merge';
ALTER TABLE metric_definitions DROP CONSTRAINT metric_definitions_kind_check;
ALTER TABLE metric_definitions ADD CONSTRAINT metric_definitions_kind_check
    CHECK (kind IN ('describe', 'family', 'formula'));
