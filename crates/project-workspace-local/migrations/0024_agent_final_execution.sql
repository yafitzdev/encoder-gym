CREATE TABLE optimization_agent_final_dispatches (
    authorization_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_agent_final_authorizations(id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TABLE optimization_agent_final_results (
    authorization_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_agent_final_dispatches(authorization_id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TRIGGER immutable_agent_final_dispatch_update BEFORE UPDATE ON optimization_agent_final_dispatches
BEGIN SELECT RAISE(ABORT, 'Final dispatch is immutable'); END;
CREATE TRIGGER immutable_agent_final_dispatch_delete BEFORE DELETE ON optimization_agent_final_dispatches
BEGIN SELECT RAISE(ABORT, 'Final dispatch is immutable'); END;
CREATE TRIGGER immutable_agent_final_result_update BEFORE UPDATE ON optimization_agent_final_results
BEGIN SELECT RAISE(ABORT, 'Final result is immutable'); END;
CREATE TRIGGER immutable_agent_final_result_delete BEFORE DELETE ON optimization_agent_final_results
BEGIN SELECT RAISE(ABORT, 'Final result is immutable'); END;
