CREATE TABLE workflow_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    project_configuration_id TEXT NOT NULL REFERENCES project_configurations(id) ON DELETE RESTRICT,
    development_suite_id TEXT NOT NULL REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    sealed_suite_id TEXT REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE workflow_runs (
    id TEXT PRIMARY KEY NOT NULL,
    definition_id TEXT NOT NULL REFERENCES workflow_definitions(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN (
        'queued', 'running', 'awaiting_approval', 'awaiting_user',
        'development_complete', 'completed', 'failed', 'cancelled',
        'exhausted', 'inconclusive'
    )),
    current_stage TEXT,
    iteration INTEGER NOT NULL,
    latest_attempt_id TEXT,
    latest_attempt_fingerprint TEXT,
    cancel_requested INTEGER NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE workflow_stage_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    workflow_run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL,
    iteration INTEGER NOT NULL,
    stage TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'running', 'completed', 'failed', 'cancelled', 'awaiting_approval',
        'awaiting_user', 'inconclusive', 'exhausted'
    )),
    predecessor_id TEXT REFERENCES workflow_stage_attempts(id) ON DELETE RESTRICT,
    predecessor_fingerprint TEXT,
    retryable INTEGER NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (workflow_run_id, sequence),
    UNIQUE (predecessor_id)
);

CREATE TABLE workflow_artifact_links (
    workflow_stage_attempt_id TEXT NOT NULL REFERENCES workflow_stage_attempts(id) ON DELETE RESTRICT,
    artifact_kind TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    artifact_fingerprint TEXT NOT NULL,
    PRIMARY KEY (workflow_stage_attempt_id, artifact_kind, artifact_id)
);

CREATE INDEX idx_workflow_definitions_dataset_created
    ON workflow_definitions(dataset_id, created_at, id);
CREATE INDEX idx_workflow_runs_definition_state_updated
    ON workflow_runs(definition_id, state, updated_at, id);
CREATE INDEX idx_workflow_stage_attempts_run_sequence
    ON workflow_stage_attempts(workflow_run_id, sequence);
CREATE INDEX idx_workflow_artifact_links_artifact
    ON workflow_artifact_links(artifact_kind, artifact_id);
