ALTER TABLE workflow_definitions
    ADD COLUMN benchmark_qualification_id TEXT
        REFERENCES workflow_benchmark_qualifications(id) ON DELETE RESTRICT;

ALTER TABLE workflow_definitions
    ADD COLUMN benchmark_qualification_review_id TEXT
        REFERENCES workflow_benchmark_qualification_reviews(id) ON DELETE RESTRICT;

CREATE INDEX idx_workflow_definitions_benchmark_qualification
    ON workflow_definitions(benchmark_qualification_id, id)
    WHERE benchmark_qualification_id IS NOT NULL;

CREATE INDEX idx_workflow_definitions_benchmark_qualification_review
    ON workflow_definitions(benchmark_qualification_review_id, id)
    WHERE benchmark_qualification_review_id IS NOT NULL;

CREATE TRIGGER workflow_definitions_qualification_insert_guard
BEFORE INSERT ON workflow_definitions
WHEN NEW.benchmark_qualification_id IS NOT NULL
  OR NEW.benchmark_qualification_review_id IS NOT NULL
BEGIN
    SELECT CASE
        WHEN NEW.benchmark_qualification_id IS NULL
          OR NEW.benchmark_qualification_review_id IS NULL
        THEN RAISE(ABORT, 'workflow qualification binding must be complete')
    END;
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM workflow_benchmark_qualifications q
        JOIN workflow_benchmark_qualification_reviews r
          ON r.qualification_id = q.id
        WHERE q.id = NEW.benchmark_qualification_id
          AND r.id = NEW.benchmark_qualification_review_id
          AND q.benchmark_bundle_id = NEW.benchmark_bundle_id
          AND q.readiness = 'ready'
          AND r.decision = 'approve'
    ) THEN RAISE(ABORT, 'workflow qualification authority is not ready and approved') END;
END;

CREATE TRIGGER workflow_definitions_qualification_update_guard
BEFORE UPDATE OF benchmark_bundle_id, benchmark_qualification_id,
                 benchmark_qualification_review_id
ON workflow_definitions
BEGIN
    SELECT CASE
        WHEN OLD.benchmark_qualification_id IS NOT NULL
          AND (
              NEW.benchmark_qualification_id IS NOT OLD.benchmark_qualification_id
              OR NEW.benchmark_qualification_review_id
                 IS NOT OLD.benchmark_qualification_review_id
          )
        THEN RAISE(ABORT, 'workflow qualification authority is immutable once bound')
    END;
    SELECT CASE
        WHEN (NEW.benchmark_qualification_id IS NULL)
           <> (NEW.benchmark_qualification_review_id IS NULL)
        THEN RAISE(ABORT, 'workflow qualification binding must be complete')
    END;
    SELECT CASE
        WHEN NEW.benchmark_qualification_id IS NOT NULL AND NOT EXISTS (
            SELECT 1
            FROM workflow_benchmark_qualifications q
            JOIN workflow_benchmark_qualification_reviews r
              ON r.qualification_id = q.id
            WHERE q.id = NEW.benchmark_qualification_id
              AND r.id = NEW.benchmark_qualification_review_id
              AND q.benchmark_bundle_id = NEW.benchmark_bundle_id
              AND q.readiness = 'ready'
              AND r.decision = 'approve'
        )
        THEN RAISE(ABORT, 'workflow qualification authority is not ready and approved')
    END;
END;
