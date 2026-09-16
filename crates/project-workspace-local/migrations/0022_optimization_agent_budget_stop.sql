CREATE TABLE optimization_agent_execution_events_v3 (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    attempt_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('started', 'interrupted', 'failed', 'completed', 'stop_requested', 'paused', 'budget_exhausted')),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(run_id, sequence)
);
INSERT INTO optimization_agent_execution_events_v3 SELECT * FROM optimization_agent_execution_events;
DROP TABLE optimization_agent_execution_events;
ALTER TABLE optimization_agent_execution_events_v3 RENAME TO optimization_agent_execution_events;
CREATE TRIGGER immutable_agent_execution_update BEFORE UPDATE ON optimization_agent_execution_events
BEGIN SELECT RAISE(ABORT, 'Agent execution history is immutable'); END;
CREATE TRIGGER immutable_agent_execution_delete BEFORE DELETE ON optimization_agent_execution_events
BEGIN SELECT RAISE(ABORT, 'Agent execution history is immutable'); END;
