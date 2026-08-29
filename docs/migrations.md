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
