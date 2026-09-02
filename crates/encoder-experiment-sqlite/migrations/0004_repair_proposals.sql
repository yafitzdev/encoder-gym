CREATE TABLE encoder_repair_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    specification_fingerprint TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    diagnosis_id TEXT NOT NULL,
    source_project_snapshot_id TEXT NOT NULL,
    execution_project_snapshot_id TEXT NOT NULL,
    source_campaign_id TEXT NOT NULL,
    source_experiment_run_id TEXT NOT NULL,
    benchmark_generation_id TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (diagnosis_id) REFERENCES encoder_repair_diagnoses(id),
    FOREIGN KEY (source_project_snapshot_id) REFERENCES encoder_experiment_projects(id),
    FOREIGN KEY (execution_project_snapshot_id) REFERENCES encoder_experiment_projects(id),
    FOREIGN KEY (source_campaign_id) REFERENCES encoder_production_campaigns(id),
    FOREIGN KEY (source_experiment_run_id) REFERENCES encoder_experiment_runs(id),
    FOREIGN KEY (benchmark_generation_id) REFERENCES encoder_benchmark_generations(id)
);

CREATE TABLE encoder_repair_proposal_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL,
    predecessor_id TEXT,
    decision TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id),
    FOREIGN KEY (predecessor_id) REFERENCES encoder_repair_proposal_reviews(id)
);

CREATE TABLE encoder_repair_proposal_applications (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL UNIQUE,
    approval_id TEXT NOT NULL UNIQUE,
    reservation_key TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (proposal_id) REFERENCES encoder_repair_proposals(id),
    FOREIGN KEY (approval_id) REFERENCES encoder_repair_proposal_reviews(id)
);

CREATE INDEX idx_encoder_repair_proposals_campaign
    ON encoder_repair_proposals(source_campaign_id, created_at, id);
CREATE INDEX idx_encoder_repair_reviews_proposal
    ON encoder_repair_proposal_reviews(proposal_id, created_at, id);

CREATE TRIGGER encoder_repair_proposals_no_update
BEFORE UPDATE ON encoder_repair_proposals
BEGIN
    SELECT RAISE(ABORT, 'repair proposals are immutable');
END;

CREATE TRIGGER encoder_repair_proposals_no_delete
BEFORE DELETE ON encoder_repair_proposals
BEGIN
    SELECT RAISE(ABORT, 'repair proposals are append-only');
END;

CREATE TRIGGER encoder_repair_proposal_reviews_no_update
BEFORE UPDATE ON encoder_repair_proposal_reviews
BEGIN
    SELECT RAISE(ABORT, 'repair proposal reviews are immutable');
END;

CREATE TRIGGER encoder_repair_proposal_reviews_no_delete
BEFORE DELETE ON encoder_repair_proposal_reviews
BEGIN
    SELECT RAISE(ABORT, 'repair proposal reviews are append-only');
END;

CREATE TRIGGER encoder_repair_proposal_applications_no_update
BEFORE UPDATE ON encoder_repair_proposal_applications
BEGIN
    SELECT RAISE(ABORT, 'repair proposal applications are immutable');
END;

CREATE TRIGGER encoder_repair_proposal_applications_no_delete
BEFORE DELETE ON encoder_repair_proposal_applications
BEGIN
    SELECT RAISE(ABORT, 'repair proposal applications are append-only');
END;
