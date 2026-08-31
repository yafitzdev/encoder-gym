ALTER TABLE model_promotions
    ADD COLUMN training_benchmark_check_id TEXT
        REFERENCES workflow_training_benchmark_checks(id) ON DELETE RESTRICT;

ALTER TABLE model_promotions
    ADD COLUMN training_benchmark_check_fingerprint TEXT;

CREATE INDEX idx_model_promotions_training_benchmark_check
    ON model_promotions(training_benchmark_check_id, id)
    WHERE training_benchmark_check_id IS NOT NULL;

CREATE TRIGGER model_promotions_training_benchmark_insert_guard
BEFORE INSERT ON model_promotions
WHEN NEW.training_benchmark_check_id IS NULL
    OR NEW.training_benchmark_check_fingerprint IS NULL
BEGIN
    SELECT RAISE(ABORT, 'new promotion requires complete training-benchmark binding');
END;

CREATE TRIGGER model_promotions_training_benchmark_update_guard
BEFORE UPDATE OF training_benchmark_check_id,
    training_benchmark_check_fingerprint ON model_promotions
WHEN OLD.training_benchmark_check_id IS NOT NEW.training_benchmark_check_id
    OR OLD.training_benchmark_check_fingerprint
        IS NOT NEW.training_benchmark_check_fingerprint
BEGIN
    SELECT RAISE(ABORT, 'promotion training-benchmark binding is immutable');
END;
