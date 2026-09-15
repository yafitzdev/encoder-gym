CREATE TABLE generation_quality_contracts (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL REFERENCES generation_plans(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    contract_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_runs (
    id TEXT PRIMARY KEY NOT NULL,
    contract_id TEXT NOT NULL REFERENCES generation_quality_contracts(id) ON DELETE RESTRICT,
    initial_prompt_version_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN (
        'queued', 'running', 'generating', 'assessing', 'paused', 'diagnosing',
        'awaiting_review', 'canary', 'completed', 'failed', 'cancelled'
    )),
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    run_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_run_events (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    previous_event_fingerprint TEXT,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    event_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (run_id, sequence)
);

CREATE TABLE generation_supervisor_prompt_versions (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    parent_version_id TEXT REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    source_proposal_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    version_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (run_id, sequence)
);

CREATE TABLE generation_supervisor_strategy_sets (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL REFERENCES generation_plans(id) ON DELETE RESTRICT,
    context_id TEXT NOT NULL REFERENCES generation_strategy_contexts(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    set_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_strategy_assignments (
    assignment_set_id TEXT NOT NULL REFERENCES generation_supervisor_strategy_sets(id) ON DELETE RESTRICT,
    cell_key TEXT NOT NULL,
    row_sequence INTEGER NOT NULL CHECK (row_sequence >= 0),
    directive_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    assignment_json TEXT NOT NULL,
    PRIMARY KEY (assignment_set_id, cell_key, row_sequence)
);

CREATE TABLE generation_supervisor_child_reservations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL,
    logical_input_key TEXT NOT NULL,
    child_id TEXT NOT NULL,
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    replaces_reservation_id TEXT REFERENCES generation_supervisor_child_reservations(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    reservation_json TEXT NOT NULL,
    reserved_at TEXT NOT NULL,
    UNIQUE (run_id, kind, logical_input_key, attempt),
    UNIQUE (run_id, child_id)
);

CREATE TABLE generation_supervisor_child_outcomes (
    id TEXT PRIMARY KEY NOT NULL,
    reservation_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_child_reservations(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('succeeded', 'failed', 'interrupted', 'cancelled')),
    fingerprint TEXT NOT NULL UNIQUE,
    outcome_json TEXT NOT NULL,
    finished_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_row_observations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    contract_id TEXT NOT NULL REFERENCES generation_quality_contracts(id) ON DELETE RESTRICT,
    prompt_version_id TEXT NOT NULL REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    generated_row_id TEXT NOT NULL REFERENCES generated_rows(id) ON DELETE RESTRICT,
    generation_job_id TEXT NOT NULL REFERENCES generation_jobs(id) ON DELETE RESTRICT,
    generation_attempt_id TEXT NOT NULL REFERENCES generation_request_attempts(id) ON DELETE RESTRICT,
    cell_key TEXT NOT NULL,
    directive_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    observation_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (run_id, generated_row_id)
);

CREATE TABLE generation_supervisor_quality_manifests (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    window_id TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    manifest_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_manifest_members (
    manifest_id TEXT NOT NULL REFERENCES generation_supervisor_quality_manifests(id) ON DELETE RESTRICT,
    observation_id TEXT NOT NULL REFERENCES generation_supervisor_row_observations(id) ON DELETE RESTRICT,
    observation_fingerprint TEXT NOT NULL,
    generated_row_id TEXT NOT NULL,
    PRIMARY KEY (manifest_id, observation_id)
);

CREATE TABLE generation_supervisor_quality_windows (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    contract_id TEXT NOT NULL REFERENCES generation_quality_contracts(id) ON DELETE RESTRICT,
    prompt_version_id TEXT NOT NULL REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    manifest_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_quality_manifests(id) ON DELETE RESTRICT,
    scope_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    fingerprint TEXT NOT NULL UNIQUE,
    observation_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (run_id, scope_key, kind, sequence)
);

CREATE TABLE generation_supervisor_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    contract_id TEXT NOT NULL REFERENCES generation_quality_contracts(id) ON DELETE RESTRICT,
    window_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_quality_windows(id) ON DELETE RESTRICT,
    state TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    decision_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_diagnosis_briefs (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    contract_id TEXT NOT NULL REFERENCES generation_quality_contracts(id) ON DELETE RESTRICT,
    decision_id TEXT NOT NULL REFERENCES generation_supervisor_decisions(id) ON DELETE RESTRICT,
    window_id TEXT NOT NULL REFERENCES generation_supervisor_quality_windows(id) ON DELETE RESTRICT,
    prompt_version_id TEXT NOT NULL REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    brief_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_advisor_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    brief_id TEXT NOT NULL REFERENCES generation_supervisor_diagnosis_briefs(id) ON DELETE RESTRICT,
    state TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    session_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_model_calls (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES generation_supervisor_advisor_sessions(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    state TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    call_json TEXT NOT NULL,
    reserved_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (session_id, sequence)
);

CREATE TABLE generation_supervisor_tool_calls (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES generation_supervisor_advisor_sessions(id) ON DELETE RESTRICT,
    external_call_id TEXT NOT NULL,
    name TEXT NOT NULL,
    state TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    call_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (session_id, external_call_id)
);

CREATE TABLE generation_supervisor_diagnoses (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_advisor_sessions(id) ON DELETE RESTRICT,
    decision_id TEXT NOT NULL REFERENCES generation_supervisor_decisions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    diagnosis_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_revision_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_advisor_sessions(id) ON DELETE RESTRICT,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    diagnosis_id TEXT NOT NULL REFERENCES generation_supervisor_diagnoses(id) ON DELETE RESTRICT,
    parent_prompt_version_id TEXT NOT NULL REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    proposal_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_revision_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES generation_supervisor_revision_proposals(id) ON DELETE RESTRICT,
    predecessor_id TEXT REFERENCES generation_supervisor_revision_reviews(id) ON DELETE RESTRICT,
    predecessor_fingerprint TEXT,
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'reject', 'request_revision')),
    fingerprint TEXT NOT NULL UNIQUE,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_revision_authorizations (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES generation_supervisor_revision_proposals(id) ON DELETE RESTRICT,
    review_id TEXT REFERENCES generation_supervisor_revision_reviews(id) ON DELETE RESTRICT,
    prompt_version_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    authorization_json TEXT NOT NULL,
    authorized_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_revision_activations (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    prompt_version_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_prompt_versions(id) ON DELETE RESTRICT,
    canary_decision_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_decisions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL UNIQUE,
    activation_json TEXT NOT NULL,
    activated_at TEXT NOT NULL
);

CREATE INDEX idx_generation_supervisor_rows_scope
    ON generation_supervisor_row_observations(run_id, cell_key, directive_id, created_at);
CREATE INDEX idx_generation_supervisor_windows_run
    ON generation_supervisor_quality_windows(run_id, created_at, id);
CREATE INDEX idx_generation_supervisor_decisions_run
    ON generation_supervisor_decisions(run_id, created_at, id);

CREATE TRIGGER generation_quality_contracts_immutable
BEFORE UPDATE ON generation_quality_contracts
BEGIN SELECT RAISE(ABORT, 'generation quality contracts are immutable'); END;
CREATE TRIGGER generation_quality_contracts_no_delete
BEFORE DELETE ON generation_quality_contracts
BEGIN SELECT RAISE(ABORT, 'generation quality contracts are append-only'); END;

CREATE TRIGGER generation_supervisor_run_events_immutable
BEFORE UPDATE ON generation_supervisor_run_events
BEGIN SELECT RAISE(ABORT, 'generation supervisor run events are immutable'); END;
CREATE TRIGGER generation_supervisor_run_events_no_delete
BEFORE DELETE ON generation_supervisor_run_events
BEGIN SELECT RAISE(ABORT, 'generation supervisor run events are append-only'); END;

CREATE TRIGGER generation_supervisor_prompt_versions_immutable
BEFORE UPDATE ON generation_supervisor_prompt_versions
BEGIN SELECT RAISE(ABORT, 'generation supervisor prompt versions are immutable'); END;
CREATE TRIGGER generation_supervisor_prompt_versions_no_delete
BEFORE DELETE ON generation_supervisor_prompt_versions
BEGIN SELECT RAISE(ABORT, 'generation supervisor prompt versions are append-only'); END;

CREATE TRIGGER generation_supervisor_strategy_sets_immutable
BEFORE UPDATE ON generation_supervisor_strategy_sets
BEGIN SELECT RAISE(ABORT, 'generation supervisor strategy sets are immutable'); END;
CREATE TRIGGER generation_supervisor_strategy_sets_no_delete
BEFORE DELETE ON generation_supervisor_strategy_sets
BEGIN SELECT RAISE(ABORT, 'generation supervisor strategy sets are append-only'); END;

CREATE TRIGGER generation_supervisor_strategy_assignments_immutable
BEFORE UPDATE ON generation_supervisor_strategy_assignments
BEGIN SELECT RAISE(ABORT, 'generation supervisor strategy assignments are immutable'); END;
CREATE TRIGGER generation_supervisor_strategy_assignments_no_delete
BEFORE DELETE ON generation_supervisor_strategy_assignments
BEGIN SELECT RAISE(ABORT, 'generation supervisor strategy assignments are append-only'); END;

CREATE TRIGGER generation_supervisor_child_reservations_immutable
BEFORE UPDATE ON generation_supervisor_child_reservations
BEGIN SELECT RAISE(ABORT, 'generation supervisor child reservations are immutable'); END;
CREATE TRIGGER generation_supervisor_child_reservations_no_delete
BEFORE DELETE ON generation_supervisor_child_reservations
BEGIN SELECT RAISE(ABORT, 'generation supervisor child reservations are append-only'); END;

CREATE TRIGGER generation_supervisor_child_outcomes_immutable
BEFORE UPDATE ON generation_supervisor_child_outcomes
BEGIN SELECT RAISE(ABORT, 'generation supervisor child outcomes are immutable'); END;
CREATE TRIGGER generation_supervisor_child_outcomes_no_delete
BEFORE DELETE ON generation_supervisor_child_outcomes
BEGIN SELECT RAISE(ABORT, 'generation supervisor child outcomes are append-only'); END;

CREATE TRIGGER generation_supervisor_row_observations_immutable
BEFORE UPDATE ON generation_supervisor_row_observations
BEGIN SELECT RAISE(ABORT, 'generation supervisor row observations are immutable'); END;
CREATE TRIGGER generation_supervisor_row_observations_no_delete
BEFORE DELETE ON generation_supervisor_row_observations
BEGIN SELECT RAISE(ABORT, 'generation supervisor row observations are append-only'); END;

CREATE TRIGGER generation_supervisor_quality_manifests_immutable
BEFORE UPDATE ON generation_supervisor_quality_manifests
BEGIN SELECT RAISE(ABORT, 'generation supervisor quality manifests are immutable'); END;
CREATE TRIGGER generation_supervisor_quality_manifests_no_delete
BEFORE DELETE ON generation_supervisor_quality_manifests
BEGIN SELECT RAISE(ABORT, 'generation supervisor quality manifests are append-only'); END;

CREATE TRIGGER generation_supervisor_manifest_members_immutable
BEFORE UPDATE ON generation_supervisor_manifest_members
BEGIN SELECT RAISE(ABORT, 'generation supervisor manifest members are immutable'); END;
CREATE TRIGGER generation_supervisor_manifest_members_no_delete
BEFORE DELETE ON generation_supervisor_manifest_members
BEGIN SELECT RAISE(ABORT, 'generation supervisor manifest members are append-only'); END;

CREATE TRIGGER generation_supervisor_quality_windows_immutable
BEFORE UPDATE ON generation_supervisor_quality_windows
BEGIN SELECT RAISE(ABORT, 'generation supervisor quality windows are immutable'); END;
CREATE TRIGGER generation_supervisor_quality_windows_no_delete
BEFORE DELETE ON generation_supervisor_quality_windows
BEGIN SELECT RAISE(ABORT, 'generation supervisor quality windows are append-only'); END;

CREATE TRIGGER generation_supervisor_decisions_immutable
BEFORE UPDATE ON generation_supervisor_decisions
BEGIN SELECT RAISE(ABORT, 'generation supervisor decisions are immutable'); END;
CREATE TRIGGER generation_supervisor_decisions_no_delete
BEFORE DELETE ON generation_supervisor_decisions
BEGIN SELECT RAISE(ABORT, 'generation supervisor decisions are append-only'); END;

CREATE TRIGGER generation_supervisor_diagnosis_briefs_immutable
BEFORE UPDATE ON generation_supervisor_diagnosis_briefs
BEGIN SELECT RAISE(ABORT, 'generation supervisor diagnosis briefs are immutable'); END;
CREATE TRIGGER generation_supervisor_diagnosis_briefs_no_delete
BEFORE DELETE ON generation_supervisor_diagnosis_briefs
BEGIN SELECT RAISE(ABORT, 'generation supervisor diagnosis briefs are append-only'); END;

CREATE TRIGGER generation_supervisor_diagnoses_immutable
BEFORE UPDATE ON generation_supervisor_diagnoses
BEGIN SELECT RAISE(ABORT, 'generation supervisor diagnoses are immutable'); END;
CREATE TRIGGER generation_supervisor_diagnoses_no_delete
BEFORE DELETE ON generation_supervisor_diagnoses
BEGIN SELECT RAISE(ABORT, 'generation supervisor diagnoses are append-only'); END;

CREATE TRIGGER generation_supervisor_revision_proposals_immutable
BEFORE UPDATE ON generation_supervisor_revision_proposals
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision proposals are immutable'); END;
CREATE TRIGGER generation_supervisor_revision_proposals_no_delete
BEFORE DELETE ON generation_supervisor_revision_proposals
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision proposals are append-only'); END;

CREATE TRIGGER generation_supervisor_revision_reviews_immutable
BEFORE UPDATE ON generation_supervisor_revision_reviews
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision reviews are immutable'); END;
CREATE TRIGGER generation_supervisor_revision_reviews_no_delete
BEFORE DELETE ON generation_supervisor_revision_reviews
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision reviews are append-only'); END;

CREATE TRIGGER generation_supervisor_revision_authorizations_immutable
BEFORE UPDATE ON generation_supervisor_revision_authorizations
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision authorizations are immutable'); END;
CREATE TRIGGER generation_supervisor_revision_authorizations_no_delete
BEFORE DELETE ON generation_supervisor_revision_authorizations
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision authorizations are append-only'); END;

CREATE TRIGGER generation_supervisor_revision_activations_immutable
BEFORE UPDATE ON generation_supervisor_revision_activations
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision activations are immutable'); END;
CREATE TRIGGER generation_supervisor_revision_activations_no_delete
BEFORE DELETE ON generation_supervisor_revision_activations
BEGIN SELECT RAISE(ABORT, 'generation supervisor revision activations are append-only'); END;
