ALTER TABLE project_preparations
    ADD COLUMN benchmark_qualification_id TEXT
        REFERENCES workflow_benchmark_qualifications(id) ON DELETE RESTRICT;

ALTER TABLE project_preparations
    ADD COLUMN benchmark_qualification_review_id TEXT
        REFERENCES workflow_benchmark_qualification_reviews(id) ON DELETE RESTRICT;

CREATE INDEX idx_project_preparations_benchmark_qualification
    ON project_preparations(benchmark_qualification_id, id)
    WHERE benchmark_qualification_id IS NOT NULL;

CREATE TRIGGER project_preparations_qualification_insert_guard
BEFORE INSERT ON project_preparations
WHEN NEW.benchmark_qualification_id IS NOT NULL
  OR NEW.benchmark_qualification_review_id IS NOT NULL
BEGIN
    SELECT CASE
        WHEN NEW.benchmark_qualification_id IS NULL
          OR NEW.benchmark_qualification_review_id IS NULL
        THEN RAISE(ABORT, 'preparation qualification binding must be complete')
    END;
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM workflow_definitions d
        WHERE d.id = NEW.workflow_definition_id
          AND d.benchmark_bundle_id = NEW.benchmark_bundle_id
          AND d.benchmark_qualification_id = NEW.benchmark_qualification_id
          AND d.benchmark_qualification_review_id = NEW.benchmark_qualification_review_id
    ) THEN RAISE(ABORT, 'preparation qualification authority differs from workflow definition') END;
END;

CREATE TRIGGER project_preparations_qualification_update_guard
BEFORE UPDATE OF benchmark_bundle_id, workflow_definition_id,
                 benchmark_qualification_id, benchmark_qualification_review_id
ON project_preparations
BEGIN
    SELECT CASE
        WHEN OLD.benchmark_qualification_id IS NOT NULL
          AND (
              NEW.benchmark_qualification_id IS NOT OLD.benchmark_qualification_id
              OR NEW.benchmark_qualification_review_id
                 IS NOT OLD.benchmark_qualification_review_id
          )
        THEN RAISE(ABORT, 'preparation qualification authority is immutable once bound')
    END;
    SELECT CASE
        WHEN (NEW.benchmark_qualification_id IS NULL)
           <> (NEW.benchmark_qualification_review_id IS NULL)
        THEN RAISE(ABORT, 'preparation qualification binding must be complete')
    END;
    SELECT CASE
        WHEN NEW.benchmark_qualification_id IS NOT NULL AND NOT EXISTS (
            SELECT 1
            FROM workflow_definitions d
            WHERE d.id = NEW.workflow_definition_id
              AND d.benchmark_bundle_id = NEW.benchmark_bundle_id
              AND d.benchmark_qualification_id = NEW.benchmark_qualification_id
              AND d.benchmark_qualification_review_id = NEW.benchmark_qualification_review_id
        )
        THEN RAISE(ABORT, 'preparation qualification authority differs from workflow definition')
    END;
END;
