CREATE TABLE optimization_training_configuration_spaces (
    proposal_id TEXT PRIMARY KEY NOT NULL
        REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL,
    baseline_training_run_id TEXT NOT NULL
        REFERENCES training_runs(id) ON DELETE RESTRICT,
    baseline_training_run_fingerprint TEXT NOT NULL,
    baseline_checkpoint_id TEXT NOT NULL
        REFERENCES training_checkpoints(id) ON DELETE RESTRICT,
    baseline_checkpoint_fingerprint TEXT NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    snapshot_fingerprint TEXT NOT NULL,
    backend_name TEXT NOT NULL,
    model_format TEXT NOT NULL,
    choice_count INTEGER NOT NULL CHECK (choice_count > 0 AND choice_count <= 256),
    space_json TEXT NOT NULL
);

CREATE TABLE optimization_training_candidates (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL
        REFERENCES optimization_training_configuration_spaces(proposal_id) ON DELETE RESTRICT,
    configuration_space_fingerprint TEXT NOT NULL,
    candidate_index INTEGER NOT NULL CHECK (candidate_index >= 0),
    changed_field_count INTEGER NOT NULL CHECK (changed_field_count > 0),
    changed_fields_json TEXT NOT NULL,
    candidate_fingerprint TEXT NOT NULL,
    candidate_json TEXT NOT NULL,
    UNIQUE (proposal_id, candidate_index)
);

CREATE TABLE optimization_proposal_review_training_selections (
    review_id TEXT NOT NULL
        REFERENCES optimization_proposal_reviews(id) ON DELETE RESTRICT,
    proposal_id TEXT NOT NULL
        REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    candidate_id TEXT NOT NULL
        REFERENCES optimization_training_candidates(id) ON DELETE RESTRICT,
    selection_index INTEGER NOT NULL CHECK (selection_index >= 0),
    PRIMARY KEY (review_id, candidate_id),
    UNIQUE (review_id, selection_index)
);

CREATE INDEX optimization_training_candidates_query_idx
    ON optimization_training_candidates(proposal_id, changed_field_count, candidate_index);
