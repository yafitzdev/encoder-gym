CREATE TABLE model_artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    content_fingerprint TEXT NOT NULL,
    origin TEXT NOT NULL,
    metadata_json TEXT NOT NULL
);

CREATE TABLE baseline_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    model_artifact_id TEXT NOT NULL,
    previous_revision_id TEXT,
    change_kind TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL,
    UNIQUE (project_id, sequence),
    FOREIGN KEY (model_artifact_id) REFERENCES model_artifacts(id),
    FOREIGN KEY (previous_revision_id) REFERENCES baseline_revisions(id)
);

CREATE TABLE model_catalog_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    project_id TEXT NOT NULL,
    active_baseline_revision_id TEXT NOT NULL,
    FOREIGN KEY (active_baseline_revision_id) REFERENCES baseline_revisions(id)
);
