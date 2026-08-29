CREATE TABLE optimization_review_only_recommendations (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    cell_key TEXT NOT NULL,
    label TEXT NOT NULL,
    finding_key TEXT NOT NULL,
    finding_fingerprint TEXT NOT NULL,
    evidence_fingerprint TEXT NOT NULL,
    final_score REAL NOT NULL CHECK (final_score >= 0),
    recommendation_fingerprint TEXT NOT NULL,
    recommendation_json TEXT NOT NULL,
    UNIQUE (proposal_id, cell_key)
);

CREATE TABLE optimization_proposal_review_advisory_selections (
    review_id TEXT NOT NULL
        REFERENCES optimization_proposal_reviews(id) ON DELETE RESTRICT,
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    recommendation_id TEXT NOT NULL
        REFERENCES optimization_review_only_recommendations(id) ON DELETE RESTRICT,
    selection_index INTEGER NOT NULL CHECK (selection_index >= 0),
    PRIMARY KEY (review_id, recommendation_id),
    UNIQUE (review_id, selection_index)
);

CREATE INDEX optimization_review_only_recommendations_query_idx
    ON optimization_review_only_recommendations(proposal_id, label, final_score DESC, cell_key);
