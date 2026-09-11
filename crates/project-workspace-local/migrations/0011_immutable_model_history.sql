CREATE TRIGGER immutable_model_artifacts_update BEFORE UPDATE ON model_artifacts
BEGIN SELECT RAISE(ABORT, 'Model artifacts are immutable'); END;
CREATE TRIGGER immutable_model_artifacts_delete BEFORE DELETE ON model_artifacts
BEGIN SELECT RAISE(ABORT, 'Model artifacts are immutable'); END;
CREATE TRIGGER immutable_baseline_revisions_update BEFORE UPDATE ON baseline_revisions
BEGIN SELECT RAISE(ABORT, 'Baseline revisions are immutable'); END;
CREATE TRIGGER immutable_baseline_revisions_delete BEFORE DELETE ON baseline_revisions
BEGIN SELECT RAISE(ABORT, 'Baseline revisions are immutable'); END;
