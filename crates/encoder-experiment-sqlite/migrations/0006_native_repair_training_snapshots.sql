CREATE TABLE encoder_native_repair_training_snapshots (
    id TEXT PRIMARY KEY NOT NULL,
    specification_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    proposal_id TEXT NOT NULL,
    selection_id TEXT NOT NULL UNIQUE,
    execution_project_snapshot_id TEXT NOT NULL,
    combined_membership_fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id),
    FOREIGN KEY (selection_id) REFERENCES encoder_native_delta_selections(id),
    FOREIGN KEY (execution_project_snapshot_id) REFERENCES encoder_experiment_projects(id)
);

CREATE TRIGGER encoder_native_repair_training_snapshots_no_update
BEFORE UPDATE ON encoder_native_repair_training_snapshots
BEGIN SELECT RAISE(ABORT, 'native repair training snapshots are immutable'); END;

CREATE TRIGGER encoder_native_repair_training_snapshots_no_delete
BEFORE DELETE ON encoder_native_repair_training_snapshots
BEGIN SELECT RAISE(ABORT, 'native repair training snapshots are append-only'); END;
