CREATE TABLE optimization_campaigns (
    id TEXT PRIMARY KEY NOT NULL,
    proposal_id TEXT NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    proposal_fingerprint TEXT NOT NULL,
    approval_review_id TEXT NOT NULL
        REFERENCES optimization_proposal_reviews(id) ON DELETE RESTRICT,
    approval_fingerprint TEXT NOT NULL,
    baseline_analysis_report_id TEXT NOT NULL
        REFERENCES analysis_reports(id) ON DELETE RESTRICT,
    baseline_analysis_fingerprint TEXT NOT NULL,
    baseline_evaluation_run_id TEXT NOT NULL
        REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    baseline_comparison_id TEXT REFERENCES evaluation_comparisons(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    source_snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    source_snapshot_fingerprint TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    campaign_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE optimization_campaign_links (
    id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL REFERENCES optimization_campaigns(id) ON DELETE RESTRICT,
    artifact_kind TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    artifact_fingerprint TEXT NOT NULL,
    details_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    link_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (campaign_id, artifact_kind, artifact_id)
);

CREATE TABLE optimization_campaign_outcomes (
    id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL UNIQUE
        REFERENCES optimization_campaigns(id) ON DELETE RESTRICT,
    comparison_id TEXT NOT NULL REFERENCES evaluation_comparisons(id) ON DELETE RESTRICT,
    comparison_fingerprint TEXT NOT NULL,
    classification TEXT NOT NULL
        CHECK (classification IN ('improved', 'regressed', 'mixed', 'inconclusive')),
    policy_fingerprint TEXT NOT NULL,
    constraints_realized INTEGER NOT NULL CHECK (constraints_realized IN (0, 1)),
    fingerprint TEXT NOT NULL,
    outcome_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX optimization_campaigns_proposal_idx
    ON optimization_campaigns(proposal_id, created_at, id);
CREATE INDEX optimization_campaign_links_query_idx
    ON optimization_campaign_links(campaign_id, artifact_kind, created_at, id);
