CREATE TABLE model_dataset_links (
    model_id TEXT PRIMARY KEY NOT NULL REFERENCES model_artifacts(id),
    version_id TEXT NOT NULL REFERENCES dataset_versions(id),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE TRIGGER immutable_model_dataset_links_update BEFORE UPDATE ON model_dataset_links
BEGIN SELECT RAISE(ABORT, 'Model training data is immutable'); END;
CREATE TRIGGER immutable_model_dataset_links_delete BEFORE DELETE ON model_dataset_links
BEGIN SELECT RAISE(ABORT, 'Model training data is immutable'); END;
