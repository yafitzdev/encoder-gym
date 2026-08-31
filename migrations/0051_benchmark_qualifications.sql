CREATE TABLE workflow_benchmark_qualifications (
    id TEXT PRIMARY KEY NOT NULL,
    benchmark_bundle_id TEXT NOT NULL
        REFERENCES workflow_benchmark_bundles(id) ON DELETE RESTRICT,
    protocol TEXT NOT NULL,
    readiness TEXT NOT NULL CHECK (readiness IN ('ready', 'blocked')),
    policy_fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    UNIQUE (benchmark_bundle_id, protocol, policy_fingerprint)
);

CREATE INDEX idx_workflow_benchmark_qualifications_bundle_created
    ON workflow_benchmark_qualifications(benchmark_bundle_id, created_at, id);
CREATE INDEX idx_workflow_benchmark_qualifications_readiness_created
    ON workflow_benchmark_qualifications(readiness, created_at, id);

CREATE TRIGGER workflow_benchmark_qualifications_no_update
BEFORE UPDATE ON workflow_benchmark_qualifications
BEGIN
    SELECT RAISE(ABORT, 'benchmark qualifications are immutable');
END;

CREATE TRIGGER workflow_benchmark_qualifications_no_delete
BEFORE DELETE ON workflow_benchmark_qualifications
BEGIN
    SELECT RAISE(ABORT, 'benchmark qualifications are immutable');
END;
