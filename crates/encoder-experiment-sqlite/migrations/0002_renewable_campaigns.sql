CREATE TABLE encoder_benchmark_generations (
    id TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    last_sequence INTEGER NOT NULL,
    last_event_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE encoder_benchmark_generation_events (
    id TEXT PRIMARY KEY NOT NULL,
    generation_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    previous_event_fingerprint TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (generation_id) REFERENCES encoder_benchmark_generations(id),
    UNIQUE (generation_id, sequence)
);

CREATE TABLE encoder_production_campaigns (
    id TEXT PRIMARY KEY NOT NULL,
    project_snapshot_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    last_sequence INTEGER NOT NULL,
    last_event_fingerprint TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (project_snapshot_id) REFERENCES encoder_experiment_projects(id)
);

CREATE TABLE encoder_production_campaign_events (
    id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    previous_event_fingerprint TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES encoder_production_campaigns(id),
    UNIQUE (campaign_id, sequence)
);

CREATE INDEX idx_encoder_benchmark_generation_events
    ON encoder_benchmark_generation_events(generation_id, sequence);
CREATE INDEX idx_encoder_production_campaigns_project
    ON encoder_production_campaigns(project_snapshot_id, created_at, id);
CREATE INDEX idx_encoder_production_campaign_events
    ON encoder_production_campaign_events(campaign_id, sequence);

CREATE TRIGGER encoder_benchmark_generations_immutable_identity
BEFORE UPDATE ON encoder_benchmark_generations
WHEN NEW.id != OLD.id
  OR NEW.fingerprint != OLD.fingerprint
  OR NEW.artifact_json != OLD.artifact_json
  OR NEW.created_at != OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'benchmark generation identity is immutable');
END;

CREATE TRIGGER encoder_benchmark_generations_no_delete
BEFORE DELETE ON encoder_benchmark_generations
BEGIN
    SELECT RAISE(ABORT, 'benchmark generations are append-only');
END;

CREATE TRIGGER encoder_benchmark_generation_events_no_update
BEFORE UPDATE ON encoder_benchmark_generation_events
BEGIN
    SELECT RAISE(ABORT, 'benchmark generation events are immutable');
END;

CREATE TRIGGER encoder_benchmark_generation_events_no_delete
BEFORE DELETE ON encoder_benchmark_generation_events
BEGIN
    SELECT RAISE(ABORT, 'benchmark generation events are append-only');
END;

CREATE TRIGGER encoder_production_campaigns_immutable_identity
BEFORE UPDATE ON encoder_production_campaigns
WHEN NEW.id != OLD.id
  OR NEW.project_snapshot_id != OLD.project_snapshot_id
  OR NEW.fingerprint != OLD.fingerprint
  OR NEW.artifact_json != OLD.artifact_json
  OR NEW.created_at != OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'production campaign identity is immutable');
END;

CREATE TRIGGER encoder_production_campaigns_no_delete
BEFORE DELETE ON encoder_production_campaigns
BEGIN
    SELECT RAISE(ABORT, 'production campaigns are append-only');
END;

CREATE TRIGGER encoder_production_campaign_events_no_update
BEFORE UPDATE ON encoder_production_campaign_events
BEGIN
    SELECT RAISE(ABORT, 'production campaign events are immutable');
END;

CREATE TRIGGER encoder_production_campaign_events_no_delete
BEFORE DELETE ON encoder_production_campaign_events
BEGIN
    SELECT RAISE(ABORT, 'production campaign events are append-only');
END;
