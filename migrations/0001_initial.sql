PRAGMA foreign_keys = ON;

CREATE TABLE dataset_definitions (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    task_description TEXT NOT NULL,
    labels_json TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_plans (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE CASCADE,
    cells_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE CASCADE,
    plan_id TEXT NOT NULL REFERENCES generation_plans(id) ON DELETE CASCADE,
    backend_name TEXT NOT NULL,
    backend_model TEXT NOT NULL,
    state TEXT NOT NULL,
    requested_rows INTEGER NOT NULL,
    generated_rows INTEGER NOT NULL,
    accepted_rows INTEGER NOT NULL,
    rejected_rows INTEGER NOT NULL,
    failed_requests INTEGER NOT NULL,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE generated_rows (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE CASCADE,
    plan_id TEXT NOT NULL REFERENCES generation_plans(id) ON DELETE CASCADE,
    generation_job_id TEXT NOT NULL REFERENCES generation_jobs(id) ON DELETE CASCADE,
    cell_key TEXT NOT NULL,
    text TEXT NOT NULL,
    normalized_text TEXT NOT NULL,
    label TEXT NOT NULL,
    dimensions_json TEXT NOT NULL,
    generator_backend TEXT NOT NULL,
    generator_model TEXT NOT NULL,
    created_at TEXT NOT NULL,
    validation_status TEXT NOT NULL,
    validation_errors_json TEXT NOT NULL,
    generation_metadata_json TEXT NOT NULL
);

CREATE INDEX generated_rows_dataset_id_idx ON generated_rows(dataset_id);
CREATE INDEX generated_rows_job_id_idx ON generated_rows(generation_job_id);
CREATE INDEX generated_rows_plan_cell_status_idx
    ON generated_rows(plan_id, cell_key, validation_status);
CREATE INDEX generated_rows_normalized_text_idx
    ON generated_rows(dataset_id, normalized_text, validation_status);

CREATE TABLE backend_configurations (
    name TEXT PRIMARY KEY NOT NULL,
    base_url TEXT,
    model TEXT NOT NULL,
    parameters_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
