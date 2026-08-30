CREATE TABLE project_bootstraps (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    bootstrap_fingerprint TEXT NOT NULL UNIQUE,
    preparation_id TEXT NOT NULL UNIQUE
        REFERENCES project_preparations(id) ON DELETE RESTRICT,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_project_bootstraps_created
    ON project_bootstraps(created_at DESC, id);
