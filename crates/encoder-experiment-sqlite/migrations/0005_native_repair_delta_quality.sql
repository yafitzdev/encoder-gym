CREATE TABLE encoder_native_delta_candidate_sets (
    id TEXT PRIMARY KEY NOT NULL,
    evidence_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    proposal_id TEXT NOT NULL UNIQUE,
    application_id TEXT NOT NULL UNIQUE,
    execution_project_snapshot_id TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id),
    FOREIGN KEY (application_id) REFERENCES encoder_repair_proposal_applications(id),
    FOREIGN KEY (execution_project_snapshot_id) REFERENCES encoder_experiment_projects(id)
);

CREATE TABLE encoder_native_delta_reports (
    id TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    candidate_set_id TEXT NOT NULL UNIQUE,
    proposal_id TEXT NOT NULL,
    policy_fingerprint TEXT NOT NULL,
    eligible INTEGER NOT NULL CHECK (eligible IN (0, 1)),
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (candidate_set_id) REFERENCES encoder_native_delta_candidate_sets(id),
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id)
);

CREATE TABLE encoder_native_delta_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    report_id TEXT NOT NULL,
    proposal_id TEXT NOT NULL,
    predecessor_id TEXT,
    decision TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (report_id) REFERENCES encoder_native_delta_reports(id),
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id),
    FOREIGN KEY (predecessor_id) REFERENCES encoder_native_delta_reviews(id)
);

CREATE TABLE encoder_native_delta_selections (
    id TEXT PRIMARY KEY NOT NULL,
    specification_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    proposal_id TEXT NOT NULL UNIQUE,
    candidate_set_id TEXT NOT NULL UNIQUE,
    report_id TEXT NOT NULL UNIQUE,
    approval_id TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id),
    FOREIGN KEY (candidate_set_id) REFERENCES encoder_native_delta_candidate_sets(id),
    FOREIGN KEY (report_id) REFERENCES encoder_native_delta_reports(id),
    FOREIGN KEY (approval_id) REFERENCES encoder_native_delta_reviews(id)
);

CREATE INDEX idx_encoder_native_delta_reviews_report
    ON encoder_native_delta_reviews(report_id, created_at, id);

CREATE TRIGGER encoder_native_delta_candidate_sets_no_update
BEFORE UPDATE ON encoder_native_delta_candidate_sets
BEGIN SELECT RAISE(ABORT, 'native delta candidate sets are immutable'); END;
CREATE TRIGGER encoder_native_delta_candidate_sets_no_delete
BEFORE DELETE ON encoder_native_delta_candidate_sets
BEGIN SELECT RAISE(ABORT, 'native delta candidate sets are append-only'); END;
CREATE TRIGGER encoder_native_delta_reports_no_update
BEFORE UPDATE ON encoder_native_delta_reports
BEGIN SELECT RAISE(ABORT, 'native delta reports are immutable'); END;
CREATE TRIGGER encoder_native_delta_reports_no_delete
BEFORE DELETE ON encoder_native_delta_reports
BEGIN SELECT RAISE(ABORT, 'native delta reports are append-only'); END;
CREATE TRIGGER encoder_native_delta_reviews_no_update
BEFORE UPDATE ON encoder_native_delta_reviews
BEGIN SELECT RAISE(ABORT, 'native delta reviews are immutable'); END;
CREATE TRIGGER encoder_native_delta_reviews_no_delete
BEFORE DELETE ON encoder_native_delta_reviews
BEGIN SELECT RAISE(ABORT, 'native delta reviews are append-only'); END;
CREATE TRIGGER encoder_native_delta_selections_no_update
BEFORE UPDATE ON encoder_native_delta_selections
BEGIN SELECT RAISE(ABORT, 'native delta selections are immutable'); END;
CREATE TRIGGER encoder_native_delta_selections_no_delete
BEFORE DELETE ON encoder_native_delta_selections
BEGIN SELECT RAISE(ABORT, 'native delta selections are append-only'); END;
