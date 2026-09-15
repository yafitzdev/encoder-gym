CREATE TABLE dataset_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    source_dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT,
    split_configuration_json TEXT NOT NULL,
    member_count INTEGER NOT NULL CHECK (member_count > 0),
    created_at TEXT NOT NULL
);

CREATE TABLE dataset_snapshot_members (
    id TEXT PRIMARY KEY NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES generated_rows(id) ON DELETE RESTRICT,
    split TEXT NOT NULL CHECK (split IN ('train', 'validation', 'test')),
    text TEXT NOT NULL,
    label TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    source_created_at TEXT NOT NULL,
    UNIQUE (snapshot_id, source_row_id)
);

CREATE INDEX dataset_snapshots_source_dataset_idx
    ON dataset_snapshots(source_dataset_id, created_at);
CREATE INDEX dataset_snapshot_members_snapshot_split_idx
    ON dataset_snapshot_members(snapshot_id, split, label);
