# Goal — Pilot-Ready CLI Project Bootstrap

Turn the completed local encoder-development platform into a practical CLI
product that can start from ordinary local benchmark files without manual
database work or snapshot UUID plumbing.

The existing slice contracts, project-preparation compiler, governed workflow,
and offline acceptance suite are complete. Do not rebuild them. Add a narrow
bootstrap layer that produces their ordinary immutable artifacts.

## User journey

The target journey is:

```text
local development JSONL/CSV + optional sealed JSONL/CSV + one strict manifest
  -> read-only bootstrap preview
  -> atomic, idempotent import and immutable cohort snapshots
  -> existing project preparation and workflow definition
  -> explicit workflow start
  -> generation, training, development evaluation, bounded iteration
  -> explicit sealed assessment and immutable promotion decision
```

A user should not copy UUIDs between commands or edit SQLite. Bootstrap must
print the prepared project identity, source cohort artifact identities, and the
exact next `synth workflow start` command.

## Core requirements

1. Add a strict, versioned bootstrap manifest for local JSONL and CSV cohort
   sources. Paths resolve relative to the manifest. Field mappings support text,
   label, and every arbitrary categorical dimension.
2. Keep the existing snapshot-ID-based preparation manifest and commands intact
   for advanced/manual workflows.
3. `project bootstrap-preview` must read and validate every source without
   writing SQLite. It reports content fingerprints, processed/accepted/rejected
   counts, cohort roles/disclosures, contamination, exact initial allocation,
   finite budgets, and eligibility.
4. Benchmark bootstrap is strict: malformed, rejected, empty, label-incompatible,
   or dimension-incompatible source rows block creation rather than silently
   shrinking evaluation evidence.
5. Each source becomes an ordinary imported benchmark dataset and immutable
   all-test snapshot. Dataset Management remains the owner of snapshot
   membership and provenance.
6. `project bootstrap` must atomically persist source datasets, completed imports,
   accepted source rows, snapshots, the existing preparation bundle, and a small
   bootstrap record. A failure rolls back the whole operation.
7. Idempotency is based on the canonical bootstrap manifest plus source-content
   fingerprints. Repeating unchanged input returns the original artifacts;
   changing file contents creates a distinct fingerprint and never mutates
   history.
8. Sealed sources remain aggregate-only and adaptation-ineligible. Existing
   contamination, evidence-governance, workflow-budget, and acceptance checks
   may not be weakened or bypassed.
9. Credentials remain environment-only. Preview and bootstrap never call a
   generation/advisor provider, train, evaluate, or start a workflow.
10. Add a checked-in, small support-classification pilot with deterministic local
    files, a fake generation backend, bounded budgets, and a start-to-finish CLI
    guide.
11. Add process-level acceptance coverage for preview purity, invalid-source
    rejection, atomic creation, source provenance, idempotent replay, changed
    source detection, workflow handoff, and doctor integrity.
12. Preserve the CLI-first architecture. Do not add or expand HTTP, graphical UI,
    authentication, cloud execution, multiple workers, or autonomous behavior.

## Architecture

- `project-preparation` owns bootstrap manifest/domain contracts and resolves a
  validated source-artifact bundle into its existing preparation compiler.
- `dataset-import` remains the JSONL/CSV format adapter and validator.
- `dataset-core` remains the owner of import/snapshot domain artifacts.
- `synthetic-data-sqlite` owns the single transaction and bootstrap persistence.
- `synthetic-data-cli` resolves local paths, streams sources, assembles adapters,
  and renders results; it does not duplicate import, snapshot, contamination, or
  preparation business rules.

Refactor existing SQLite import/snapshot/preparation writers into reusable
transaction-scoped helpers where necessary. Do not copy their SQL into a new god
module.

## Verification and delivery

Build in coherent commits: specification/contracts, atomic persistence, CLI and
reference pilot, acceptance/docs. After every stage run:

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

The goal is complete only when a fresh database can preview and bootstrap the
checked-in pilot from local files, repeat it without duplicates, start the
resulting workflow, complete the deterministic offline lifecycle, pass doctor,
and pass every repository gate. A real-provider smoke remains explicit,
credential-gated, spend-bounded, and excluded from ordinary verification.
