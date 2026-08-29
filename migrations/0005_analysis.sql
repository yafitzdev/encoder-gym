CREATE TABLE analysis_reports (
    id TEXT PRIMARY KEY NOT NULL,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    minimum_support INTEGER NOT NULL CHECK (minimum_support > 0),
    prediction_count INTEGER NOT NULL,
    error_count INTEGER NOT NULL,
    report_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX analysis_reports_evaluation_idx
    ON analysis_reports(evaluation_run_id, created_at);
