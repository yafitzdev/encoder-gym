CREATE TABLE dataset_branches (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    origin_version_id TEXT REFERENCES dataset_versions(id),
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json))
);
CREATE UNIQUE INDEX dataset_base_per_project ON dataset_branches(project_id)
    WHERE origin_version_id IS NULL;

CREATE TABLE dataset_versions (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_branches(id),
    number INTEGER NOT NULL CHECK(number > 0),
    parent_version_id TEXT REFERENCES dataset_versions(id),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(dataset_id, number)
);

CREATE TRIGGER immutable_dataset_branches_update BEFORE UPDATE ON dataset_branches
BEGIN SELECT RAISE(ABORT, 'Dataset identities are immutable'); END;
CREATE TRIGGER immutable_dataset_branches_delete BEFORE DELETE ON dataset_branches
BEGIN SELECT RAISE(ABORT, 'Dataset identities are immutable'); END;
CREATE TRIGGER immutable_dataset_versions_update BEFORE UPDATE ON dataset_versions
BEGIN SELECT RAISE(ABORT, 'Dataset versions are immutable'); END;
CREATE TRIGGER immutable_dataset_versions_delete BEFORE DELETE ON dataset_versions
BEGIN SELECT RAISE(ABORT, 'Dataset versions are immutable'); END;
