CREATE TABLE encoder_repair_observation_sets (
    id TEXT PRIMARY KEY NOT NULL,
    evidence_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    project_snapshot_id TEXT NOT NULL,
    source_campaign_id TEXT NOT NULL,
    source_experiment_run_id TEXT NOT NULL,
    candidate_id TEXT,
    evaluation_report_id TEXT NOT NULL,
    suite_key TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (project_snapshot_id) REFERENCES encoder_experiment_projects(id),
    FOREIGN KEY (source_campaign_id) REFERENCES encoder_production_campaigns(id),
    FOREIGN KEY (source_experiment_run_id) REFERENCES encoder_experiment_runs(id)
);

CREATE TABLE encoder_repair_diagnoses (
    id TEXT PRIMARY KEY NOT NULL,
    derivation_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    project_snapshot_id TEXT NOT NULL,
    source_campaign_id TEXT NOT NULL,
    source_experiment_run_id TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (project_snapshot_id) REFERENCES encoder_experiment_projects(id),
    FOREIGN KEY (source_campaign_id) REFERENCES encoder_production_campaigns(id),
    FOREIGN KEY (source_experiment_run_id) REFERENCES encoder_experiment_runs(id)
);

CREATE TABLE encoder_repair_diagnosis_observation_sets (
    diagnosis_id TEXT NOT NULL,
    observation_set_id TEXT NOT NULL,
    observation_set_fingerprint TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (diagnosis_id, observation_set_id),
    UNIQUE (diagnosis_id, ordinal),
    FOREIGN KEY (diagnosis_id) REFERENCES encoder_repair_diagnoses(id),
    FOREIGN KEY (observation_set_id) REFERENCES encoder_repair_observation_sets(id)
);

CREATE INDEX idx_encoder_repair_observations_campaign
    ON encoder_repair_observation_sets(source_campaign_id, created_at, id);
CREATE INDEX idx_encoder_repair_observations_run
    ON encoder_repair_observation_sets(source_experiment_run_id, suite_key, candidate_id);
CREATE INDEX idx_encoder_repair_diagnoses_campaign
    ON encoder_repair_diagnoses(source_campaign_id, created_at, id);

CREATE TRIGGER encoder_repair_observation_sets_no_update
BEFORE UPDATE ON encoder_repair_observation_sets
BEGIN
    SELECT RAISE(ABORT, 'repair observation sets are immutable');
END;

CREATE TRIGGER encoder_repair_observation_sets_no_delete
BEFORE DELETE ON encoder_repair_observation_sets
BEGIN
    SELECT RAISE(ABORT, 'repair observation sets are append-only');
END;

CREATE TRIGGER encoder_repair_diagnoses_no_update
BEFORE UPDATE ON encoder_repair_diagnoses
BEGIN
    SELECT RAISE(ABORT, 'repair diagnoses are immutable');
END;

CREATE TRIGGER encoder_repair_diagnoses_no_delete
BEFORE DELETE ON encoder_repair_diagnoses
BEGIN
    SELECT RAISE(ABORT, 'repair diagnoses are append-only');
END;

CREATE TRIGGER encoder_repair_diagnosis_observation_sets_no_update
BEFORE UPDATE ON encoder_repair_diagnosis_observation_sets
BEGIN
    SELECT RAISE(ABORT, 'repair diagnosis bindings are immutable');
END;

CREATE TRIGGER encoder_repair_diagnosis_observation_sets_no_delete
BEFORE DELETE ON encoder_repair_diagnosis_observation_sets
BEGIN
    SELECT RAISE(ABORT, 'repair diagnosis bindings are append-only');
END;
