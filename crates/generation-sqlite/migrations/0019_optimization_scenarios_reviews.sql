CREATE TABLE optimization_scenario_groups (
    id TEXT PRIMARY KEY NOT NULL,
    source_evidence_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    group_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE optimization_scenarios (
    id TEXT PRIMARY KEY NOT NULL,
    group_id TEXT NOT NULL REFERENCES optimization_scenario_groups(id) ON DELETE RESTRICT,
    scenario_index INTEGER NOT NULL CHECK (scenario_index >= 0),
    kind TEXT NOT NULL,
    protocol_fingerprint TEXT NOT NULL,
    proposal_id TEXT NOT NULL,
    proposal_fingerprint TEXT NOT NULL,
    allocated_budget INTEGER NOT NULL CHECK (allocated_budget >= 0),
    unallocated_budget INTEGER NOT NULL CHECK (unallocated_budget >= 0),
    concentration_json TEXT NOT NULL,
    scenario_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    UNIQUE (group_id, scenario_index),
    UNIQUE (group_id, kind)
);

CREATE TABLE optimization_scenario_comparisons (
    group_id TEXT NOT NULL REFERENCES optimization_scenario_groups(id) ON DELETE RESTRICT,
    comparison_index INTEGER NOT NULL CHECK (comparison_index >= 0),
    left_scenario_id TEXT NOT NULL REFERENCES optimization_scenarios(id) ON DELETE RESTRICT,
    right_scenario_id TEXT NOT NULL REFERENCES optimization_scenarios(id) ON DELETE RESTRICT,
    shared_allocated_cells INTEGER NOT NULL CHECK (shared_allocated_cells >= 0),
    union_allocated_cells INTEGER NOT NULL CHECK (union_allocated_cells >= shared_allocated_cells),
    allocation_overlap REAL NOT NULL CHECK (allocation_overlap >= 0 AND allocation_overlap <= 1),
    comparison_json TEXT NOT NULL,
    PRIMARY KEY (group_id, comparison_index),
    UNIQUE (group_id, left_scenario_id, right_scenario_id)
);

CREATE TABLE optimization_proposal_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    proposal_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL,
    note TEXT,
    superseding_proposal_id TEXT REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    campaign_id TEXT,
    fingerprint TEXT NOT NULL,
    review_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE optimization_proposal_review_selections (
    review_id TEXT NOT NULL REFERENCES optimization_proposal_reviews(id) ON DELETE RESTRICT,
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    recommendation_id TEXT NOT NULL REFERENCES optimization_recommendations(id) ON DELETE RESTRICT,
    selection_index INTEGER NOT NULL CHECK (selection_index >= 0),
    PRIMARY KEY (review_id, recommendation_id),
    UNIQUE (review_id, selection_index)
);

CREATE INDEX optimization_scenario_groups_source_idx
    ON optimization_scenario_groups(source_evidence_fingerprint, created_at, id);
CREATE INDEX optimization_proposal_reviews_query_idx
    ON optimization_proposal_reviews(proposal_id, state, created_at, id);
