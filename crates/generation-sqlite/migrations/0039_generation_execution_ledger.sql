CREATE TABLE generation_execution_specs (
    job_id TEXT PRIMARY KEY NOT NULL
        REFERENCES generation_jobs(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL REFERENCES generation_plans(id) ON DELETE RESTRICT,
    backend_name TEXT NOT NULL,
    backend_model TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_generation_execution_specs_plan
    ON generation_execution_specs(plan_id, created_at);

CREATE TABLE generation_request_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL REFERENCES generation_jobs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    cell_key TEXT NOT NULL,
    retry_index INTEGER NOT NULL CHECK (retry_index >= 0),
    requested_count INTEGER NOT NULL CHECK (requested_count > 0),
    state TEXT NOT NULL CHECK (state IN ('started', 'succeeded', 'failed', 'interrupted')),
    request_fingerprint TEXT NOT NULL,
    outcome_fingerprint TEXT,
    artifact_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE(job_id, sequence)
);

CREATE INDEX idx_generation_request_attempts_job_state
    ON generation_request_attempts(job_id, state, sequence);

-- Preserve any historical duplicates while preventing every new accepted source
-- row from claiming normalized text that already belongs to the dataset.
CREATE TABLE dataset_normalized_text_claims (
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    normalized_text TEXT NOT NULL,
    PRIMARY KEY (dataset_id, normalized_text)
);

INSERT INTO dataset_normalized_text_claims (dataset_id, normalized_text)
SELECT DISTINCT dataset_id, normalized_text
FROM dataset_source_rows;

CREATE TRIGGER claim_dataset_normalized_text_before_insert
BEFORE INSERT ON dataset_source_rows
BEGIN
    INSERT INTO dataset_normalized_text_claims (dataset_id, normalized_text)
    VALUES (NEW.dataset_id, NEW.normalized_text);
END;
