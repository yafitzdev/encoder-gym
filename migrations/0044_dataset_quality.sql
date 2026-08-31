CREATE TABLE dataset_quality_audit_plans (
    id TEXT PRIMARY KEY NOT NULL,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    dataset_fingerprint TEXT NOT NULL,
    policy_fingerprint TEXT NOT NULL,
    source_set_fingerprint TEXT NOT NULL,
    resolved_guidance_fingerprint TEXT NOT NULL,
    evaluator_protocol_version TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    plan_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE dataset_quality_audit_plan_items (
    plan_id TEXT NOT NULL REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    source_row_fingerprint TEXT NOT NULL,
    cell_json TEXT NOT NULL,
    provenance_stratum_json TEXT NOT NULL,
    selection TEXT NOT NULL CHECK (selection IN ('selected', 'unselected_report_only')),
    item_json TEXT NOT NULL,
    PRIMARY KEY (plan_id, source_row_id)
);

CREATE TABLE dataset_quality_audit_guidance (
    plan_id TEXT PRIMARY KEY NOT NULL
        REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    resolved_guidance_fingerprint TEXT NOT NULL,
    guidance_json TEXT NOT NULL
);

CREATE TABLE dataset_quality_audit_runs (
    id TEXT PRIMARY KEY NOT NULL,
    plan_id TEXT NOT NULL REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    specification_fingerprint TEXT NOT NULL UNIQUE,
    population_rows INTEGER NOT NULL CHECK (population_rows > 0),
    selected_rows INTEGER NOT NULL CHECK (selected_rows > 0 AND selected_rows <= population_rows),
    assessed_rows INTEGER NOT NULL CHECK (assessed_rows >= 0),
    pending_review_rows INTEGER NOT NULL CHECK (pending_review_rows >= 0),
    qualified_rows INTEGER NOT NULL CHECK (qualified_rows >= 0),
    borderline_rows INTEGER NOT NULL CHECK (borderline_rows >= 0),
    quarantined_rows INTEGER NOT NULL CHECK (quarantined_rows >= 0),
    invalid_rows INTEGER NOT NULL CHECK (invalid_rows >= 0),
    assessment_count INTEGER NOT NULL CHECK (assessment_count >= 0),
    evaluator_requests INTEGER NOT NULL CHECK (evaluator_requests >= 0),
    request_attempts INTEGER NOT NULL CHECK (request_attempts >= 0),
    input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
    output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
    total_tokens INTEGER NOT NULL CHECK (total_tokens >= 0),
    cost_microusd INTEGER NOT NULL CHECK (cost_microusd >= 0),
    cancel_requested INTEGER NOT NULL CHECK (cancel_requested IN (0, 1)),
    run_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE dataset_quality_execution_leases (
    run_id TEXT PRIMARY KEY NOT NULL
        REFERENCES dataset_quality_audit_runs(id) ON DELETE CASCADE,
    invocation_token TEXT NOT NULL UNIQUE,
    process_id INTEGER NOT NULL CHECK (process_id > 0),
    process_started_at INTEGER NOT NULL CHECK (process_started_at > 0),
    acquired_at TEXT NOT NULL
);

CREATE TABLE dataset_quality_evaluator_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES dataset_quality_audit_runs(id) ON DELETE RESTRICT,
    request_id TEXT NOT NULL UNIQUE,
    request_sequence INTEGER NOT NULL CHECK (request_sequence > 0),
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    request_fingerprint TEXT NOT NULL,
    retry_payload_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('started', 'succeeded', 'failed', 'invalid_response', 'interrupted')),
    fingerprint TEXT NOT NULL UNIQUE,
    request_json TEXT NOT NULL,
    attempt_json TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (run_id, request_sequence, attempt_number),
    UNIQUE (id, request_id)
);

