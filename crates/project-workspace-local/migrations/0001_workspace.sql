CREATE TABLE workspace_identity (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    project_id TEXT NOT NULL,
    manifest_sha256 TEXT NOT NULL
);
CREATE TABLE dataset_imports (
    id TEXT PRIMARY KEY NOT NULL,
    content_sha256 TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL
);
