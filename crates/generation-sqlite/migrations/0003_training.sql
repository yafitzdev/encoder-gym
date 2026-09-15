CREATE TABLE training_runs (
    id TEXT PRIMARY KEY NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    backend_name TEXT NOT NULL,
    model_format TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    configuration_json TEXT NOT NULL,
    completed_epochs INTEGER NOT NULL DEFAULT 0,
    latest_training_loss REAL,
    latest_validation_loss REAL,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE training_checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES training_runs(id) ON DELETE RESTRICT,
    epoch INTEGER NOT NULL,
    artifact_path TEXT NOT NULL UNIQUE,
    artifact_checksum TEXT NOT NULL,
    model_format TEXT NOT NULL,
    training_loss REAL NOT NULL,
    validation_loss REAL,
    is_final INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (run_id, epoch)
);

CREATE INDEX training_runs_snapshot_idx ON training_runs(snapshot_id, created_at);
CREATE INDEX training_checkpoints_run_idx ON training_checkpoints(run_id, epoch);