CREATE TABLE dataset_quality_evaluator_attempt_rows (
    attempt_id TEXT NOT NULL REFERENCES dataset_quality_evaluator_attempts(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    PRIMARY KEY (attempt_id, source_row_id)
);

CREATE TABLE dataset_quality_row_assessments (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES dataset_quality_audit_runs(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    attempt_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    verdict TEXT NOT NULL CHECK (verdict IN ('qualified', 'borderline', 'quarantined')),
    fingerprint TEXT NOT NULL UNIQUE,
    assessment_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (attempt_id, source_row_id),
    FOREIGN KEY (attempt_id, request_id)
        REFERENCES dataset_quality_evaluator_attempts(id, request_id) ON DELETE RESTRICT
);

CREATE TABLE dataset_quality_reports (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES dataset_quality_audit_runs(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    assessment_set_fingerprint TEXT NOT NULL,
    invalid_attempt_set_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    report_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE dataset_quality_report_rows (
    report_id TEXT NOT NULL REFERENCES dataset_quality_reports(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    verdict TEXT NOT NULL CHECK (verdict IN (
        'qualified', 'borderline', 'quarantined', 'invalid_evaluator_output', 'unaudited'
    )),
    assessment_set_fingerprint TEXT NOT NULL,
    invalid_attempt_set_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    row_json TEXT NOT NULL,
    PRIMARY KEY (report_id, source_row_id),
    UNIQUE (report_id, fingerprint)
);

CREATE TABLE dataset_quality_row_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    report_id TEXT NOT NULL REFERENCES dataset_quality_reports(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    predecessor_id TEXT REFERENCES dataset_quality_row_reviews(id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('include', 'exclude', 'request_reassessment')),
    fingerprint TEXT NOT NULL UNIQUE,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (predecessor_id)
);

CREATE TABLE dataset_curation_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    report_id TEXT NOT NULL REFERENCES dataset_quality_reports(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    predecessor_id TEXT REFERENCES dataset_curation_proposals(id) ON DELETE RESTRICT,
    row_review_set_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    proposal_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (report_id, row_review_set_fingerprint),
    UNIQUE (predecessor_id)
);

CREATE TABLE dataset_curation_proposal_entries (
    proposal_id TEXT NOT NULL REFERENCES dataset_curation_proposals(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('include', 'exclude', 'needs_review')),
    basis TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    entry_json TEXT NOT NULL,
    PRIMARY KEY (proposal_id, source_row_id),
    UNIQUE (proposal_id, fingerprint)
);

CREATE TABLE dataset_curation_manifest_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES dataset_curation_proposals(id) ON DELETE RESTRICT,
    predecessor_id TEXT REFERENCES dataset_curation_manifest_reviews(id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('approve', 'reject', 'request_revision')),
    fingerprint TEXT NOT NULL UNIQUE,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (predecessor_id)
);

CREATE TABLE dataset_curation_manifests (
    id TEXT PRIMARY KEY NOT NULL,
    report_id TEXT NOT NULL REFERENCES dataset_quality_reports(id) ON DELETE RESTRICT,
    proposal_id TEXT NOT NULL UNIQUE REFERENCES dataset_curation_proposals(id) ON DELETE RESTRICT,
    approval_id TEXT NOT NULL UNIQUE REFERENCES dataset_curation_manifest_reviews(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    complete_member_fingerprint TEXT NOT NULL,
    selected_member_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    manifest_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE dataset_curation_manifest_members (
    manifest_id TEXT NOT NULL REFERENCES dataset_curation_manifests(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('include', 'exclude')),
    source_row_fingerprint TEXT NOT NULL,
    proposal_entry_fingerprint TEXT NOT NULL,
    member_json TEXT NOT NULL,
    PRIMARY KEY (manifest_id, source_row_id)
);

CREATE TABLE dataset_curation_applications (
    id TEXT PRIMARY KEY NOT NULL,
    manifest_id TEXT NOT NULL UNIQUE REFERENCES dataset_curation_manifests(id) ON DELETE RESTRICT,
    approval_id TEXT NOT NULL REFERENCES dataset_curation_manifest_reviews(id) ON DELETE RESTRICT,
    snapshot_id TEXT NOT NULL UNIQUE REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    manifest_fingerprint TEXT NOT NULL,
    snapshot_fingerprint TEXT NOT NULL,
    selected_member_fingerprint TEXT NOT NULL,
    snapshot_membership_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    application_json TEXT NOT NULL,
    applied_at TEXT NOT NULL
);

CREATE INDEX dataset_quality_plans_dataset_created_idx
    ON dataset_quality_audit_plans(dataset_id, created_at, id);
CREATE INDEX dataset_quality_plan_items_selection_idx
    ON dataset_quality_audit_plan_items(plan_id, selection, source_row_id);
CREATE INDEX dataset_quality_runs_plan_created_idx
    ON dataset_quality_audit_runs(plan_id, created_at, id);
CREATE INDEX dataset_quality_attempts_run_sequence_idx
    ON dataset_quality_evaluator_attempts(run_id, request_sequence, attempt_number);
CREATE INDEX dataset_quality_attempt_rows_source_attempt_idx
    ON dataset_quality_evaluator_attempt_rows(source_row_id, attempt_id);
CREATE INDEX dataset_snapshot_members_quality_candidate_idx
    ON dataset_snapshot_members(source_row_id, snapshot_id, split);
CREATE INDEX dataset_quality_assessments_run_row_idx
    ON dataset_quality_row_assessments(run_id, source_row_id, created_at, id);
CREATE INDEX dataset_quality_row_reviews_report_row_idx
    ON dataset_quality_row_reviews(report_id, source_row_id, created_at, id);
CREATE INDEX dataset_curation_proposals_report_created_idx
    ON dataset_curation_proposals(report_id, created_at, id);
CREATE INDEX dataset_curation_manifest_reviews_proposal_idx
    ON dataset_curation_manifest_reviews(proposal_id, created_at, id);
