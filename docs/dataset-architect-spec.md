# Dataset Architect specification

## Objective

Turn explicit dataset facts into a bounded, inspectable recommendation for
what synthetic examples to generate and in what proportions.

```text
dataset + semantics + approved authenticity + coverage + operator priorities
  + optional governed development diagnostics
  -> bounded Pi exploration through application-owned read-only tools
  -> explicit per-cell targets and generation-strategy directives
  -> deterministic allocation validation
  -> append-only human review
  -> immutable application to one normal Slice 1 generation plan
```

The authenticity researcher answers how real inputs look. The Dataset
Architect answers which regions of the declared generation space deserve rows,
which controlled variations should appear inside those cells, and why.

## Authority boundary

Pi is an advisory search process over already-authorized local facts. It may
not mutate a dataset, create a plan, generate rows, call a generation backend,
start training, or decide approval. The host exposes only narrow tools for
inspecting pinned facts, previewing candidate allocations, estimating declared
generation costs, submitting a proposal, and finishing the run.

The existing `workflow-core` initial allocator is the sole feasibility
authority. Every proposal explicitly names every generation cell and is
replayed through its `Explicit` policy with the pinned coverage and constraints.
Infeasible, incomplete, over-budget, or stale proposals cannot be applied.

## Inputs

One immutable resolved brief pins:

- the complete dataset definition and fingerprint;
- exact desired total rows, optional reserve, current accepted coverage, and
  exact cell constraints;
- ordered operator priorities and relative weights;
- optional resolved semantic and approved authenticity contexts;
- an optional complete diagnostic contract with active cohort role evidence;
- a declared token/cost estimation model;
- finite model-turn, tool-call, allocation-preview, token, cost, and wall-clock
  budgets;
- Pi provider/model identity and an API-key environment-variable name only.

Diagnostics are aggregate development or diagnostic evidence. The core rejects
sealed acceptance, training, external-benchmark, retired, legacy, or
fingerprint-invalid diagnostic context. No row text, prediction list, sealed
slice, or holdout example belongs in an architect prompt.

## Proposal

A proposal contains:

- an explicit target and rationale for every cell;
- confidence and expected-benefit categories per allocation;
- scoped generation-strategy directives such as hard negatives, boundary
  cases, ambiguity, realistic noise, rare patterns, channel variation, and
  length variation;
- a bounded approximate share and instructions for each directive;
- trade-offs, uncertainty, deterministic feasibility output, pinned coverage,
  and a reproducible cost estimate.

Strategies guide prompt construction only for matching cells. They never alter
trusted labels or dimensions and do not supersede semantic or authenticity
context.

## Lifecycle and review

Runs are durable and finite: `queued`, `running`, `awaiting_review`, `failed`,
or `cancelled`. Tool intent and outcome, usage, stop reason, proposal, review,
and application are persisted facts. Cancellation prevents new work. Recovery
does not replay an unknown paid model turn.

Reviews are append-only `approve`, `reject`, or `request_revision` decisions.
Application requires the latest review to approve the exact proposal and must
reproduce the current accepted-coverage fingerprint. It atomically creates one
ordinary unequal generation plan plus its immutable strategy binding. Repeating
the same application is idempotent.

## CLI-first acceptance

The CLI supports brief validation, start/status/watch/cancel/recover, proposal
inspection, append-only review, application, and strategy-context inspection.
Ordinary acceptance uses scripted Pi turns and a fresh local database. It must
prove that an approved proposal becomes a normal plan, generation pins the
strategy context, only matching-cell instructions enter prompts, and no
architect/model call occurs during generation. No HTTP or graphical UI is part
of this capability.

The concrete flow is:

```text
synth dataset create ...
synth architect brief-validate examples/architect/support-architect-brief.json
synth architect start examples/architect/support-architect-brief.json \
  --script examples/architect/support-architect-scripted-turns.json
synth architect proposal <RUN_ID>
synth architect review <PROPOSAL_ID> --approve --reason "approved budget"
synth architect apply <PROPOSAL_ID>
synth architect context <PLAN_ID>
synth generate <PLAN_ID> --backend fake
synth job execution <JOB_ID>
synth job prompt <JOB_ID> --cell-index 0 --requested-count 1
```

The checked-in script exercises the actual JSONL Pi process with a fake model,
not an alternate in-process architect. A real provider uses the same command
without `--script`; its brief names the provider, model, finite budgets, and an
environment-variable name containing the credential.

If a brief names `analysis_report_id`, the CLI reconstructs a normalized
diagnostic contract from persisted analysis facts and reviews. It requires the
report's exact cohort to have one current active `development` or `diagnostic`
role and appends a `dataset_architecture`/`slices` adaptive exposure. The brief
file cannot inject a fabricated diagnostic contract and a sealed cohort cannot
cross this boundary.
