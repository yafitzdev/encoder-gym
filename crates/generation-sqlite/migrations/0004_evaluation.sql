CREATE TABLE evaluation_runs (
    id TEXT PRIMARY KEY NOT NULL,
    checkpoint_id TEXT NOT NULL REFERENCES training_checkpoints(id) ON DELETE RESTRICT,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    split TEXT NOT NULL CHECK (split IN ('train', 'validation', 'test')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed')),
    example_count INTEGER NOT NULL DEFAULT 0,
    metrics_json TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE evaluation_predictions (
    id TEXT PRIMARY KEY NOT NULL,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    snapshot_member_id TEXT NOT NULL REFERENCES dataset_snapshot_members(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES generated_rows(id) ON DELETE RESTRICT,
    text TEXT NOT NULL,
    expected_label TEXT NOT NULL,
    predicted_label TEXT NOT NULL,
    confidence REAL NOT NULL,
    probabilities_json TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (evaluation_run_id, snapshot_member_id)
);

CREATE INDEX evaluation_runs_checkpoint_idx
    ON evaluation_runs(checkpoint_id, created_at);
CREATE INDEX evaluation_runs_snapshot_idx
    ON evaluation_runs(snapshot_id, created_at);
CREATE INDEX evaluation_predictions_run_idx
    ON evaluation_predictions(evaluation_run_id, created_at);
