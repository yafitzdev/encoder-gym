CREATE TABLE workflow_benchmark_bundles (
    id TEXT PRIMARY KEY NOT NULL,
    development_suite_id TEXT NOT NULL
        REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    sealed_suite_id TEXT
        REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    contamination_report_id TEXT NOT NULL
        REFERENCES workflow_contamination_reports(id) ON DELETE RESTRICT,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    CHECK (sealed_suite_id IS NULL OR sealed_suite_id <> development_suite_id)
);

ALTER TABLE workflow_definitions
    ADD COLUMN benchmark_bundle_id TEXT
        REFERENCES workflow_benchmark_bundles(id) ON DELETE RESTRICT;

ALTER TABLE project_preparations
    ADD COLUMN benchmark_bundle_id TEXT
        REFERENCES workflow_benchmark_bundles(id) ON DELETE RESTRICT;

CREATE INDEX idx_workflow_benchmark_bundles_development_created
    ON workflow_benchmark_bundles(development_suite_id, created_at, id);
CREATE INDEX idx_workflow_benchmark_bundles_sealed_created
    ON workflow_benchmark_bundles(sealed_suite_id, created_at, id)
    WHERE sealed_suite_id IS NOT NULL;
CREATE INDEX idx_workflow_benchmark_bundles_contamination_created
    ON workflow_benchmark_bundles(contamination_report_id, created_at, id);
CREATE INDEX idx_workflow_definitions_benchmark_bundle
    ON workflow_definitions(benchmark_bundle_id, id)
    WHERE benchmark_bundle_id IS NOT NULL;
CREATE INDEX idx_project_preparations_benchmark_bundle
    ON project_preparations(benchmark_bundle_id, id)
    WHERE benchmark_bundle_id IS NOT NULL;
