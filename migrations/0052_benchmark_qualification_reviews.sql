CREATE TABLE workflow_benchmark_qualification_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    qualification_id TEXT NOT NULL UNIQUE
        REFERENCES workflow_benchmark_qualifications(id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'reject')),
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_workflow_benchmark_qualification_reviews_decision_created
    ON workflow_benchmark_qualification_reviews(decision, created_at, id);

CREATE TRIGGER workflow_benchmark_qualification_reviews_no_update
BEFORE UPDATE ON workflow_benchmark_qualification_reviews
BEGIN
    SELECT RAISE(ABORT, 'benchmark qualification reviews are immutable');
END;

CREATE TRIGGER workflow_benchmark_qualification_reviews_no_delete
BEFORE DELETE ON workflow_benchmark_qualification_reviews
BEGIN
    SELECT RAISE(ABORT, 'benchmark qualification reviews are immutable');
END;
