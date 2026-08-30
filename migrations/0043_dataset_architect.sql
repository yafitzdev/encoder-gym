CREATE TABLE dataset_architect_briefs (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    dataset_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    brief_json TEXT NOT NULL,
    resolved_at TEXT NOT NULL
);

CREATE TABLE dataset_architect_runs (
    id TEXT PRIMARY KEY NOT NULL,
    brief_id TEXT NOT NULL REFERENCES dataset_architect_briefs(id) ON DELETE RESTRICT,
    brief_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'awaiting_review', 'failed', 'cancelled')),
    specification_fingerprint TEXT NOT NULL UNIQUE,
    run_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE dataset_architect_tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES dataset_architect_runs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('started', 'succeeded', 'failed', 'interrupted')),
    call_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE(run_id, sequence)
);

CREATE TABLE dataset_architecture_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES dataset_architect_runs(id) ON DELETE RESTRICT,
    brief_id TEXT NOT NULL REFERENCES dataset_architect_briefs(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    coverage_fingerprint TEXT NOT NULL,
    proposal_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE dataset_architecture_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES dataset_architecture_proposals(id) ON DELETE RESTRICT,
    predecessor_id TEXT REFERENCES dataset_architecture_reviews(id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'reject', 'request_revision')),
    fingerprint TEXT NOT NULL UNIQUE,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_strategy_contexts (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL UNIQUE REFERENCES generation_plans(id) ON DELETE RESTRICT,
    proposal_id TEXT NOT NULL REFERENCES dataset_architecture_proposals(id) ON DELETE RESTRICT,
    approval_id TEXT NOT NULL REFERENCES dataset_architecture_reviews(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    context_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE dataset_architecture_applications (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL UNIQUE REFERENCES dataset_architecture_proposals(id) ON DELETE RESTRICT,
    approval_id TEXT NOT NULL REFERENCES dataset_architecture_reviews(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL UNIQUE REFERENCES generation_plans(id) ON DELETE RESTRICT,
    strategy_context_id TEXT NOT NULL UNIQUE REFERENCES generation_strategy_contexts(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    application_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_job_strategies (
    job_id TEXT PRIMARY KEY NOT NULL REFERENCES generation_jobs(id) ON DELETE RESTRICT,
    context_id TEXT NOT NULL REFERENCES generation_strategy_contexts(id) ON DELETE RESTRICT,
    context_fingerprint TEXT NOT NULL,
    assignment_fingerprint TEXT NOT NULL UNIQUE,
    assignment_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_dataset_architect_runs_brief ON dataset_architect_runs(brief_id, created_at);
CREATE INDEX idx_dataset_architecture_proposals_run ON dataset_architecture_proposals(run_id, created_at);
CREATE INDEX idx_dataset_architecture_reviews_proposal ON dataset_architecture_reviews(proposal_id, created_at);
