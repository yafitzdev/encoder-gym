CREATE TABLE optimization_repair_not_executed_v2 (
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    reason TEXT NOT NULL CHECK(reason IN ('canary_rejected', 'zero_surviving_edits')),
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(run_id, iteration)
);

INSERT INTO optimization_repair_not_executed_v2
    (run_id, iteration, reason, fingerprint, metadata_json, created_at)
SELECT run_id, iteration, reason, fingerprint, metadata_json, created_at
FROM optimization_repair_not_executed;

DROP TABLE optimization_repair_not_executed;
ALTER TABLE optimization_repair_not_executed_v2 RENAME TO optimization_repair_not_executed;

CREATE TRIGGER immutable_optimization_repair_not_executed_update
BEFORE UPDATE ON optimization_repair_not_executed
BEGIN SELECT RAISE(ABORT, 'Repair execution stop receipts are immutable'); END;

CREATE TRIGGER immutable_optimization_repair_not_executed_delete
BEFORE DELETE ON optimization_repair_not_executed
BEGIN SELECT RAISE(ABORT, 'Repair execution stop receipts are immutable'); END;
