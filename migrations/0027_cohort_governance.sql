CREATE TABLE workflow_evaluation_cohorts (
    id TEXT PRIMARY KEY NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES dataset_snapshots(id) ON DELETE RESTRICT,
    split TEXT NOT NULL CHECK (split IN ('train', 'validation', 'test')),
    origin TEXT NOT NULL CHECK (origin IN ('internal_snapshot', 'external_benchmark')),
    name TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX workflow_evaluation_cohorts_snapshot_created_idx
    ON workflow_evaluation_cohorts(snapshot_id, created_at DESC, id);

CREATE TABLE workflow_cohort_role_decisions (
    id TEXT PRIMARY KEY NOT NULL,
    cohort_id TEXT NOT NULL REFERENCES workflow_evaluation_cohorts(id) ON DELETE RESTRICT,
    sequence INTEGER NOT NULL,
    role TEXT NOT NULL CHECK (
        role IN ('training', 'development', 'diagnostic', 'sealed_acceptance', 'external_benchmark')
    ),
    disposition TEXT NOT NULL CHECK (disposition IN ('active', 'retired')),
    predecessor_id TEXT REFERENCES workflow_cohort_role_decisions(id) ON DELETE RESTRICT,
    fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (cohort_id, sequence),
    UNIQUE (predecessor_id)
);

CREATE INDEX workflow_cohort_role_decisions_current_idx
    ON workflow_cohort_role_decisions(cohort_id, sequence DESC);

CREATE TABLE workflow_evidence_exposures (
    id TEXT PRIMARY KEY NOT NULL,
    cohort_id TEXT NOT NULL REFERENCES workflow_evaluation_cohorts(id) ON DELETE RESTRICT,
    role_decision_id TEXT NOT NULL REFERENCES workflow_cohort_role_decisions(id) ON DELETE RESTRICT,
    evaluation_run_id TEXT REFERENCES evaluation_runs(id) ON DELETE RESTRICT,
    workflow_run_id TEXT,
    workflow_iteration INTEGER,
    purpose TEXT NOT NULL CHECK (
        purpose IN (
            'training', 'development_evaluation', 'diagnosis', 'comparison',
            'acceptance', 'manual_inspection', 'advisor', 'optimization'
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

CREATE INDEX workflow_evidence_exposures_cohort_created_idx
    ON workflow_evidence_exposures(cohort_id, created_at, id);

CREATE INDEX workflow_evidence_exposures_evaluation_idx
    ON workflow_evidence_exposures(evaluation_run_id);
