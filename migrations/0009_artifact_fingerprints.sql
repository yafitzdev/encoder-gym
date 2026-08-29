ALTER TABLE dataset_snapshots ADD COLUMN fingerprint TEXT;
ALTER TABLE evaluation_runs ADD COLUMN input_fingerprint TEXT;
ALTER TABLE analysis_reports ADD COLUMN fingerprint TEXT;
ALTER TABLE optimization_proposals ADD COLUMN fingerprint TEXT;

CREATE TABLE project_configurations (
    id TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL,
    dataset_id TEXT NOT NULL UNIQUE REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    generation_plan_id TEXT NOT NULL UNIQUE REFERENCES generation_plans(id) ON DELETE RESTRICT,
    resolved_toml_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX project_configurations_fingerprint_idx
    ON project_configurations(fingerprint, dataset_id);
