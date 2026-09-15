CREATE TABLE optimization_training_attempts (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration_id TEXT NOT NULL REFERENCES optimization_iteration_training(iteration_id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TABLE optimization_training_completions (
    attempt_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_training_attempts(id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TRIGGER immutable_training_attempt_update BEFORE UPDATE ON optimization_training_attempts
BEGIN SELECT RAISE(ABORT, 'Training attempt is immutable'); END;
CREATE TRIGGER immutable_training_attempt_delete BEFORE DELETE ON optimization_training_attempts
BEGIN SELECT RAISE(ABORT, 'Training attempt is immutable'); END;
CREATE TRIGGER immutable_training_completion_update BEFORE UPDATE ON optimization_training_completions
BEGIN SELECT RAISE(ABORT, 'Training completion is immutable'); END;
CREATE TRIGGER immutable_training_completion_delete BEFORE DELETE ON optimization_training_completions
BEGIN SELECT RAISE(ABORT, 'Training completion is immutable'); END;
