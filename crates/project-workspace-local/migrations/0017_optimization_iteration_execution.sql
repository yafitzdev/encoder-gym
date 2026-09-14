CREATE TABLE optimization_iteration_training (
    iteration_id TEXT PRIMARY KEY NOT NULL REFERENCES project_optimization_iterations(id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TABLE optimization_iteration_results (
    iteration_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_iteration_training(iteration_id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TRIGGER immutable_iteration_training_update BEFORE UPDATE ON optimization_iteration_training
BEGIN SELECT RAISE(ABORT, 'Iteration training is immutable'); END;
CREATE TRIGGER immutable_iteration_training_delete BEFORE DELETE ON optimization_iteration_training
BEGIN SELECT RAISE(ABORT, 'Iteration training is immutable'); END;
CREATE TRIGGER immutable_iteration_results_update BEFORE UPDATE ON optimization_iteration_results
BEGIN SELECT RAISE(ABORT, 'Iteration results are immutable'); END;
CREATE TRIGGER immutable_iteration_results_delete BEFORE DELETE ON optimization_iteration_results
BEGIN SELECT RAISE(ABORT, 'Iteration results are immutable'); END;
