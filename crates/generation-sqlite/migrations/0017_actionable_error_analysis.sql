ALTER TABLE analysis_reports ADD COLUMN protocol_json TEXT;
ALTER TABLE analysis_reports ADD COLUMN protocol_fingerprint TEXT;
ALTER TABLE analysis_reports ADD COLUMN source_identity_json TEXT;
ALTER TABLE analysis_reports ADD COLUMN comparison_id TEXT REFERENCES evaluation_comparisons(id) ON DELETE RESTRICT;
ALTER TABLE analysis_reports ADD COLUMN comparison_fingerprint TEXT;

CREATE TABLE analysis_findings (
    analysis_report_id TEXT NOT NULL REFERENCES analysis_reports(id) ON DELETE RESTRICT,
    finding_key TEXT NOT NULL,
    rank INTEGER NOT NULL CHECK (rank > 0),
    kind TEXT NOT NULL,
    support INTEGER NOT NULL CHECK (support > 0),
    error_count INTEGER NOT NULL CHECK (error_count > 0 AND error_count <= support),
    identity_json TEXT NOT NULL,
    finding_json TEXT NOT NULL,
    PRIMARY KEY (analysis_report_id, finding_key),
    UNIQUE (analysis_report_id, rank)
);

CREATE INDEX analysis_findings_query_idx
    ON analysis_findings(analysis_report_id, kind, rank, finding_key);

CREATE TABLE analysis_finding_evidence (
    analysis_report_id TEXT NOT NULL,
    finding_key TEXT NOT NULL,
    evidence_order INTEGER NOT NULL CHECK (evidence_order >= 0),
    category TEXT NOT NULL,
    prediction_id TEXT NOT NULL REFERENCES evaluation_predictions(id) ON DELETE RESTRICT,
    snapshot_member_id TEXT NOT NULL REFERENCES dataset_snapshot_members(id) ON DELETE RESTRICT,
    source_row_id TEXT NOT NULL REFERENCES dataset_source_rows(id) ON DELETE RESTRICT,
    evidence_json TEXT NOT NULL,
    PRIMARY KEY (analysis_report_id, finding_key, evidence_order),
    UNIQUE (analysis_report_id, finding_key, prediction_id),
    FOREIGN KEY (analysis_report_id, finding_key)
        REFERENCES analysis_findings(analysis_report_id, finding_key) ON DELETE RESTRICT
);

CREATE INDEX analysis_finding_evidence_prediction_idx
    ON analysis_finding_evidence(prediction_id, analysis_report_id, finding_key);

CREATE TABLE analysis_finding_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    analysis_report_id TEXT NOT NULL,
    finding_key TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'open',
        'acknowledged',
        'accepted_limitation',
        'candidate_for_more_data',
        'candidate_for_label_schema_review',
        'resolved_by_later_evidence'
    )),
    note TEXT,
    resolution_evaluation_run_id TEXT REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    resolution_comparison_id TEXT REFERENCES evaluation_comparisons(id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (analysis_report_id, finding_key)
        REFERENCES analysis_findings(analysis_report_id, finding_key) ON DELETE RESTRICT
);

CREATE INDEX analysis_finding_reviews_history_idx
    ON analysis_finding_reviews(analysis_report_id, finding_key, created_at, id);

-- Preserve queryability for reports created before normalized finding storage.
-- Their original keys remain legacy display identifiers; new reports enforce
-- canonical FindingIdentity keys in the application layer.
INSERT INTO analysis_findings (
    analysis_report_id, finding_key, rank, kind, support, error_count,
    identity_json, finding_json
)
SELECT
    reports.id,
    json_extract(finding.value, '$.key'),
    json_extract(finding.value, '$.rank'),
    json_extract(finding.value, '$.kind'),
    json_extract(finding.value, '$.support'),
    json_extract(finding.value, '$.error_count'),
    json_object(
        'kind', json_extract(finding.value, '$.kind'),
        'attributes', json(json_extract(finding.value, '$.attributes'))
    ),
    finding.value
FROM analysis_reports AS reports,
     json_each(reports.report_json, '$.findings') AS finding;
