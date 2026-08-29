CREATE TABLE workflow_initial_allocations (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    generation_plan_id TEXT NOT NULL UNIQUE REFERENCES generation_plans(id) ON DELETE RESTRICT,
    requested_total_rows INTEGER NOT NULL,
    initial_target_rows INTEGER NOT NULL,
    reserved_rows INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX workflow_initial_allocations_dataset_created_idx
    ON workflow_initial_allocations(dataset_id, created_at DESC, id);
