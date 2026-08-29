ALTER TABLE training_runs ADD COLUMN current_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE training_runs ADD COLUMN completed_batches INTEGER NOT NULL DEFAULT 0;
ALTER TABLE training_runs ADD COLUMN batches_in_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE training_runs ADD COLUMN processed_examples INTEGER NOT NULL DEFAULT 0;
ALTER TABLE training_runs ADD COLUMN latest_learning_rate REAL;
ALTER TABLE training_runs ADD COLUMN elapsed_milliseconds INTEGER NOT NULL DEFAULT 0;
