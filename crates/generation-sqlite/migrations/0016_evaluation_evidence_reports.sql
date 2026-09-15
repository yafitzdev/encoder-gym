CREATE TABLE evaluation_comparisons (
    id TEXT PRIMARY KEY NOT NULL,
    left_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    right_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    cohort_fingerprint TEXT NOT NULL,
    protocol_fingerprint TEXT NOT NULL,
    report_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    CHECK (left_run_id <> right_run_id)
);

CREATE TABLE model_selection_reports (
    id TEXT PRIMARY KEY NOT NULL,
    report_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    selected_evaluation_run_id TEXT REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    selected_checkpoint_id TEXT REFERENCES training_checkpoints(id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL
);

CREATE TABLE model_selection_candidates (
    selection_report_id TEXT NOT NULL REFERENCES model_selection_reports(id) ON DELETE RESTRICT,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    candidate_order INTEGER NOT NULL CHECK (candidate_order >= 0),
    PRIMARY KEY (selection_report_id, evaluation_run_id),
    UNIQUE (selection_report_id, candidate_order)
);

CREATE INDEX evaluation_comparisons_runs_idx
    ON evaluation_comparisons(left_run_id, right_run_id, created_at);
CREATE INDEX model_selection_candidates_run_idx
    ON model_selection_candidates(evaluation_run_id, selection_report_id);
