ALTER TABLE training_runs ADD COLUMN input_protocol TEXT;
ALTER TABLE training_runs ADD COLUMN input_population_fingerprint TEXT;
ALTER TABLE training_runs ADD COLUMN input_member_count INTEGER;
ALTER TABLE training_runs ADD COLUMN input_fingerprint TEXT;
ALTER TABLE training_runs ADD COLUMN input_authority_kind TEXT;
ALTER TABLE training_runs ADD COLUMN input_authority_id TEXT;
ALTER TABLE training_runs ADD COLUMN input_authority_fingerprint TEXT;

CREATE TRIGGER training_runs_input_binding_insert_guard
BEFORE INSERT ON training_runs
WHEN (
    (NEW.input_protocol IS NULL) +
    (NEW.input_population_fingerprint IS NULL) +
    (NEW.input_member_count IS NULL) +
    (NEW.input_fingerprint IS NULL) +
    (NEW.input_authority_kind IS NULL) +
    (NEW.input_authority_id IS NULL) +
    (NEW.input_authority_fingerprint IS NULL)
) NOT IN (0, 7)
BEGIN
    SELECT RAISE(ABORT, 'training input authority binding must be complete');
END;

CREATE TRIGGER training_runs_input_binding_update_guard
BEFORE UPDATE OF input_protocol, input_population_fingerprint, input_member_count,
    input_fingerprint,
    input_authority_kind, input_authority_id, input_authority_fingerprint ON training_runs
WHEN
    OLD.input_protocol IS NOT NEW.input_protocol OR
    OLD.input_population_fingerprint IS NOT NEW.input_population_fingerprint OR
    OLD.input_member_count IS NOT NEW.input_member_count OR
    OLD.input_fingerprint IS NOT NEW.input_fingerprint OR
    OLD.input_authority_kind IS NOT NEW.input_authority_kind OR
    OLD.input_authority_id IS NOT NEW.input_authority_id OR
    OLD.input_authority_fingerprint IS NOT NEW.input_authority_fingerprint
BEGIN
    SELECT RAISE(ABORT, 'training input authority binding is immutable');
END;

CREATE INDEX training_runs_input_authority_idx
    ON training_runs(input_authority_kind, input_authority_id, created_at)
    WHERE input_authority_id IS NOT NULL;
