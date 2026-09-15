CREATE TABLE research_briefs (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL,
    dataset_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    brief_json TEXT NOT NULL,
    resolved_at TEXT NOT NULL,
    FOREIGN KEY (dataset_id) REFERENCES dataset_definitions(id) ON DELETE RESTRICT
);

CREATE INDEX idx_research_briefs_dataset
    ON research_briefs(dataset_id, resolved_at, id);

CREATE TABLE research_runs (
    id TEXT PRIMARY KEY NOT NULL,
    brief_id TEXT NOT NULL,
    brief_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'awaiting_review', 'failed', 'cancelled')),
    specification_fingerprint TEXT NOT NULL UNIQUE,
    run_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (brief_id) REFERENCES research_briefs(id) ON DELETE RESTRICT
);

CREATE INDEX idx_research_runs_brief_state
    ON research_runs(brief_id, state, created_at, id);

CREATE TABLE research_tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('started', 'succeeded', 'failed', 'interrupted')),
    call_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (run_id, sequence),
    FOREIGN KEY (run_id) REFERENCES research_runs(id) ON DELETE RESTRICT
);

CREATE INDEX idx_research_tool_calls_run
    ON research_tool_calls(run_id, sequence);

CREATE TABLE research_evidence (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    canonical_url TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_class TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    evidence_json TEXT NOT NULL,
    retrieved_at TEXT NOT NULL,
    UNIQUE (run_id, canonical_url, content_hash),
    FOREIGN KEY (run_id) REFERENCES research_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY (tool_call_id) REFERENCES research_tool_calls(id) ON DELETE RESTRICT
);

CREATE INDEX idx_research_evidence_run_source
    ON research_evidence(run_id, source_class, retrieved_at, id);

CREATE TABLE research_claims (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    claim_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES research_runs(id) ON DELETE RESTRICT
);

CREATE INDEX idx_research_claims_run
    ON research_claims(run_id, created_at, id);

CREATE TABLE authenticity_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    dataset_id TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    predecessor_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    profile_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (run_id) REFERENCES research_runs(id) ON DELETE RESTRICT,
    FOREIGN KEY (dataset_id) REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    FOREIGN KEY (predecessor_id) REFERENCES authenticity_profiles(id) ON DELETE RESTRICT
);

CREATE INDEX idx_authenticity_profiles_dataset
    ON authenticity_profiles(dataset_id, version, created_at, id);

CREATE TABLE authenticity_profile_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL,
    predecessor_id TEXT,
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'reject', 'request_revision')),
    fingerprint TEXT NOT NULL UNIQUE,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (profile_id) REFERENCES authenticity_profiles(id) ON DELETE RESTRICT,
    FOREIGN KEY (predecessor_id) REFERENCES authenticity_profile_reviews(id) ON DELETE RESTRICT
);

CREATE INDEX idx_authenticity_reviews_profile
    ON authenticity_profile_reviews(profile_id, created_at, id);

CREATE TABLE authenticity_profile_bindings (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    approval_id TEXT NOT NULL,
    predecessor_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    binding_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (dataset_id) REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    FOREIGN KEY (profile_id) REFERENCES authenticity_profiles(id) ON DELETE RESTRICT,
    FOREIGN KEY (approval_id) REFERENCES authenticity_profile_reviews(id) ON DELETE RESTRICT,
    FOREIGN KEY (predecessor_id) REFERENCES authenticity_profile_bindings(id) ON DELETE RESTRICT
);

CREATE INDEX idx_authenticity_bindings_dataset
    ON authenticity_profile_bindings(dataset_id, created_at, id);
