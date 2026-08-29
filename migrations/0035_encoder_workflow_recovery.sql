CREATE TABLE workflow_execution_leases_new (
    workflow_kind TEXT NOT NULL CHECK (workflow_kind IN ('generation', 'training', 'evaluation', 'encoder_workflow')),
    workflow_id TEXT NOT NULL,
    process_id INTEGER NOT NULL,
    process_started_at INTEGER NOT NULL,
    acquired_at TEXT NOT NULL,
    PRIMARY KEY (workflow_kind, workflow_id)
);
INSERT INTO workflow_execution_leases_new SELECT * FROM workflow_execution_leases;
DROP TABLE workflow_execution_leases;
ALTER TABLE workflow_execution_leases_new RENAME TO workflow_execution_leases;

CREATE TABLE workflow_recovery_records_new (
    workflow_kind TEXT NOT NULL CHECK (workflow_kind IN ('generation', 'training', 'evaluation', 'encoder_workflow')),
    workflow_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'resumed', 'restarted', 'dismissed')),
    resumable_in_place INTEGER NOT NULL CHECK (resumable_in_place IN (0, 1)),
    message TEXT NOT NULL,
    detected_at TEXT NOT NULL,
    replacement_id TEXT,
    resolved_at TEXT,
    PRIMARY KEY (workflow_kind, workflow_id)
);
INSERT INTO workflow_recovery_records_new SELECT * FROM workflow_recovery_records;
DROP TABLE workflow_recovery_records;
ALTER TABLE workflow_recovery_records_new RENAME TO workflow_recovery_records;
CREATE INDEX workflow_recovery_records_state
    ON workflow_recovery_records(state, detected_at, workflow_kind, workflow_id);
