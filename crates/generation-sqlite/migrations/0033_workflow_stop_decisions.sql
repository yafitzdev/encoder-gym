CREATE TABLE workflow_stop_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    workflow_run_id TEXT NOT NULL,
    workflow_iteration INTEGER NOT NULL CHECK (workflow_iteration >= 0),
    acceptance_assessment_id TEXT NOT NULL,
    should_continue INTEGER NOT NULL CHECK (should_continue IN (0, 1)),
    reason TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workflow_run_id) REFERENCES workflow_runs(id),
    FOREIGN KEY (acceptance_assessment_id) REFERENCES workflow_acceptance_assessments(id),
    UNIQUE (workflow_run_id, workflow_iteration)
);
