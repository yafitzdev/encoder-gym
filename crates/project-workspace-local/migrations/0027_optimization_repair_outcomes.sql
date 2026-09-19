CREATE TABLE IF NOT EXISTS optimization_repair_outcomes (
    run_id TEXT NOT NULL REFERENCES project_optimization_runs(id),
    iteration INTEGER NOT NULL CHECK(iteration BETWEEN 1 AND 10),
    target_id TEXT NOT NULL,
    output_evidence_fingerprint TEXT NOT NULL,
    intervention_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(run_id, iteration, target_id)
);
CREATE TRIGGER IF NOT EXISTS immutable_optimization_repair_outcomes_update
BEFORE UPDATE ON optimization_repair_outcomes
BEGIN SELECT RAISE(ABORT, 'Repair outcomes are immutable'); END;
CREATE TRIGGER IF NOT EXISTS immutable_optimization_repair_outcomes_delete
BEFORE DELETE ON optimization_repair_outcomes
BEGIN SELECT RAISE(ABORT, 'Repair outcomes are immutable'); END;
