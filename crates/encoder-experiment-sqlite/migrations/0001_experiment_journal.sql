CREATE TABLE encoder_experiment_projects (
    id TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    source_fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE encoder_experiment_protocols (
    id TEXT PRIMARY KEY NOT NULL,
    project_snapshot_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (project_snapshot_id) REFERENCES encoder_experiment_projects(id)
);

CREATE TABLE encoder_experiment_runs (
    id TEXT PRIMARY KEY NOT NULL,
    protocol_id TEXT NOT NULL,
    last_sequence INTEGER NOT NULL,
    last_event_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (protocol_id) REFERENCES encoder_experiment_protocols(id)
);

CREATE TABLE encoder_experiment_events (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    previous_event_fingerprint TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES encoder_experiment_runs(id),
    UNIQUE (run_id, sequence)
);

CREATE INDEX idx_encoder_experiment_protocols_project
    ON encoder_experiment_protocols(project_snapshot_id, created_at, id);

CREATE INDEX idx_encoder_experiment_runs_protocol
    ON encoder_experiment_runs(protocol_id, created_at, id);

CREATE INDEX idx_encoder_experiment_events_run
    ON encoder_experiment_events(run_id, sequence);
