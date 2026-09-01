# Goal — Governed Smart-Generation Workflow

Integrate the completed Generation Quality Supervisor into the primary finite
encoder-development workflow as an optional first-class generation mode.

The operator should configure outcomes, not author the supervisor's detailed
contract by hand. One strict project/workflow manifest must be able to express:

- quality level: `economical`, `balanced`, or `strict`;
- authenticity importance and diversity importance;
- the maximum number of prompt-repair attempts;
- finite row, request, token, duration, retry, and optional cost limits;
- manual review or an exact finite pre-authorization for prompt repairs; and
- separate non-secret generator and evaluator profiles, including distinct
  endpoints/models/API-key environment-variable names when desired.

Compile those controls deterministically into the complete immutable
`GenerationQualityContract`. Runtime execution must consume only the resolved
contract. Credentials remain environment-only.

## Required journey

```text
dataset architecture / initial allocation
  -> supervised generation
  -> deterministic quality windows and drift decisions
  -> pause for bounded diagnosis, repair review, or canary when necessary
  -> immutable directly-qualified handoff
  -> ordinary curation proposal
  -> explicit exact-manifest approval
  -> ordinary immutable snapshot
  -> training-to-benchmark contamination check
  -> training, development evaluation, analysis, and bounded iteration
```

Apply the same supervised boundary to an approved iteration data diff. Do not
copy generation, qualification, curation, snapshot, or training logic into the
workflow layer.

## Compatibility and authority

- Supervision is optional. A definition that omits it retains the existing
  unsupervised stage graph, fingerprints, and behavior.
- The legacy `quality_gate` remains independently useful. Reject definitions
  that configure it together with generation supervision because supervision
  already produces full-population qualification evidence.
- A workflow stage reserves and links the exact supervisor run before work.
  Cancellation and recovery target that link.
- A supervisor pause becomes `awaiting_user` with exact evidence and commands.
  Resuming before the required diagnosis/review/canary action is an idempotent
  no-op. Resuming after the boundary is cleared continues the same run.
- Completion must call the existing zero-I/O qualification finalizer and then
  use the existing Dataset Qualification curation review/manifest path.
- Neither the supervisor nor the workflow may approve a curation manifest or
  create an unqualified snapshot.
- Workflow budgets and supervisor budgets must both be enforced and reconciled
  from persisted usage.

## Delivery

Implement core contracts and preset compilation, strict manifest preparation,
workflow lifecycle and child links, SQLite compatibility/integrity,
provenance/Doctor, CLI status and recovery, checked-in examples, documentation,
and an offline fake end-to-end test that reaches snapshot, training, and
development evaluation through the supervised path.

Do not add GUI, TUI, HTTP/API work, distributed execution, semantic dedup,
reinforcement learning, bandits, or an unbounded agent loop.

Follow `AGENTS.md`, `docs/platform-spec.md`,
`docs/generation-quality-supervisor-spec.md`,
`docs/workflow-governance-spec.md`, `docs/architecture.md`, and
`docs/development.md`. After each coherent component, run the standard Rust
quality gates and commit it.

## Completion criterion

This goal is complete when a fresh offline CLI project can be prepared from a
small outcome-oriented manifest, start one governed workflow, execute initial
and iterative generation through exact supervisor runs, pause and resume safely
at repair boundaries, hand only directly qualified rows to explicit curation,
create the qualified snapshot, pass the normal training firewall, train and
evaluate, and reproduce the complete evidence chain through status, provenance,
Doctor, and migration-upgrade tests. Legacy unsupervised workflows must remain
green.
