CREATE TABLE workflow_child_executions (
    id TEXT PRIMARY KEY NOT NULL,
    workflow_run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE RESTRICT,
    workflow_stage_attempt_id TEXT NOT NULL REFERENCES workflow_stage_attempts(id) ON DELETE RESTRICT,
    stage TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    child_kind TEXT NOT NULL CHECK (child_kind IN (
        'generation_job', 'quality_audit_run', 'training_run', 'evaluation_run'
    )),
    logical_key TEXT NOT NULL CHECK (
        length(logical_key) BETWEEN 1 AND 256 AND trim(logical_key) = logical_key
    ),
    child_execution_id TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    UNIQUE (workflow_stage_attempt_id, ordinal),
    UNIQUE (child_kind, child_execution_id)
);

CREATE INDEX idx_workflow_child_executions_attempt
    ON workflow_child_executions(workflow_stage_attempt_id, ordinal);
CREATE INDEX idx_workflow_child_executions_run
    ON workflow_child_executions(workflow_run_id, created_at, id);

CREATE TRIGGER workflow_child_executions_insert_authority
BEFORE INSERT ON workflow_child_executions
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM workflow_stage_attempts AS attempt
        JOIN workflow_runs AS run ON run.id = attempt.workflow_run_id
        WHERE attempt.id = NEW.workflow_stage_attempt_id
          AND attempt.workflow_run_id = NEW.workflow_run_id
          AND attempt.stage = NEW.stage
          AND attempt.state = 'running'
          AND run.latest_attempt_id = attempt.id
          AND run.state = 'running'
          AND run.cancel_requested = 0
    ) THEN RAISE(ABORT, 'workflow child execution has no current running parent attempt') END;

    SELECT CASE WHEN NEW.ordinal != (
        SELECT COUNT(*) + 1
        FROM workflow_child_executions
        WHERE workflow_stage_attempt_id = NEW.workflow_stage_attempt_id
    ) THEN RAISE(ABORT, 'workflow child execution ordinal is not consecutive') END;

    SELECT CASE WHEN NOT (
        (NEW.stage IN ('generation', 'dataset_diff_generation') AND NEW.child_kind = 'generation_job')
        OR (NEW.stage IN ('quality_audit', 'iteration_quality_audit') AND NEW.child_kind = 'quality_audit_run')
        OR (NEW.stage IN ('training', 'iteration_training') AND NEW.child_kind = 'training_run')
        OR (NEW.stage IN ('development_evaluation', 'iteration_evaluation', 'sealed_evaluation') AND NEW.child_kind = 'evaluation_run')
    ) THEN RAISE(ABORT, 'workflow child execution kind is not allowed for stage') END;
END;

CREATE TRIGGER workflow_child_executions_immutable_update
BEFORE UPDATE ON workflow_child_executions
BEGIN
    SELECT RAISE(ABORT, 'workflow child executions are immutable');
END;

CREATE TRIGGER workflow_child_executions_immutable_delete
BEFORE DELETE ON workflow_child_executions
BEGIN
    SELECT RAISE(ABORT, 'workflow child executions are immutable');
END;
