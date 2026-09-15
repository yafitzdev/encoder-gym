CREATE TABLE optimization_decision_evidence (
    proposal_id TEXT PRIMARY KEY NOT NULL
        REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL,
    diagnostic_contract_fingerprint TEXT NOT NULL,
    coverage_fingerprint TEXT NOT NULL,
    evidence_json TEXT NOT NULL
);

CREATE TABLE optimization_evidence_coverage (
    proposal_id TEXT NOT NULL
        REFERENCES optimization_decision_evidence(proposal_id) ON DELETE RESTRICT,
    cell_key TEXT NOT NULL,
    label TEXT NOT NULL,
    accepted INTEGER NOT NULL CHECK (accepted >= 0),
    coverage_index INTEGER NOT NULL CHECK (coverage_index >= 0),
    coverage_json TEXT NOT NULL,
    PRIMARY KEY (proposal_id, cell_key),
    UNIQUE (proposal_id, coverage_index)
);

CREATE INDEX optimization_evidence_coverage_query_idx
    ON optimization_evidence_coverage(proposal_id, label, cell_key);
