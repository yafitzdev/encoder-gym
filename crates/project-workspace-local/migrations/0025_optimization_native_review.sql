CREATE TABLE IF NOT EXISTS optimization_native_review_calls (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    category TEXT NOT NULL CHECK(category IN ('blind_semantic_assessment', 'repair_target_fit_assessment')),
    request_id TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    attempt INTEGER NOT NULL CHECK(attempt BETWEEN 1 AND 2),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    process_id INTEGER NOT NULL,
    process_started_at INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(request_fingerprint, attempt)
);
CREATE TRIGGER IF NOT EXISTS immutable_optimization_native_review_calls_update BEFORE UPDATE ON optimization_native_review_calls
BEGIN SELECT RAISE(ABORT, 'Native review call reservations are immutable'); END;
CREATE TRIGGER IF NOT EXISTS immutable_optimization_native_review_calls_delete BEFORE DELETE ON optimization_native_review_calls
BEGIN SELECT RAISE(ABORT, 'Native review call reservations are immutable'); END;

CREATE TABLE IF NOT EXISTS optimization_native_review_outcomes (
    call_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_native_review_calls(id),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS immutable_optimization_native_review_outcomes_update BEFORE UPDATE ON optimization_native_review_outcomes
BEGIN SELECT RAISE(ABORT, 'Native review outcomes are immutable'); END;
CREATE TRIGGER IF NOT EXISTS immutable_optimization_native_review_outcomes_delete BEFORE DELETE ON optimization_native_review_outcomes
BEGIN SELECT RAISE(ABORT, 'Native review outcomes are immutable'); END;

CREATE TABLE IF NOT EXISTS optimization_native_admissions (
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    row_id TEXT NOT NULL,
    row_fingerprint TEXT NOT NULL,
    decision TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(run_id, iteration, row_id)
);
CREATE TRIGGER IF NOT EXISTS immutable_optimization_native_admissions_update BEFORE UPDATE ON optimization_native_admissions
BEGIN SELECT RAISE(ABORT, 'Native admission evidence is immutable'); END;
CREATE TRIGGER IF NOT EXISTS immutable_optimization_native_admissions_delete BEFORE DELETE ON optimization_native_admissions
BEGIN SELECT RAISE(ABORT, 'Native admission evidence is immutable'); END;
