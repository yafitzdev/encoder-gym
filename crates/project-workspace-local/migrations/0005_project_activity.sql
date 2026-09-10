CREATE TABLE project_activity_events (
    id TEXT PRIMARY KEY NOT NULL,
    action_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    previous_event_fingerprint TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    operation TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('started', 'progress', 'succeeded', 'failed')),
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (action_id, sequence)
);

CREATE INDEX idx_project_activity_actions
ON project_activity_events(action_id, sequence);

CREATE INDEX idx_project_activity_recent
ON project_activity_events(created_at, id);

CREATE TRIGGER project_activity_events_no_update
BEFORE UPDATE ON project_activity_events
BEGIN
    SELECT RAISE(ABORT, 'project activity events are immutable');
END;

CREATE TRIGGER project_activity_events_no_delete
BEFORE DELETE ON project_activity_events
BEGIN
    SELECT RAISE(ABORT, 'project activity events are append-only');
END;
