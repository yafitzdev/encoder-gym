CREATE TABLE encoder_production_optimization_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    manifest_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    project_snapshot_id TEXT NOT NULL,
    metric_source_protocol_id TEXT NOT NULL,
    training_snapshot_id TEXT NOT NULL,
    benchmark_generation_id TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (project_snapshot_id) REFERENCES encoder_experiment_projects(id),
    FOREIGN KEY (metric_source_protocol_id) REFERENCES encoder_experiment_protocols(id),
    FOREIGN KEY (training_snapshot_id) REFERENCES encoder_native_repair_training_snapshots(id),
    FOREIGN KEY (benchmark_generation_id) REFERENCES encoder_benchmark_generations(id)
);

CREATE TABLE encoder_production_optimization_runs (
    id TEXT PRIMARY KEY NOT NULL,
    definition_id TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    reserved_campaign_id TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    last_sequence INTEGER NOT NULL,
    last_event_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (definition_id) REFERENCES encoder_production_optimization_definitions(id)
);

CREATE TABLE encoder_production_optimization_events (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    previous_event_fingerprint TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES encoder_production_optimization_runs(id),
    UNIQUE (run_id, sequence)
);

CREATE TRIGGER encoder_production_optimization_definitions_no_update
BEFORE UPDATE ON encoder_production_optimization_definitions
BEGIN
    SELECT RAISE(ABORT, 'production optimization definitions are immutable');
END;

CREATE TRIGGER encoder_production_optimization_definitions_no_delete
BEFORE DELETE ON encoder_production_optimization_definitions
BEGIN
    SELECT RAISE(ABORT, 'production optimization definitions are append-only');
END;

CREATE TRIGGER encoder_production_optimization_events_no_update
BEFORE UPDATE ON encoder_production_optimization_events
BEGIN
    SELECT RAISE(ABORT, 'production optimization events are immutable');
END;

CREATE TRIGGER encoder_production_optimization_events_no_delete
BEFORE DELETE ON encoder_production_optimization_events
BEGIN
    SELECT RAISE(ABORT, 'production optimization events are append-only');
END;

CREATE INDEX idx_encoder_production_optimization_events_run
ON encoder_production_optimization_events(run_id, sequence);
