CREATE TABLE IF NOT EXISTS optimization_repair_not_executed (
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    reason TEXT NOT NULL CHECK(reason = 'canary_rejected'),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(run_id, iteration)
);
CREATE TRIGGER IF NOT EXISTS immutable_optimization_repair_not_executed_update
BEFORE UPDATE ON optimization_repair_not_executed
BEGIN SELECT RAISE(ABORT, 'Repair execution stop receipts are immutable'); END;
CREATE TRIGGER IF NOT EXISTS immutable_optimization_repair_not_executed_delete
BEFORE DELETE ON optimization_repair_not_executed
BEGIN SELECT RAISE(ABORT, 'Repair execution stop receipts are immutable'); END;
