CREATE TABLE optimization_agent_final_authorizations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES project_optimization_runs(id),
    launch_id TEXT NOT NULL REFERENCES optimization_launch_authorizations(id),
    completion_id TEXT NOT NULL REFERENCES optimization_iteration_completions(iteration_id),
    iteration_id TEXT NOT NULL REFERENCES project_optimization_iterations(id),
    scope_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TRIGGER immutable_agent_final_authorization_update BEFORE UPDATE ON optimization_agent_final_authorizations
BEGIN SELECT RAISE(ABORT, 'Final holdout authority is immutable'); END;
CREATE TRIGGER immutable_agent_final_authorization_delete BEFORE DELETE ON optimization_agent_final_authorizations
BEGIN SELECT RAISE(ABORT, 'Final holdout authority is immutable'); END;
