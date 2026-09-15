ALTER TABLE optimization_proposal_applications
    ADD COLUMN approval_review_id TEXT REFERENCES optimization_proposal_reviews(id) ON DELETE RESTRICT;
ALTER TABLE optimization_proposal_applications ADD COLUMN approval_fingerprint TEXT;
ALTER TABLE optimization_proposal_applications
    ADD COLUMN selected_recommendation_ids_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE optimization_proposal_applications ADD COLUMN verified_coverage_fingerprint TEXT;

CREATE UNIQUE INDEX optimization_proposal_applications_approval_idx
    ON optimization_proposal_applications(approval_review_id)
    WHERE approval_review_id IS NOT NULL;
