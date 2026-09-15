CREATE TABLE generation_job_authenticity (
    job_id TEXT PRIMARY KEY NOT NULL
        REFERENCES generation_jobs(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL
        REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    profile_id TEXT NOT NULL
        REFERENCES authenticity_profiles(id) ON DELETE RESTRICT,
    binding_id TEXT NOT NULL
        REFERENCES authenticity_profile_bindings(id) ON DELETE RESTRICT,
    context_fingerprint TEXT NOT NULL,
    assignment_fingerprint TEXT NOT NULL UNIQUE,
    assignment_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_generation_job_authenticity_profile
    ON generation_job_authenticity(profile_id, created_at, job_id);
