# Goal — Bounded Benchmark Architect and Evaluation Steward

Implement the Benchmark Architect defined in
`docs/benchmark-architect-spec.md` as the next benchmark-stewardship milestone.

The system must turn immutable task semantics, deployment risks, operator
objectives, aggregate current-benchmark facts, aggregate exposure history, and
bounded research evidence into a deterministic, human-reviewable benchmark
blueprint and immutable acquisition handoff.

## Required capability

- Add a provider-neutral `benchmark-architect-core` with strict brief,
  research-evidence, lifecycle, blueprint, validation, review, freshness, and
  acquisition-handoff contracts.
- Reuse the existing provider-neutral search/fetch boundary and the Pi JSONL
  process, but expose a separate, versioned benchmark-architecture capability
  set with narrow application-owned tools.
- Give Pi no raw benchmark rows, predictions, member identities, source paths,
  or sealed diagnostics. Fetched pages remain untrusted delimited evidence.
- Require explicit, evidence-linked development and optional sealed cohort
  designs, protocols, metrics, thresholds, support, coverage, source diversity,
  risk coverage, and finite renewal policies.
- Validate proposals deterministically against existing evaluation,
  acceptance-contract, qualification, disclosure, and exposure invariants.
- Persist all briefs, runs, tool calls, evidence, proposals, reviews, and
  handoffs immutably in SQLite with migration coverage.
- Provide complete CLI workflows for validate/start/status/watch/cancel/recover,
  evidence/proposal inspection, review, and acquisition handoff.
- Extend provenance and Doctor so missing, stale, reordered, or tampered
  authority/evidence chains fail closed.
- Add checked-in fake examples and an offline process E2E through the actual Pi
  sidecar. Ordinary tests must make no network or paid model call.
- Document how a handoff guides ordinary candidate acquisition and successor
  benchmark preparation without creating a suite, bundle, qualification,
  workflow, or approval itself.

## Freshness and holdout protection

Freshness must be cohort-specific and exposure-aware. Development evidence has
finite age, adaptive-exposure, and workflow-iteration limits. Sealed evidence
remains aggregate-only, adaptation-ineligible, and normally single-use for
acceptance. A stale or exposed cohort requires a successor acquisition handoff
and a newly clean, qualified, approved bundle; an active workflow's immutable
authority is never silently replaced.

## Non-goals

Do not add GUI/TUI/HTTP work, benchmark generation, automatic web-to-dataset
copying, automatic labeling, automatic approval, hidden holdout inspection,
threshold tuning against sealed results, distributed execution, multiple
workers, semantic deduplication, encoder training changes, reinforcement
learning, bandits, or an unbounded agent loop.

## Completion criterion

A fresh offline CLI run can validate and persist a safe brief, execute a finite
scripted Pi research session, record source-backed risk evidence, preview and
submit a valid benchmark blueprint, reject unsafe or underpowered variants,
record append-only human review, produce exactly one immutable acquisition
handoff, inspect its cohort/freshness requirements, and reproduce the complete
provenance chain through Doctor. Cancellation, interrupted recovery,
idempotency, migration upgrade, tampering, sealed-data non-disclosure, and
legacy workflows remain covered and green under all standard Rust and Pi gates.
