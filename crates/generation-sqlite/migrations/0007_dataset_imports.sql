CREATE TABLE dataset_imports (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    source_path TEXT NOT NULL,
    source_format TEXT NOT NULL CHECK (source_format IN ('jsonl', 'csv')),
    mapping_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed')),
    processed_rows INTEGER NOT NULL DEFAULT 0,
    accepted_rows INTEGER NOT NULL DEFAULT 0,
    rejected_rows INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE imported_rows (
    id TEXT PRIMARY KEY NOT NULL,
    import_id TEXT NOT NULL REFERENCES dataset_imports(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    source_row_number INTEGER NOT NULL CHECK (source_row_number > 0),
    cell_key TEXT,
    text TEXT NOT NULL,
    normalized_text TEXT NOT NULL,
    label TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    validation_status TEXT NOT NULL CHECK (validation_status IN ('accepted', 'rejected')),
    validation_errors_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (import_id, source_row_number)
);

CREATE TABLE dataset_source_rows (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('generated', 'imported')),
    source_ref TEXT NOT NULL,
    cell_key TEXT NOT NULL,
    text TEXT NOT NULL,
    normalized_text TEXT NOT NULL,
    label TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (source_kind, source_ref)
);

INSERT INTO dataset_source_rows (
    id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text,
    label, dimensions_json, provenance_json, created_at
)
SELECT
    id,
    dataset_id,
    'generated',
    id,
    cell_key,
    text,
    normalized_text,
    label,
    dimensions_json,
    json_object(
        'kind', 'generated',
        'generation_job_id', generation_job_id,
        'backend', generator_backend,
        'model', generator_model
    ),
    created_at
FROM generated_rows
WHERE validation_status = 'accepted';

PRAGMA legacy_alter_table = ON;

ALTER TABLE dataset_snapshot_members RENAME TO dataset_snapshot_members_legacy;

CREATE TABLE dataset_snapshot_members (
    id TEXT PRIMARY KEY NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    split TEXT NOT NULL CHECK (split IN ('train', 'validation', 'test')),
    text TEXT NOT NULL,
    label TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    source_provenance_json TEXT NOT NULL,
    source_created_at TEXT NOT NULL,
    UNIQUE (snapshot_id, source_row_id)
);

INSERT INTO dataset_snapshot_members (
    id, snapshot_id, source_row_id, split, text, label, dimensions_json,
    source_provenance_json, source_created_at
)
SELECT
    legacy.id,
    legacy.snapshot_id,
    legacy.source_row_id,
    legacy.split,
    legacy.text,
    legacy.label,
    legacy.dimensions_json,
    source.provenance_json,
    legacy.source_created_at
FROM dataset_snapshot_members_legacy AS legacy
JOIN dataset_source_rows AS source ON source.id = legacy.source_row_id;

DROP TABLE dataset_snapshot_members_legacy;

ALTER TABLE evaluation_predictions RENAME TO evaluation_predictions_legacy;

CREATE TABLE evaluation_predictions (
    id TEXT PRIMARY KEY NOT NULL,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    snapshot_member_id TEXT NOT NULL REFERENCES dataset_snapshot_members(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    text TEXT NOT NULL,
    expected_label TEXT NOT NULL,
    predicted_label TEXT NOT NULL,
    confidence REAL NOT NULL,
    probabilities_json TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (evaluation_run_id, snapshot_member_id)
);

INSERT INTO evaluation_predictions (
    id, evaluation_run_id, snapshot_member_id, source_row_id, text,
    expected_label, predicted_label, confidence, probabilities_json,
    dimensions_json, created_at
)
SELECT
    id, evaluation_run_id, snapshot_member_id, source_row_id, text,
    expected_label, predicted_label, confidence, probabilities_json,
    dimensions_json, created_at
FROM evaluation_predictions_legacy;

DROP TABLE evaluation_predictions_legacy;

PRAGMA legacy_alter_table = OFF;

CREATE INDEX dataset_imports_dataset_created_idx
    ON dataset_imports(dataset_id, created_at, id);
CREATE INDEX imported_rows_import_status_idx
    ON imported_rows(import_id, validation_status, source_row_number);
CREATE INDEX imported_rows_dataset_cell_status_idx
    ON imported_rows(dataset_id, cell_key, validation_status);
CREATE INDEX dataset_source_rows_dataset_created_idx
    ON dataset_source_rows(dataset_id, created_at, id);
CREATE INDEX dataset_source_rows_dataset_normalized_idx
    ON dataset_source_rows(dataset_id, normalized_text);
CREATE INDEX dataset_source_rows_dataset_cell_idx
    ON dataset_source_rows(dataset_id, cell_key);
CREATE INDEX dataset_snapshot_members_snapshot_split_idx_v2
    ON dataset_snapshot_members(snapshot_id, split, label);
CREATE INDEX evaluation_predictions_run_idx_v2
    ON evaluation_predictions(evaluation_run_id, created_at);
