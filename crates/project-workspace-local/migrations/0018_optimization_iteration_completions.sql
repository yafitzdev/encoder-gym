CREATE TABLE optimization_iteration_completions (
    iteration_id TEXT PRIMARY KEY NOT NULL REFERENCES project_optimization_iterations(id),
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(run_id, iteration)
);
CREATE TRIGGER immutable_iteration_completions_update BEFORE UPDATE ON optimization_iteration_completions
BEGIN SELECT RAISE(ABORT, 'Iteration completion is immutable'); END;
CREATE TRIGGER immutable_iteration_completions_delete BEFORE DELETE ON optimization_iteration_completions
BEGIN SELECT RAISE(ABORT, 'Iteration completion is immutable'); END;
