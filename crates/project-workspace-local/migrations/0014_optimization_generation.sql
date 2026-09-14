CREATE TABLE optimization_generation_tasks (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    FOREIGN KEY(run_id, iteration) REFERENCES optimization_agent_scopes(run_id, iteration)
);
CREATE TABLE optimization_generation_calls (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES optimization_generation_tasks(id),
    attempt INTEGER NOT NULL CHECK(attempt > 0),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    process_id INTEGER NOT NULL,
    process_started_at INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(task_id, attempt)
);
CREATE TABLE optimization_generation_outcomes (
    call_id TEXT PRIMARY KEY REFERENCES optimization_generation_calls(id),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL
);
CREATE TRIGGER optimization_generation_tasks_no_update BEFORE UPDATE ON optimization_generation_tasks BEGIN SELECT RAISE(ABORT, 'generation tasks are immutable'); END;
CREATE TRIGGER optimization_generation_tasks_no_delete BEFORE DELETE ON optimization_generation_tasks BEGIN SELECT RAISE(ABORT, 'generation tasks are immutable'); END;
CREATE TRIGGER optimization_generation_calls_no_update BEFORE UPDATE ON optimization_generation_calls BEGIN SELECT RAISE(ABORT, 'generation calls are immutable'); END;
CREATE TRIGGER optimization_generation_calls_no_delete BEFORE DELETE ON optimization_generation_calls BEGIN SELECT RAISE(ABORT, 'generation calls are immutable'); END;
CREATE TRIGGER optimization_generation_outcomes_no_update BEFORE UPDATE ON optimization_generation_outcomes BEGIN SELECT RAISE(ABORT, 'generation outcomes are immutable'); END;
CREATE TRIGGER optimization_generation_outcomes_no_delete BEFORE DELETE ON optimization_generation_outcomes BEGIN SELECT RAISE(ABORT, 'generation outcomes are immutable'); END;
