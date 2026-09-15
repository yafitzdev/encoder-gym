CREATE TABLE workflow_training_benchmark_checks (
    id TEXT PRIMARY KEY NOT NULL,
    training_snapshot_id TEXT NOT NULL
        REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    benchmark_bundle_id TEXT NOT NULL
        REFERENCES workflow_benchmark_bundles(id) ON DELETE RESTRICT,
    contamination_report_id TEXT NOT NULL
        REFERENCES workflow_contamination_reports(id) ON DELETE RESTRICT,
    check_protocol_version TEXT NOT NULL,
    training_input_protocol TEXT NOT NULL,
    status TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    UNIQUE (
        training_snapshot_id,
        benchmark_bundle_id,
        check_protocol_version,
        training_input_protocol
    )
);

CREATE TABLE workflow_training_benchmark_check_cohorts (
    check_id TEXT NOT NULL
        REFERENCES workflow_training_benchmark_checks(id) ON DELETE RESTRICT,
    cohort_id TEXT NOT NULL
        REFERENCES workflow_evaluation_cohorts(id) ON DELETE RESTRICT,
    role_decision_id TEXT NOT NULL
        REFERENCES workflow_cohort_role_decisions(id) ON DELETE RESTRICT,
    split TEXT NOT NULL,
    member_count INTEGER NOT NULL CHECK (member_count > 0),
    PRIMARY KEY (check_id, cohort_id),
    UNIQUE (check_id, split)
);

CREATE INDEX idx_workflow_training_benchmark_checks_snapshot_created
    ON workflow_training_benchmark_checks(training_snapshot_id, created_at, id);
CREATE INDEX idx_workflow_training_benchmark_checks_bundle_created
    ON workflow_training_benchmark_checks(benchmark_bundle_id, created_at, id);
CREATE INDEX idx_workflow_training_benchmark_checks_status_created
    ON workflow_training_benchmark_checks(status, created_at, id);
CREATE INDEX idx_workflow_training_benchmark_check_cohorts_cohort
    ON workflow_training_benchmark_check_cohorts(cohort_id, check_id);

CREATE TRIGGER workflow_training_benchmark_checks_update_guard
BEFORE UPDATE ON workflow_training_benchmark_checks
BEGIN
    SELECT RAISE(ABORT, 'training-benchmark checks are immutable');
END;

CREATE TRIGGER workflow_training_benchmark_checks_delete_guard
BEFORE DELETE ON workflow_training_benchmark_checks
BEGIN
    SELECT RAISE(ABORT, 'training-benchmark checks are immutable');
END;

CREATE TRIGGER workflow_training_benchmark_check_cohorts_update_guard
BEFORE UPDATE ON workflow_training_benchmark_check_cohorts
BEGIN
    SELECT RAISE(ABORT, 'training-benchmark cohort bindings are immutable');
END;

CREATE TRIGGER workflow_training_benchmark_check_cohorts_delete_guard
BEFORE DELETE ON workflow_training_benchmark_check_cohorts
BEGIN
    SELECT RAISE(ABORT, 'training-benchmark cohort bindings are immutable');
END;
