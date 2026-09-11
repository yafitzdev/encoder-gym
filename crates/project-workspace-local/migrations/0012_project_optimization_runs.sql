CREATE TABLE project_optimization_runs (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    launch_id TEXT NOT NULL UNIQUE REFERENCES optimization_launch_authorizations(id),
    launch_fingerprint TEXT NOT NULL,
    setup_id TEXT NOT NULL REFERENCES optimization_setups(id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL
);
CREATE INDEX project_optimization_runs_created ON project_optimization_runs(created_at, id);
CREATE TRIGGER immutable_project_optimization_runs_update BEFORE UPDATE ON project_optimization_runs
BEGIN SELECT RAISE(ABORT, 'Project optimization runs are immutable'); END;
CREATE TRIGGER immutable_project_optimization_runs_delete BEFORE DELETE ON project_optimization_runs
BEGIN SELECT RAISE(ABORT, 'Project optimization runs are immutable'); END;

CREATE TABLE project_optimization_events (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    sequence INTEGER NOT NULL,
    previous_event_fingerprint TEXT,
    kind TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL,
    UNIQUE(run_id, sequence)
);
CREATE INDEX project_optimization_events_run ON project_optimization_events(run_id, sequence);
CREATE TRIGGER immutable_project_optimization_events_update BEFORE UPDATE ON project_optimization_events
BEGIN SELECT RAISE(ABORT, 'Project optimization events are immutable'); END;
CREATE TRIGGER immutable_project_optimization_events_delete BEFORE DELETE ON project_optimization_events
BEGIN SELECT RAISE(ABORT, 'Project optimization events are immutable'); END;
