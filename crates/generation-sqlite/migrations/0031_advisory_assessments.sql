CREATE TABLE advisory_assessments (
    id TEXT PRIMARY KEY NOT NULL,
    workflow_run_id TEXT NOT NULL,
    workflow_iteration INTEGER NOT NULL CHECK (workflow_iteration >= 0),
    analysis_report_id TEXT NOT NULL,
    acceptance_assessment_id TEXT NOT NULL,
    backend TEXT NOT NULL,
    model TEXT NOT NULL,
    egress_policy TEXT NOT NULL CHECK (egress_policy IN ('aggregate_only', 'development_text')),
    prompt_version TEXT NOT NULL,
    prompt_fingerprint TEXT NOT NULL,
    input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
    total_tokens INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0),
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workflow_run_id) REFERENCES workflow_runs(id),
    FOREIGN KEY (analysis_report_id) REFERENCES analysis_reports(id),
    FOREIGN KEY (acceptance_assessment_id) REFERENCES workflow_acceptance_assessments(id)
);

CREATE UNIQUE INDEX advisory_assessments_request_identity
    ON advisory_assessments(workflow_run_id, workflow_iteration, analysis_report_id, prompt_fingerprint);
CREATE INDEX advisory_assessments_workflow_run
    ON advisory_assessments(workflow_run_id, created_at, id);
CREATE INDEX advisory_assessments_analysis_report
    ON advisory_assessments(analysis_report_id, created_at, id);
