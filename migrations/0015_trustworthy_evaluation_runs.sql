PRAGMA legacy_alter_table = ON;

ALTER TABLE evaluation_runs RENAME TO evaluation_runs_legacy;

CREATE TABLE evaluation_runs (
    id TEXT PRIMARY KEY NOT NULL,
    checkpoint_id TEXT NOT NULL REFERENCES training_checkpoints(id) ON DELETE RESTRICT,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    split TEXT NOT NULL CHECK (split IN ('train', 'validation', 'test')),
    input_fingerprint TEXT,
    protocol_json TEXT NOT NULL,
    protocol_fingerprint TEXT NOT NULL,
    source_identity_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    total_examples INTEGER NOT NULL DEFAULT 0 CHECK (total_examples >= 0),
    processed_examples INTEGER NOT NULL DEFAULT 0 CHECK (processed_examples >= 0),
    completed_batches INTEGER NOT NULL DEFAULT 0 CHECK (completed_batches >= 0),
    current_batch INTEGER NOT NULL DEFAULT 0 CHECK (current_batch >= 0),
    correct_predictions INTEGER NOT NULL DEFAULT 0 CHECK (correct_predictions >= 0),
    elapsed_milliseconds INTEGER NOT NULL DEFAULT 0 CHECK (elapsed_milliseconds >= 0),
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    example_count INTEGER NOT NULL DEFAULT 0 CHECK (example_count >= 0),
    metrics_json TEXT,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO evaluation_runs (
    id, checkpoint_id, snapshot_id, split, input_fingerprint,
    protocol_json, protocol_fingerprint, source_identity_json, state,
    total_examples, processed_examples, completed_batches, current_batch, correct_predictions,
    elapsed_milliseconds, cancel_requested, example_count, metrics_json,
    error_message, created_at, updated_at
)
SELECT
    id, checkpoint_id, snapshot_id, split, input_fingerprint,
    json_object(
        'split', split,
        'batch_size', 32,
        'top_k', json_array(1),
        'calibration_bins', 10,
        'minimum_slice_support', 1,
        'dimension_intersections', json_array(),
        'bootstrap_samples', 1000,
        'statistical_seed', 42,
        'confidence_level', 0.95
    ),
    'legacy:unavailable',
    json_object(
        'checkpoint_checksum', 'legacy:unavailable',
        'checkpoint_model_format', 'legacy:unavailable',
        'base_model_fingerprint', NULL,
        'tokenizer_fingerprint', NULL,
        'snapshot_fingerprint', 'legacy:unavailable',
        'cohort_fingerprint', 'legacy:unavailable',
        'labels', json_array()
    ),
    state,
    CASE WHEN state = 'completed' THEN example_count ELSE 0 END,
    CASE WHEN state = 'completed' THEN example_count ELSE 0 END,
    0, 0,
    CASE
        WHEN state = 'completed' AND metrics_json IS NOT NULL
        THEN COALESCE(json_extract(metrics_json, '$.overall.correct'), 0)
        ELSE 0
    END,
    0, 0, example_count, metrics_json, error_message, created_at, updated_at
FROM evaluation_runs_legacy;

DROP TABLE evaluation_runs_legacy;

ALTER TABLE evaluation_predictions RENAME TO evaluation_predictions_legacy;

CREATE TABLE evaluation_predictions (
    id TEXT PRIMARY KEY NOT NULL,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    snapshot_member_id TEXT NOT NULL REFERENCES dataset_snapshot_members(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    text TEXT NOT NULL,
    expected_label TEXT NOT NULL,
    predicted_label TEXT NOT NULL,
    confidence REAL NOT NULL,
    probabilities_json TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (evaluation_run_id, snapshot_member_id)
);

INSERT INTO evaluation_predictions SELECT * FROM evaluation_predictions_legacy;
DROP TABLE evaluation_predictions_legacy;

ALTER TABLE analysis_reports RENAME TO analysis_reports_legacy;

CREATE TABLE analysis_reports (
    id TEXT PRIMARY KEY NOT NULL,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    minimum_support INTEGER NOT NULL CHECK (minimum_support > 0),
    prediction_count INTEGER NOT NULL,
    error_count INTEGER NOT NULL,
    report_json TEXT NOT NULL,
    fingerprint TEXT,
    created_at TEXT NOT NULL
);

INSERT INTO analysis_reports (
    id, evaluation_run_id, minimum_support, prediction_count, error_count,
    report_json, fingerprint, created_at
)
SELECT
    id, evaluation_run_id, minimum_support, prediction_count, error_count,
    report_json, fingerprint, created_at
FROM analysis_reports_legacy;
DROP TABLE analysis_reports_legacy;

ALTER TABLE optimization_proposals RENAME TO optimization_proposals_legacy;

CREATE TABLE optimization_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    analysis_report_id TEXT NOT NULL REFERENCES analysis_reports(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    additional_example_budget INTEGER NOT NULL CHECK (additional_example_budget > 0),
    minimum_support INTEGER NOT NULL CHECK (minimum_support > 0),
    proposal_json TEXT NOT NULL,
    fingerprint TEXT,
    created_at TEXT NOT NULL
);

INSERT INTO optimization_proposals (
    id, analysis_report_id, dataset_id, additional_example_budget,
    minimum_support, proposal_json, fingerprint, created_at
)
SELECT
    id, analysis_report_id, dataset_id, additional_example_budget,
    minimum_support, proposal_json, fingerprint, created_at
FROM optimization_proposals_legacy;
DROP TABLE optimization_proposals_legacy;

ALTER TABLE optimization_proposal_applications RENAME TO optimization_proposal_applications_legacy;

CREATE TABLE optimization_proposal_applications (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    generation_plan_id TEXT NOT NULL UNIQUE REFERENCES generation_plans(id) ON DELETE RESTRICT,
    applied_at TEXT NOT NULL
);

INSERT INTO optimization_proposal_applications
SELECT * FROM optimization_proposal_applications_legacy;
DROP TABLE optimization_proposal_applications_legacy;

PRAGMA legacy_alter_table = OFF;

CREATE INDEX evaluation_runs_checkpoint_idx_v2
    ON evaluation_runs(checkpoint_id, created_at, id);
CREATE INDEX evaluation_runs_snapshot_idx_v2
    ON evaluation_runs(snapshot_id, split, created_at, id);
CREATE INDEX evaluation_runs_state_idx_v2
    ON evaluation_runs(state, updated_at, id);
CREATE INDEX evaluation_predictions_filter_idx
    ON evaluation_predictions(evaluation_run_id, expected_label, predicted_label, confidence);
CREATE INDEX evaluation_predictions_run_idx_v3
    ON evaluation_predictions(evaluation_run_id, snapshot_member_id, id);
CREATE INDEX analysis_reports_evaluation_idx_v2
    ON analysis_reports(evaluation_run_id, created_at, id);
CREATE INDEX optimization_proposals_analysis_idx_v2
    ON optimization_proposals(analysis_report_id, created_at, id);
