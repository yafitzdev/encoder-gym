CREATE TABLE optimization_setups (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    number INTEGER NOT NULL CHECK(number > 0),
    parent_id TEXT REFERENCES optimization_setups(id),
    baseline_revision_id TEXT NOT NULL REFERENCES baseline_revisions(id),
    model_id TEXT NOT NULL REFERENCES model_artifacts(id),
    dataset_version_id TEXT NOT NULL REFERENCES dataset_versions(id),
    benchmark_version_id TEXT NOT NULL REFERENCES project_benchmark_versions(id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(project_id, number)
);
CREATE TRIGGER immutable_optimization_setups_update BEFORE UPDATE ON optimization_setups
BEGIN SELECT RAISE(ABORT, 'Optimization setups are immutable'); END;
CREATE TRIGGER immutable_optimization_setups_delete BEFORE DELETE ON optimization_setups
BEGIN SELECT RAISE(ABORT, 'Optimization setups are immutable'); END;
