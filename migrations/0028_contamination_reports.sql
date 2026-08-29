CREATE TABLE workflow_contamination_reports (
    id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('clean', 'blocked')),
    cohort_ids_json TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE workflow_contamination_report_cohorts (
    report_id TEXT NOT NULL REFERENCES workflow_contamination_reports(id) ON DELETE RESTRICT,
    cohort_id TEXT NOT NULL REFERENCES workflow_evaluation_cohorts(id) ON DELETE RESTRICT,
    PRIMARY KEY (report_id, cohort_id)
);

CREATE TABLE workflow_contamination_overrides (
    id TEXT PRIMARY KEY NOT NULL,
    report_id TEXT NOT NULL UNIQUE REFERENCES workflow_contamination_reports(id) ON DELETE RESTRICT,
    report_fingerprint TEXT NOT NULL,
    reason TEXT NOT NULL,
    approved_by TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_workflow_contamination_reports_status_created
    ON workflow_contamination_reports(status, created_at, id);
CREATE INDEX idx_workflow_contamination_report_cohorts_cohort
    ON workflow_contamination_report_cohorts(cohort_id, report_id);
