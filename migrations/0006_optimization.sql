CREATE TABLE optimization_proposals (
    id TEXT PRIMARY KEY NOT NULL,
    analysis_report_id TEXT NOT NULL REFERENCES analysis_reports(id) ON DELETE RESTRICT,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    additional_example_budget INTEGER NOT NULL CHECK (additional_example_budget > 0),
    minimum_support INTEGER NOT NULL CHECK (minimum_support > 0),
    proposal_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE optimization_proposal_applications (
    proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES optimization_proposals(id) ON DELETE RESTRICT,
    generation_plan_id TEXT NOT NULL UNIQUE REFERENCES generation_plans(id) ON DELETE RESTRICT,
    applied_at TEXT NOT NULL
);

CREATE INDEX optimization_proposals_analysis_idx
    ON optimization_proposals(analysis_report_id, created_at);
