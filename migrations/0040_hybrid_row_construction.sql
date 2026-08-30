ALTER TABLE generated_rows
    ADD COLUMN fields_json TEXT NOT NULL DEFAULT '{}';

ALTER TABLE generated_rows
    ADD COLUMN construction_json TEXT;

ALTER TABLE dataset_source_rows
    ADD COLUMN fields_json TEXT NOT NULL DEFAULT '{}';

ALTER TABLE dataset_snapshot_members
    ADD COLUMN fields_json TEXT NOT NULL DEFAULT '{}';
