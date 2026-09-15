CREATE TABLE workflow_benchmark_suites (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('development', 'sealed_acceptance')),
    contamination_report_id TEXT NOT NULL REFERENCES workflow_contamination_reports(id) ON DELETE RESTRICT,
    contamination_override_fingerprint TEXT,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE workflow_benchmark_suite_cohorts (
    suite_id TEXT NOT NULL REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    cohort_id TEXT NOT NULL REFERENCES workflow_evaluation_cohorts(id) ON DELETE RESTRICT,
    role_decision_id TEXT NOT NULL REFERENCES workflow_cohort_role_decisions(id) ON DELETE RESTRICT,
    protocol_fingerprint TEXT NOT NULL,
    PRIMARY KEY (suite_id, cohort_id)
);

CREATE TABLE workflow_acceptance_assessments (
    id TEXT PRIMARY KEY NOT NULL,
    suite_id TEXT NOT NULL REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    checkpoint_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('pass', 'fail', 'inconclusive', 'invalid')),
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_workflow_benchmark_suites_kind_created
    ON workflow_benchmark_suites(kind, created_at, id);
CREATE INDEX idx_workflow_benchmark_suite_cohorts_cohort
    ON workflow_benchmark_suite_cohorts(cohort_id, suite_id);
CREATE INDEX idx_workflow_acceptance_suite_state_created
    ON workflow_acceptance_assessments(suite_id, state, created_at, id);
CREATE INDEX idx_workflow_acceptance_checkpoint_created
    ON workflow_acceptance_assessments(checkpoint_id, created_at, id);
