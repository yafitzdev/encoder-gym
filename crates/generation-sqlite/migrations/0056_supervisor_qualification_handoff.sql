CREATE TABLE generation_supervisor_qualification_handoffs (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_runs(id) ON DELETE RESTRICT,
    contract_id TEXT NOT NULL REFERENCES generation_quality_contracts(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    plan_id TEXT NOT NULL REFERENCES generation_plans(id) ON DELETE RESTRICT,
    completion_event_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_run_events(id) ON DELETE RESTRICT,
    replay_audit_plan_id TEXT NOT NULL UNIQUE REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    replay_audit_run_id TEXT NOT NULL UNIQUE REFERENCES dataset_quality_audit_runs(id) ON DELETE RESTRICT,
    selected_member_fingerprint TEXT NOT NULL,
    complete_evidence_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    handoff_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE generation_supervisor_qualification_entries (
    handoff_id TEXT NOT NULL REFERENCES generation_supervisor_qualification_handoffs(id) ON DELETE RESTRICT,
    observation_id TEXT NOT NULL REFERENCES generation_supervisor_row_observations(id) ON DELETE RESTRICT,
    source_row_id TEXT REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    disposition TEXT NOT NULL CHECK (disposition IN ('selected', 'excluded')),
    exclusion_reason TEXT,
    cell_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    entry_json TEXT NOT NULL,
    PRIMARY KEY (handoff_id, observation_id),
    CHECK (
        (disposition = 'selected' AND source_row_id IS NOT NULL AND exclusion_reason IS NULL)
        OR (disposition = 'excluded' AND exclusion_reason IS NOT NULL)
    )
);

CREATE TABLE generation_supervisor_qualification_applications (
    id TEXT PRIMARY KEY NOT NULL,
    handoff_id TEXT NOT NULL UNIQUE REFERENCES generation_supervisor_qualification_handoffs(id) ON DELETE RESTRICT,
    replay_audit_plan_id TEXT NOT NULL UNIQUE REFERENCES dataset_quality_audit_plans(id) ON DELETE RESTRICT,
    replay_audit_run_id TEXT NOT NULL UNIQUE REFERENCES dataset_quality_audit_runs(id) ON DELETE RESTRICT,
    quality_report_id TEXT NOT NULL UNIQUE REFERENCES dataset_quality_reports(id) ON DELETE RESTRICT,
    curation_proposal_id TEXT NOT NULL UNIQUE REFERENCES dataset_curation_proposals(id) ON DELETE RESTRICT,
    selected_member_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    application_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_supervisor_qualification_entries_cell
    ON generation_supervisor_qualification_entries(handoff_id, cell_key, disposition);

CREATE TRIGGER generation_supervisor_qualification_handoffs_immutable
BEFORE UPDATE ON generation_supervisor_qualification_handoffs
BEGIN SELECT RAISE(ABORT, 'generation supervisor qualification handoffs are immutable'); END;
CREATE TRIGGER generation_supervisor_qualification_handoffs_no_delete
BEFORE DELETE ON generation_supervisor_qualification_handoffs
BEGIN SELECT RAISE(ABORT, 'generation supervisor qualification handoffs are append-only'); END;

CREATE TRIGGER generation_supervisor_qualification_entries_immutable
BEFORE UPDATE ON generation_supervisor_qualification_entries
BEGIN SELECT RAISE(ABORT, 'generation supervisor qualification entries are immutable'); END;
CREATE TRIGGER generation_supervisor_qualification_entries_no_delete
BEFORE DELETE ON generation_supervisor_qualification_entries
BEGIN SELECT RAISE(ABORT, 'generation supervisor qualification entries are append-only'); END;

CREATE TRIGGER generation_supervisor_qualification_applications_immutable
BEFORE UPDATE ON generation_supervisor_qualification_applications
BEGIN SELECT RAISE(ABORT, 'generation supervisor qualification applications are immutable'); END;
CREATE TRIGGER generation_supervisor_qualification_applications_no_delete
BEFORE DELETE ON generation_supervisor_qualification_applications
BEGIN SELECT RAISE(ABORT, 'generation supervisor qualification applications are append-only'); END;
