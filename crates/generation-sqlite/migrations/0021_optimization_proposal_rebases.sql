CREATE TABLE optimization_proposal_rebases (
    proposal_id TEXT PRIMARY KEY NOT NULL
        REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    previous_proposal_id TEXT NOT NULL
        REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    previous_proposal_fingerprint TEXT NOT NULL,
    previous_coverage_fingerprint TEXT NOT NULL,
    refreshed_coverage_fingerprint TEXT NOT NULL,
    lineage_json TEXT NOT NULL,
    CHECK (proposal_id <> previous_proposal_id),
    CHECK (previous_coverage_fingerprint <> refreshed_coverage_fingerprint)
);

CREATE INDEX optimization_proposal_rebases_previous_idx
    ON optimization_proposal_rebases(previous_proposal_id, proposal_id);
