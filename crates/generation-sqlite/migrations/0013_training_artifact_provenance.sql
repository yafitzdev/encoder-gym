ALTER TABLE training_runs ADD COLUMN base_model_id TEXT REFERENCES registered_encoders(id) ON DELETE RESTRICT;
ALTER TABLE training_runs ADD COLUMN parent_checkpoint_id TEXT REFERENCES training_checkpoints(id) ON DELETE RESTRICT;
ALTER TABLE training_runs ADD COLUMN backend_configuration_fingerprint TEXT;
ALTER TABLE training_checkpoints ADD COLUMN artifact_size_bytes INTEGER NOT NULL DEFAULT 0;

CREATE INDEX training_runs_base_model_idx ON training_runs(base_model_id, created_at);
CREATE INDEX training_runs_parent_checkpoint_idx ON training_runs(parent_checkpoint_id);
