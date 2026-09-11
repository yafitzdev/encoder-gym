CREATE TABLE optimization_launch_authorizations (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    setup_id TEXT NOT NULL REFERENCES optimization_setups(id),
    provider_catalog_id TEXT NOT NULL REFERENCES provider_catalog_revisions(id),
    scope_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL
);
CREATE INDEX optimization_launches_setup ON optimization_launch_authorizations(setup_id, created_at);
CREATE TRIGGER immutable_optimization_launches_update BEFORE UPDATE ON optimization_launch_authorizations
BEGIN SELECT RAISE(ABORT, 'Optimization launch authorizations are immutable'); END;
CREATE TRIGGER immutable_optimization_launches_delete BEFORE DELETE ON optimization_launch_authorizations
BEGIN SELECT RAISE(ABORT, 'Optimization launch authorizations are immutable'); END;
