CREATE TABLE workflow_evidence_exposures_v2 (
    id TEXT PRIMARY KEY NOT NULL,
    cohort_id TEXT NOT NULL REFERENCES workflow_evaluation_cohorts(id) ON DELETE RESTRICT,
    role_decision_id TEXT NOT NULL REFERENCES workflow_cohort_role_decisions(id) ON DELETE RESTRICT,
    evaluation_run_id TEXT REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    workflow_run_id TEXT REFERENCES workflow_runs(id) ON DELETE RESTRICT,
    workflow_iteration INTEGER,
    purpose TEXT NOT NULL CHECK (
        purpose IN (
            'training', 'development_evaluation', 'diagnosis', 'comparison',
            'acceptance', 'manual_inspection', 'advisor', 'optimization',
            'dataset_architecture'
        )
    ),
    disclosure TEXT NOT NULL CHECK (
        disclosure IN ('aggregate', 'slices', 'predictions', 'row_content')
    ),
    adaptation_eligible INTEGER NOT NULL,
    requires_retirement INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

INSERT INTO workflow_evidence_exposures_v2 (
    id, cohort_id, role_decision_id, evaluation_run_id, workflow_run_id,
    workflow_iteration, purpose, disclosure, adaptation_eligible,
    requires_retirement, fingerprint, artifact_json, created_at
)
SELECT
    id, cohort_id, role_decision_id, evaluation_run_id, workflow_run_id,
    workflow_iteration, purpose, disclosure, adaptation_eligible,
    requires_retirement, fingerprint, artifact_json, created_at
FROM workflow_evidence_exposures;

DROP TABLE workflow_evidence_exposures;
ALTER TABLE workflow_evidence_exposures_v2 RENAME TO workflow_evidence_exposures;

CREATE INDEX workflow_evidence_exposures_cohort_created_idx
    ON workflow_evidence_exposures(cohort_id, created_at, id);

CREATE INDEX workflow_evidence_exposures_evaluation_idx
    ON workflow_evidence_exposures(evaluation_run_id);

CREATE INDEX workflow_evidence_exposures_workflow_idx
    ON workflow_evidence_exposures(workflow_run_id, workflow_iteration, created_at, id);
