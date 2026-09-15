CREATE TABLE workflow_execution_leases (
    workflow_kind TEXT NOT NULL CHECK (workflow_kind IN ('generation', 'training', 'evaluation')),
    workflow_id TEXT NOT NULL,
    process_id INTEGER NOT NULL,
    process_started_at INTEGER NOT NULL,
    acquired_at TEXT NOT NULL,
    PRIMARY KEY (workflow_kind, workflow_id)
);

CREATE TABLE workflow_recovery_records (
    workflow_kind TEXT NOT NULL CHECK (workflow_kind IN ('generation', 'training', 'evaluation')),
    workflow_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'resumed', 'restarted', 'dismissed')),
    resumable_in_place INTEGER NOT NULL,
    message TEXT NOT NULL,
    detected_at TEXT NOT NULL,
    replacement_id TEXT,
    resolved_at TEXT,
    PRIMARY KEY (workflow_kind, workflow_id)
);

CREATE INDEX workflow_recovery_state_detected_idx
    ON workflow_recovery_records(state, detected_at, workflow_kind, workflow_id);
