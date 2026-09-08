CREATE TABLE provider_catalog_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    previous_revision_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    catalog_json TEXT NOT NULL,
    UNIQUE (project_id, sequence),
    FOREIGN KEY (previous_revision_id) REFERENCES provider_catalog_revisions(id)
);

CREATE TABLE provider_catalog_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    project_id TEXT NOT NULL,
    active_revision_id TEXT NOT NULL,
    FOREIGN KEY (active_revision_id) REFERENCES provider_catalog_revisions(id)
);
