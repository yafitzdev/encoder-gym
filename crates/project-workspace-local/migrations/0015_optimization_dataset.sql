CREATE TABLE optimization_dataset_publications (
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL,
    version_id TEXT NOT NULL REFERENCES dataset_versions(id),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    PRIMARY KEY(run_id, iteration),
    FOREIGN KEY(run_id, iteration) REFERENCES optimization_agent_scopes(run_id, iteration)
);
CREATE TRIGGER optimization_dataset_publications_no_update BEFORE UPDATE ON optimization_dataset_publications BEGIN SELECT RAISE(ABORT, 'optimization dataset publications are immutable'); END;
CREATE TRIGGER optimization_dataset_publications_no_delete BEFORE DELETE ON optimization_dataset_publications BEGIN SELECT RAISE(ABORT, 'optimization dataset publications are immutable'); END;
