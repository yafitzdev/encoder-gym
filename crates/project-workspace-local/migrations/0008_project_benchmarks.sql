CREATE TABLE project_benchmark_versions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL,
    number INTEGER NOT NULL CHECK(number > 0),
    parent_id TEXT REFERENCES project_benchmark_versions(id),
    definition_fingerprint TEXT NOT NULL,
    source_binding_id TEXT NOT NULL REFERENCES scientific_bindings(id),
    fingerprint TEXT NOT NULL UNIQUE,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(project_id, number),
    UNIQUE(project_id, definition_fingerprint)
);
CREATE TRIGGER immutable_project_benchmarks_update BEFORE UPDATE ON project_benchmark_versions
BEGIN SELECT RAISE(ABORT, 'Benchmark versions are immutable'); END;
CREATE TRIGGER immutable_project_benchmarks_delete BEFORE DELETE ON project_benchmark_versions
BEGIN SELECT RAISE(ABORT, 'Benchmark versions are immutable'); END;
