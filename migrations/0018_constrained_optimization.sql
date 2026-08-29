ALTER TABLE optimization_proposals ADD COLUMN protocol_fingerprint TEXT;
ALTER TABLE optimization_proposals ADD COLUMN evidence_fingerprint TEXT;
ALTER TABLE optimization_proposals ADD COLUMN source_snapshot_id TEXT REFERENCES dataset_snapshots(id) ON DELETE RESTRICT;
ALTER TABLE optimization_proposals ADD COLUMN scoring_policy TEXT;
ALTER TABLE optimization_proposals ADD COLUMN unallocated_budget INTEGER CHECK (unallocated_budget >= 0);

CREATE TABLE optimization_protocols (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL,
    protocol_json TEXT NOT NULL
);

CREATE TABLE optimization_proposal_sources (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    analysis_fingerprint TEXT NOT NULL,
    analysis_protocol_fingerprint TEXT NOT NULL,
    diagnostic_contract_fingerprint TEXT NOT NULL,
    evaluation_run_id TEXT NOT NULL REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    evaluation_input_fingerprint TEXT NOT NULL,
    evaluation_protocol_fingerprint TEXT NOT NULL,
    cohort_fingerprint TEXT NOT NULL,
    dataset_fingerprint TEXT NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    snapshot_fingerprint TEXT NOT NULL,
    coverage_fingerprint TEXT NOT NULL,
    comparison_id TEXT REFERENCES evaluation_comparisons(id) ON DELETE RESTRICT,
    comparison_fingerprint TEXT,
    training_configuration_space_fingerprint TEXT,
    source_json TEXT NOT NULL,
    CHECK ((comparison_id IS NULL) = (comparison_fingerprint IS NULL))
);

CREATE TABLE optimization_allocations (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    requested_budget INTEGER NOT NULL CHECK (requested_budget > 0),
    allocated_budget INTEGER NOT NULL CHECK (allocated_budget >= 0),
    unallocated_budget INTEGER NOT NULL CHECK (unallocated_budget >= 0),
    feasibility TEXT NOT NULL CHECK (feasibility IN ('feasible', 'infeasible')),
    allocation_json TEXT NOT NULL,
    CHECK (allocated_budget + unallocated_budget = requested_budget)
);

CREATE TABLE optimization_constraint_issues (
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    issue_index INTEGER NOT NULL CHECK (issue_index >= 0),
    issue_json TEXT NOT NULL,
    PRIMARY KEY (proposal_id, issue_index)
);

CREATE TABLE optimization_decision_cells (
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    finding_key TEXT NOT NULL,
    cell_key TEXT NOT NULL,
    label TEXT NOT NULL,
    eligible INTEGER NOT NULL CHECK (eligible IN (0, 1)),
    final_score REAL NOT NULL CHECK (final_score >= 0),
    evidence_fingerprint TEXT NOT NULL,
    decision_json TEXT NOT NULL,
    PRIMARY KEY (proposal_id, finding_key)
);

CREATE TABLE optimization_recommendations (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL,
    cell_key TEXT NOT NULL,
    label TEXT NOT NULL,
    eligible INTEGER NOT NULL CHECK (eligible IN (0, 1)),
    final_score REAL NOT NULL CHECK (final_score >= 0),
    current_accepted INTEGER NOT NULL CHECK (current_accepted >= 0),
    additional_count INTEGER NOT NULL CHECK (additional_count > 0),
    proposed_target INTEGER NOT NULL CHECK (proposed_target >= additional_count),
    finding_key TEXT NOT NULL,
    evidence_fingerprint TEXT NOT NULL,
    recommendation_fingerprint TEXT NOT NULL,
    constraints_json TEXT NOT NULL,
    recommendation_json TEXT NOT NULL,
    UNIQUE (proposal_id, cell_key)
);

CREATE INDEX optimization_proposals_protocol_idx
    ON optimization_proposals(protocol_fingerprint, created_at, id);
CREATE INDEX optimization_decision_cells_filter_idx
    ON optimization_decision_cells(proposal_id, eligible, label, final_score DESC, cell_key);
CREATE INDEX optimization_recommendations_filter_idx
    ON optimization_recommendations(proposal_id, kind, label, final_score DESC, cell_key);
