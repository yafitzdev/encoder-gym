CREATE TABLE project_optimization_iterations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    scope_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL,
    UNIQUE(run_id, iteration)
);
CREATE TRIGGER immutable_project_optimization_iterations_update
BEFORE UPDATE ON project_optimization_iterations
BEGIN SELECT RAISE(ABORT, 'Optimization iteration inputs are immutable'); END;
CREATE TRIGGER immutable_project_optimization_iterations_delete
BEFORE DELETE ON project_optimization_iterations
BEGIN SELECT RAISE(ABORT, 'Optimization iteration inputs are immutable'); END;
