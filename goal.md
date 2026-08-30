# Goal — Dataset Qualification and Curation

Build a high-trust quality boundary between structurally accepted source rows
and training snapshots. A replaceable evaluator must produce bounded semantic-
quality evidence; deterministic policy and explicit review must produce an
immutable curation manifest that the ordinary snapshot builder can consume.

Do not add a graphical UI, HTTP API, automatic relabeling, source-row mutation,
semantic deduplication, automatic regeneration, distributed workers, or sealed-
holdout access. Follow `docs/dataset-quality-spec.md` as the authoritative
capability specification.

## Product flow

```text
accepted generated/imported rows
  -> immutable row-complete audit plan
  -> bounded replaceable quality evaluator
  -> blind label/dimension/authenticity/risk assessments
  -> deterministic qualified/borderline/quarantined verdicts
  -> append-only row review and complete curation proposal
  -> explicit manifest approval
  -> immutable qualified snapshot linked to the manifest
```

The evaluator supplies evidence. It never decides membership, rewrites a row,
approves a manifest, starts another slice, or reads evaluation evidence.

## Required architecture

- `dataset-quality-core` owns source-set manifests, explicit integer policies,
  evaluator request/response validation, verdicts, reviews, curation, and
  fingerprints. Provider and SQLite types never cross its boundary.
- Audit plans pin every source-row fingerprint plus exact semantic and optional
  authenticity guidance. New rows require a new plan.
- A deterministic fake and an OpenAI-compatible adapter implement the project-
  owned evaluator port. Prompt policy remains outside provider transport.
- One durable local runner owns batching, attempts, retry, cancellation,
  interruption, usage, and reconciled counters under finite persisted budgets.
- Every manifest decides every pinned row. Unevaluated rows are excluded; human
  inclusion overrides are append-only and require a reason.
- Applying a manifest atomically persists an ordinary snapshot plus a curation
  application. Historical snapshot shapes and fingerprints remain unchanged.

## Evidence and verdicts

Assessments blindly rank every known label and every allowed value for each row
dimension. They also score optional authenticity adherence, leakage risk,
shortcut risk, and evaluator confidence in basis points. The host compares the
blind result with pinned source facts and derives verdicts from the immutable
policy. Evaluator output cannot contain replacement content or a membership
decision.

## Governance and durability

- Persist plans, runs, evaluator attempts, normalized assessments, immutable
  reports, row reviews, curation proposals, manifest reviews, approved
  manifests, applications, and snapshot links.
- External calls are recorded before I/O and uncertain calls consume retry
  budget. Success plus assessments plus counters commits atomically.
- Sealed and external benchmark contents are categorically ineligible.
- Reviews bind immutable reports/proposals, never a mutable run projection.
- `doctor` and provenance verify every fingerprint, counter, review chain,
  source-row link, selection entry, and qualified snapshot application.

## CLI and acceptance

Implement the complete CLI in `docs/dataset-quality-spec.md`. Ordinary
acceptance uses a fresh database, fake generated/imported sources, fake
evaluator, actual CLI process boundary, row review, manifest approval, qualified
snapshot, provenance, export, and doctor. Prove unassessed rows never enter the
snapshot, tampering is detected, legacy snapshots remain compatible, and all
Rust quality gates pass.

Implement and commit coherent stages. Follow `AGENTS.md`,
`docs/dataset-quality-spec.md`, `docs/architecture.md`, and
`docs/development.md`.
