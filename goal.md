# Goal — Bounded Agentic Dataset Architect

Build a high-impact decision layer between dataset definition/authenticity
research and synthetic generation. Use Pi agentically to recommend what data
to generate, while retaining deterministic host authority, explicit human
approval, immutable provenance, and strict evaluation-evidence isolation.

Do not add a graphical UI, HTTP API, automatic training, autonomous closed-loop
optimization, multiple agents, distributed workers, or sealed-holdout access.

## Product flow

```text
dataset definition
  + current accepted coverage
  + semantic definitions
  + optional approved authenticity profile
  + operator priorities and hard constraints
  + optional governed aggregate development diagnostics
    -> bounded Pi Dataset Architect run
    -> visible plan and deterministic allocation previews
    -> explicit every-cell allocation + scoped generation strategies + cost
    -> operator inspection and append-only approval
    -> immutable ordinary GenerationPlan + normalized strategy context
    -> normal generation job pins and consumes that context
```

The architect advises. It must never generate rows, create or approve its own
plan, call a generation backend, start training/evaluation, mutate datasets, or
read sealed acceptance evidence.

## Required architecture

- Rust core owns briefs, finite budgets, lifecycle, tool-call ledger, proposal
  validation, review, application, fingerprints, and normalized handoff.
- Pi remains behind the product-neutral `agent-runtime-core` process contract.
  Pi/provider types do not leak into domain, SQLite, planning, prompting, or CLI
  contracts.
- Expose only application-owned tools for pinned-fact inspection, deterministic
  allocation previews, declared cost estimates, proposal submission, and finish.
  Never expose shell, filesystem, database, arbitrary network, generation,
  training, evaluation, approval, or sealed-evidence tools.
- The existing deterministic allocator remains the sole feasibility authority.
  Every proposal explicitly allocates every Cartesian cell and is replayed as
  an ordinary explicit allocation.
- Generation strategy guidance is provider-neutral, scoped to exact matching
  cells, and cannot alter trusted labels, dimensions, deterministic fields, or
  validation rules.
- Generation jobs pin the approved strategy fingerprint alongside semantics,
  authenticity, prompt template, construction plan, backend, and parameters.
  Generation performs no architect/Pi calls.

## Brief and proposal

A strict resolved brief pins the full dataset and fingerprint, exact total and
reserve, persisted accepted coverage, hard cell constraints, weighted operator
priorities, optional semantic/authenticity contexts, optional eligible aggregate
diagnostics, declared generation cost assumptions, provider/model identity,
credential environment-variable name, and finite turn/tool/preview/token/cost/
wall-clock limits.

The proposal contains an explicit target, rationale, confidence, and expected
benefits for every cell; scoped strategies such as boundary cases, hard
negatives, ambiguity, realistic noise, rare patterns, channel variation, and
length variation; approximate shares and instructions; tradeoffs, uncertainty,
the deterministic feasibility result, pinned coverage, and cost estimate.

## Governance and durability

- Only current active development or diagnostic cohort evidence is eligible.
  Reconstruct diagnostics from persisted analysis facts; do not accept an
  arbitrary diagnostic blob from the brief file.
- Record a slices-level adaptive `dataset_architecture` evidence exposure.
- Reject sealed, external-benchmark, training, retired, legacy, mismatched, or
  fingerprint-invalid evidence before it enters an agent prompt.
- Persist queued/running/awaiting-review/failed/cancelled runs, every tool intent
  and outcome, usage, stop reason, proposal, reviews, application, plan, and job
  assignment. Recovery does not replay uncertain model/tool calls.
- Reviews are append-only. Application requires the latest approval for the
  exact proposal and fails when accepted coverage differs from the pinned
  proposal coverage. Repeated successful application is idempotent.

## CLI and acceptance

Provide:

```text
synth architect brief-validate <FILE>
synth architect start <FILE> [--script <FILE>]
synth architect status|watch|proposal <RUN_ID>
synth architect cancel|recover <RUN_ID>
synth architect review <PROPOSAL_ID> --approve|--reject|--request-revision --reason <TEXT>
synth architect apply <PROPOSAL_ID>
synth architect context <PLAN_ID>
```

Ordinary acceptance must use a fresh local database, checked-in brief, scripted
Pi turns, actual JSONL process boundary, and fake generation backend. Prove the
proposal is deterministically validated; approval and fresh coverage gate
application; an ordinary generation job pins the strategy; only matching-cell
instructions enter reconstructed prompts; generation adds no architect tool
calls; tampering is reported by `doctor`; older plans without strategy remain
compatible; and all Rust/Pi quality gates pass.

Implement and commit coherent stages. Follow `AGENTS.md`,
`docs/dataset-architect-spec.md`, `docs/architecture.md`, and
`docs/development.md`.
