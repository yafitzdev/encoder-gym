CREATE TABLE workflow_approval_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    workflow_run_id TEXT NOT NULL,
    workflow_iteration INTEGER NOT NULL CHECK (workflow_iteration >= 0),
    proposal_id TEXT NOT NULL,
    proposal_review_id TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('human_review', 'preauthorized_envelope')),
    approved_additional_rows INTEGER NOT NULL CHECK (approved_additional_rows > 0),
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workflow_run_id) REFERENCES workflow_runs(id),
    FOREIGN KEY (proposal_id) REFERENCES optimization_proposals(id),
    FOREIGN KEY (proposal_review_id) REFERENCES optimization_proposal_reviews(id),
    UNIQUE (workflow_run_id, workflow_iteration)
);

CREATE INDEX workflow_approval_decisions_proposal
    ON workflow_approval_decisions(proposal_id, created_at, id);
