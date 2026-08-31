# SQLite migrations

Migrations are ordered, append-only SQL files under `migrations/` and are
applied automatically when the SQLite store connects.

- `0001`–`0006`: generation through optimization slice storage.
- `0007_dataset_imports`: imports, unified accepted source rows, snapshot source
  provenance, and repaired dependent evaluation foreign keys.
- `0008_workflow_recovery`: process leases and interruption history.
- `0009_artifact_fingerprints`: snapshot/evaluation/analysis/proposal fingerprints
  and persisted resolved project configurations.
- `0010_cli_query_indexes`: indexes for actual CLI filters and stable pagination.
- `0011_registered_encoders`: validated local encoder identities, artifact
  paths, checksums, and display metadata.
- `0012_training_batch_progress`: durable epoch/batch/example/loss/rate progress.
- `0013_transformer_provenance`: base-model and parent-checkpoint edges plus
  backend configuration identity and checkpoint sizes.
- `0014_transformer_run_configuration`: exact normalized transformer settings
  on every training run.
- `0015_trustworthy_evaluation_runs`: immutable protocols, source identity,
  durable progress, cancellation, and repaired downstream foreign keys.
- `0016_evaluation_evidence_reports`: paired-comparison and advisory-selection
  reports with complete candidate provenance.
- `0017_actionable_error_analysis`: resolved analysis protocols/source
  identities, normalized ranked findings, bounded prediction evidence links,
  optional comparison provenance, and append-only finding reviews. Historical
  report JSON and nullable legacy columns remain readable.
- `0018_constrained_optimization`: normalized optimization protocols and source
  identities, decision cells, allocation feasibility/issues, and immutable
  data recommendations. Historical proposal JSON and nullable Slice 6.1
  columns remain readable.
- `0019_optimization_scenarios_reviews`: immutable scenario groups and pairwise
  sensitivity rows plus append-only proposal reviews and normalized partial
  selections.
- `0020_safe_proposal_applications`: approval, selected-recommendation, and
  verified-coverage provenance on idempotent plan applications. Historical
  legacy applications keep nullable approval fields.
- `0021_optimization_proposal_rebases`: immutable lineage from a refreshed
  proposal to its stale predecessor, including both accepted-coverage
  fingerprints.
- `0022_optimization_training_candidates`: exact finite typed configuration
  spaces, bounded immutable candidates, baseline run/checkpoint provenance,
  and normalized training-candidate review selections.
- `0023_optimization_campaigns`: immutable campaign decisions, append-only
  compatible artifact links, and one immutable deterministic outcome
  assessment per campaign.
- `0024_optimization_review_recommendations`: normalized advisory label/schema
  review recommendations and their append-only explicit acceptance selections.
- `0025_optimization_decision_evidence`: complete bounded evidence artifacts
  and normalized accepted coverage required to reproduce scores, allocations,
  stale checks, and proposal fingerprints offline.
- `0026_initial_allocations`: immutable exact initial-budget allocation artifacts,
  normalized totals/fingerprints, and a unique atomic link to the ordinary
  generation plan produced from their complete absolute cell targets.
- `0027_cohort_governance`: immutable evaluation cohorts, append-only role
  transitions, and append-only evidence exposures with normalized policy fields
  and reproducible fingerprints.
- `0028_contamination_reports`: immutable cross-cohort leakage reports,
  normalized cohort membership, and one explicit append-only override per
  blocked report.
- `0029_benchmark_suites`: immutable suite definitions and normalized cohort
  bindings plus deterministic acceptance assessments keyed by suite,
  checkpoint, state, and reproducible artifact fingerprints.
- `0030_workflow_runs`: immutable bounded workflow definitions, durable current
  run state, fingerprint-linked append-only stage attempts, and normalized
  cross-slice artifact links with optimistic concurrency support.
- `0031_advisory_assessments`: immutable advisory assessments with normalized
  workflow/report identity, versioned prompt, validation, provider metadata,
  and observable token usage.
- `0032_workflow_approvals`: immutable human and preauthorization decisions
  binding exact proposals, reviews, recommendation selections, and envelopes.
- `0033_workflow_stop_decisions`: deterministic iteration stop/continue facts
  linked to acceptance and paired-comparison evidence.
- `0034_model_promotions`: immutable promote/reject records linking checkpoint,
  snapshot, development and sealed assessments, suites, and policy identity.
- `0035_encoder_workflow_recovery`: expands process leases and interruption
  records to the durable cross-slice encoder workflow while preserving legacy
  generation, training, and evaluation recovery rows.
- `0036_project_preparations`: immutable manifest-fingerprinted project
  preparation summaries linking the atomically created dataset configuration,
  benchmark suites, and workflow definition.
- `0037_project_bootstraps`: content-fingerprinted local-source bootstrap
  summaries linking atomically created import/snapshot artifacts to one existing
  project preparation.
- `0038_semantic_catalog`: immutable semantic profile versions, append-only
  dataset binding decisions, and exact generation-job semantic assignments.
- `0039_generation_execution_ledger`: immutable generation execution
  specifications, append-only provider request attempts, and transactional
  normalized-text claims that preserve legacy duplicates while rejecting new
  cross-process duplicates.
- `0040_hybrid_row_construction`: typed arbitrary generated/source/snapshot row
  fields and optional per-field construction provenance, with empty defaults
  for every legacy row.
- `0041_authenticity_research`: immutable research briefs/runs, append-only tool
  calls, evidence and claims, versioned authenticity profiles, append-only
  reviews, and approval-gated dataset bindings.
- `0042_generation_authenticity_contexts`: immutable job-specific authenticity
  assignments linking a generation execution to its approved profile and
  binding; the execution JSON pins the resolved context and source-novelty
  guard fingerprints.
- `0043_dataset_architect`: immutable architect briefs and runs, durable tool
  calls, reviewable allocation proposals, append-only reviews, and atomic
  generation-plan strategy applications.
- `0044_dataset_quality`: immutable quality plans and complete source manifests,
  the exact normalized evaluator guidance pinned at plan creation, exact
  evaluator requests paired one-to-one with durable attempts, row assessments,
  reproducible invalid-output-aware reports, append-only row/manifest review
  chains, complete curation selections, and an atomic link between an approved
  manifest and its ordinary dataset snapshot.
- `0045_dataset_architecture_exposures`: adds dataset-architecture use to the
  append-only governed evidence-exposure ledger while preserving prior rows and
  foreign keys.
- `0046_benchmark_bundles`: immutable development/optional sealed suite and
  strict global-contamination authority with normalized foreign keys, a unique
  artifact fingerprint, distinct-suite enforcement, and indexed nullable links
  from workflow definitions and project preparations. Existing records retain
  `NULL` links so their historical JSON, status, and provenance remain readable.
  Those legacy definitions and runs are intentionally non-executable: create a
  new bundle-backed definition before starting or advancing workflow work.

SQLite table rebuilds require special care: dependent foreign keys may be
rewritten to a temporary table name during `ALTER TABLE ... RENAME`. Rebuild
dependent tables in the same migration and validate with
`PRAGMA foreign_key_check`.

Use a temporary database for migration tests. Operational verification is:

```text
synth doctor
```

Do not delete or rewrite an applied migration. Add the next numbered migration
and keep historical artifact/provenance rows readable.
