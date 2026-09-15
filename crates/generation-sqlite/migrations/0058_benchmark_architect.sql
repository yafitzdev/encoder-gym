CREATE TABLE benchmark_architect_briefs (
    id TEXT PRIMARY KEY NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    fingerprint TEXT NOT NULL UNIQUE,
    brief_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE benchmark_architect_runs (
    id TEXT PRIMARY KEY NOT NULL,
    brief_id TEXT NOT NULL REFERENCES benchmark_architect_briefs(id) ON DELETE RESTRICT,
    brief_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'awaiting_review', 'failed', 'cancelled')),
    specification_fingerprint TEXT NOT NULL UNIQUE,
    run_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE benchmark_architect_tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES benchmark_architect_runs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    kind TEXT NOT NULL CHECK (kind IN (
        'inspect_brief', 'inspect_existing_benchmark', 'inspect_exposure_history',
        'search_web', 'fetch_page', 'record_evidence', 'inspect_evidence',
        'preview_blueprint', 'submit_proposal', 'finish_architecture'
    )),
    state TEXT NOT NULL CHECK (state IN ('started', 'succeeded', 'failed', 'interrupted')),
    call_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (run_id, sequence)
);

CREATE TABLE benchmark_architect_evidence (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES benchmark_architect_runs(id) ON DELETE RESTRICT,
    tool_call_id TEXT NOT NULL REFERENCES benchmark_architect_tool_calls(id) ON DELETE RESTRICT,
    canonical_url TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_class TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    evidence_json TEXT NOT NULL,
    retrieved_at TEXT NOT NULL,
    UNIQUE (run_id, canonical_url, content_hash)
);

CREATE TABLE benchmark_architecture_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES benchmark_architect_runs(id) ON DELETE RESTRICT,
    brief_id TEXT NOT NULL REFERENCES benchmark_architect_briefs(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    validation_fingerprint TEXT NOT NULL,
    proposal_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE benchmark_architecture_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES benchmark_architecture_proposals(id) ON DELETE RESTRICT,
    predecessor_id TEXT REFERENCES benchmark_architecture_reviews(id) ON DELETE RESTRICT,
    predecessor_fingerprint TEXT,
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'reject', 'request_revision')),
    fingerprint TEXT NOT NULL UNIQUE,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK ((predecessor_id IS NULL) = (predecessor_fingerprint IS NULL))
);

CREATE TABLE benchmark_acquisition_handoffs (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL UNIQUE REFERENCES benchmark_architecture_proposals(id) ON DELETE RESTRICT,
    approval_id TEXT NOT NULL REFERENCES benchmark_architecture_reviews(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    handoff_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_benchmark_architect_runs_brief
    ON benchmark_architect_runs(brief_id, created_at, id);
CREATE INDEX idx_benchmark_architect_tool_calls_run
    ON benchmark_architect_tool_calls(run_id, sequence);
CREATE INDEX idx_benchmark_architect_evidence_run
    ON benchmark_architect_evidence(run_id, source_class, retrieved_at, id);
CREATE INDEX idx_benchmark_architecture_reviews_proposal
    ON benchmark_architecture_reviews(proposal_id, created_at, id);

CREATE TRIGGER benchmark_architect_briefs_immutable
BEFORE UPDATE ON benchmark_architect_briefs
BEGIN SELECT RAISE(ABORT, 'benchmark architect briefs are immutable'); END;
CREATE TRIGGER benchmark_architect_briefs_no_delete
BEFORE DELETE ON benchmark_architect_briefs
BEGIN SELECT RAISE(ABORT, 'benchmark architect briefs are append-only'); END;

CREATE TRIGGER benchmark_architect_evidence_immutable
BEFORE UPDATE ON benchmark_architect_evidence
BEGIN SELECT RAISE(ABORT, 'benchmark architect evidence is immutable'); END;
CREATE TRIGGER benchmark_architect_evidence_no_delete
BEFORE DELETE ON benchmark_architect_evidence
BEGIN SELECT RAISE(ABORT, 'benchmark architect evidence is append-only'); END;

CREATE TRIGGER benchmark_architecture_proposals_immutable
BEFORE UPDATE ON benchmark_architecture_proposals
BEGIN SELECT RAISE(ABORT, 'benchmark architecture proposals are immutable'); END;
CREATE TRIGGER benchmark_architecture_proposals_no_delete
BEFORE DELETE ON benchmark_architecture_proposals
BEGIN SELECT RAISE(ABORT, 'benchmark architecture proposals are append-only'); END;

CREATE TRIGGER benchmark_architecture_reviews_immutable
BEFORE UPDATE ON benchmark_architecture_reviews
BEGIN SELECT RAISE(ABORT, 'benchmark architecture reviews are immutable'); END;
CREATE TRIGGER benchmark_architecture_reviews_no_delete
BEFORE DELETE ON benchmark_architecture_reviews
BEGIN SELECT RAISE(ABORT, 'benchmark architecture reviews are append-only'); END;

CREATE TRIGGER benchmark_acquisition_handoffs_immutable
BEFORE UPDATE ON benchmark_acquisition_handoffs
BEGIN SELECT RAISE(ABORT, 'benchmark acquisition handoffs are immutable'); END;
CREATE TRIGGER benchmark_acquisition_handoffs_no_delete
BEFORE DELETE ON benchmark_acquisition_handoffs
BEGIN SELECT RAISE(ABORT, 'benchmark acquisition handoffs are append-only'); END;
