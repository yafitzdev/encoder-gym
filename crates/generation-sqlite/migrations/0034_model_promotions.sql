CREATE TABLE model_promotions (
    id TEXT PRIMARY KEY NOT NULL,
    workflow_run_id TEXT NOT NULL UNIQUE,
    checkpoint_id TEXT NOT NULL,
    training_snapshot_id TEXT NOT NULL,
    development_assessment_id TEXT NOT NULL,
    sealed_assessment_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('promoted', 'rejected', 'inconclusive')),
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workflow_run_id) REFERENCES workflow_runs(id),
    FOREIGN KEY (checkpoint_id) REFERENCES training_checkpoints(id),
    FOREIGN KEY (training_snapshot_id) REFERENCES dataset_snapshots(id),
    FOREIGN KEY (development_assessment_id) REFERENCES workflow_acceptance_assessments(id),
    FOREIGN KEY (sealed_assessment_id) REFERENCES workflow_acceptance_assessments(id)
);
