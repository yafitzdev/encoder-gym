CREATE TABLE optimization_agent_scopes (
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    PRIMARY KEY(run_id, iteration)
);
CREATE TRIGGER immutable_optimization_agent_scopes_update BEFORE UPDATE ON optimization_agent_scopes
BEGIN SELECT RAISE(ABORT, 'Agent analysis scopes are immutable'); END;
CREATE TRIGGER immutable_optimization_agent_scopes_delete BEFORE DELETE ON optimization_agent_scopes
BEGIN SELECT RAISE(ABORT, 'Agent analysis scopes are immutable'); END;

CREATE TABLE optimization_agent_calls (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    iteration INTEGER NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 32),
    scope_fingerprint TEXT NOT NULL REFERENCES optimization_agent_scopes(fingerprint),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    process_id INTEGER NOT NULL,
    process_started_at INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(run_id, iteration) REFERENCES optimization_agent_scopes(run_id, iteration),
    UNIQUE(run_id, iteration, sequence)
);
CREATE TRIGGER immutable_optimization_agent_calls_update BEFORE UPDATE ON optimization_agent_calls
BEGIN SELECT RAISE(ABORT, 'Agent call reservations are immutable'); END;
CREATE TRIGGER immutable_optimization_agent_calls_delete BEFORE DELETE ON optimization_agent_calls
BEGIN SELECT RAISE(ABORT, 'Agent call reservations are immutable'); END;

CREATE TABLE optimization_agent_outcomes (
    call_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_agent_calls(id),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL
);
CREATE TRIGGER immutable_optimization_agent_outcomes_update BEFORE UPDATE ON optimization_agent_outcomes
BEGIN SELECT RAISE(ABORT, 'Agent call outcomes are immutable'); END;
CREATE TRIGGER immutable_optimization_agent_outcomes_delete BEFORE DELETE ON optimization_agent_outcomes
BEGIN SELECT RAISE(ABORT, 'Agent call outcomes are immutable'); END;
