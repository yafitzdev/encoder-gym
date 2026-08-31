# Generation Quality Supervisor specification

## Purpose

Prevent a synthetic-data run from producing a large population of technically
valid but strategically weak examples. The supervisor composes ordinary Slice 1
generation jobs with immutable Dataset Qualification evidence, detects immediate
contract violations and later quality drift, and permits a bounded Pi advisor to
propose a prompt-guidance repair. Deterministic policy, not Pi or the generator,
decides whether generation pauses and whether a canary permits a revision to
continue.

The capability is optional. Existing generation plans and jobs remain complete,
independently useful, and backward compatible.

## Product boundary

```text
dataset + plan + accepted coverage
  + pinned semantics/authenticity/strategy/construction
  + generator/evaluator identities
  -> immutable generation-quality contract
  -> finite generation segment
  -> immutable row-quality and batch observations
  -> deterministic scoped decision
       healthy               -> next finite segment
       insufficient evidence -> collect only the authorized evidence
       weak or drifting       -> pause affected scope
  -> optional bounded Pi diagnosis
  -> immutable prompt-guidance revision proposal
  -> append-only review or exact finite pre-authorization
  -> independently assessed canary segment
       pass -> activate the revision for remaining absolute coverage
       fail -> reject, retry within budget, or escalate
```

The supervisor does not own generation planning, provider transport, source-row
acceptance, semantic definitions, quality scoring, dataset snapshots, training,
evaluation, or optimization. It links their normal immutable contracts.

## Terminology

`Immediate weakness` means the first eligible observation window already fails
the approved contract. `Longitudinal drift` means a later window degrades from a
qualified baseline or exits an approved distribution. Both produce the same
class of generation-quality evidence while retaining different issue codes.

A `segment` is one bounded ordinary generation execution under one exact prompt
guidance version. A `canary` is a small segment whose accepted rows remain
quarantined from qualified supervisor coverage until the deterministic canary
decision passes. A failed canary remains immutable evidence.

## Dependency direction

`generation-supervisor-core` owns provider-neutral contracts, pure aggregation,
deterministic decisions, strategy assignment, revision/review rules, lifecycle,
finite budgets, and the ports required from persistence and child executors. It
may depend on narrow immutable contracts from `generation-core`,
`dataset-quality-core`, `research-core`, `semantic-catalog`, and `artifact-core`.
It imports no SQLite, CLI, Pi, HTTP, SDK, or provider types.

`generation-supervisor-runner` owns the bounded Pi loop and application-owned
tool dispatch. It depends on `generation-supervisor-core` and
`agent-runtime-core`. The existing Pi process adapter implements the generic
runtime port; Pi types never cross into either core.

The CLI assembles generation, quality, supervisor, Pi, and SQLite adapters.
`generation-sqlite` implements feature-owned supervisor persistence and atomic
child reservations. The legacy HTTP server is unchanged.

## Immutable quality contract

One `GenerationQualityContract` pins:

- dataset definition and generation plan identities/fingerprints;
- the exact accepted-coverage fingerprint at contract creation;
- optional semantic, authenticity, construction, and strategy context
  identities/fingerprints;
- exact generator and evaluator backend/model/protocol identities and their
  declared relationship;
- enabled row and batch criteria with explicit thresholds;
- initial-canary and rolling-window sizes, minimum evidence, baseline policy,
  and affected scope;
- finite generation segment, revision, canary, evaluator request/attempt,
  model-turn, tool-call, token, duration, retry, and optional cost ceilings;
- explicit-review or finite pre-authorization policy;
- protected prompt fields and the only guidance patches a revision may change;
- schema/protocol version, creation time, and reproducible fingerprint.

Presets may help author a contract but persisted execution consumes only the
fully resolved values. Unknown fields, non-finite thresholds, inconsistent
ceilings, an evaluator relationship that falsely claims independence, missing
context fingerprints, or a contract that cannot collect its minimum evidence
within budget are rejected before any child starts.

The contract may require a property only when its expected meaning or target is
pinned. Missing authenticity, difficulty, or strategy guidance produces an
explicit unavailable criterion rather than an invented score or target.

## Strategy assignments

Approved per-cell strategy shares are compiled deterministically before a
segment starts. For every cell, the compiler converts basis-point shares into
integer assignment targets with stable largest-remainder allocation. The
unassigned remainder is the ordinary/default strategy. Overlapping directives
must be explicitly declared compatible; V1 otherwise assigns at most one
directive to a row.

Every assignment pins the plan, strategy context, cell, directive, row sequence
or deterministic seed, expected criterion, instructions, and fingerprint.
Prompt construction receives only the assignment for the requested row or
small batch. The backend never chooses trusted label, dimension, or assignment
identity.

Persisted facts derive strategy coverage:

```text
target / attempted / structurally accepted / assessed / qualified /
rejected / quarantined / remaining
```

Approximate shares in an architect proposal therefore become inspectable
execution targets instead of a free-form request to an LLM.

## Observation model

Observations are small immutable facts. Deterministic aggregators compose them;
there is no monolithic quality function.

### Row observations

One row observation pins the source/generated row fingerprint, generation job
and attempt, prompt version, optional strategy assignment, evaluator assessment
and attempt, and normalized criterion outcomes. Supported criteria include:

- structural/schema validity and non-empty text;
- trusted cell identity;
- exact/normalized duplication;
- assigned-label score and competing-label margin;
- declared dimension adherence;
- expected difficulty/challenge adherence when pinned;
- authenticity adherence when pinned;
- assigned strategy adherence when pinned;
- label leakage and shortcut/artifact risk;
- evaluator confidence and bounded issue codes.

The generator cannot be represented as an independent reviewer of its own rows.
If generator and evaluator share provider, model, protocol, or other declared
lineage, the relationship is visible and policy may reject or downgrade it.

### Batch observations

A batch window is reconstructed from exact row observations and contains:

- attempted, accepted, assessed, qualified, borderline, quarantined, invalid,
  and duplicate counts;
- per-cell and per-strategy counts and rates;
- deterministic lexical/template repetition indicators;
- text-length and other pinned measurable distributions;
- difficulty, authenticity, and strategy outcome distributions;
- label/cell shortcut concentration;
- missing required rare, boundary, ambiguity, noise, or channel patterns;
- exact member IDs/fingerprints in a separately protected evidence manifest;
- a redacted aggregate artifact safe for the Pi tools;
- a reproducible fingerprint.

No row is inferred qualified from a sampled neighbor. Canary and monitoring
windows may be bounded, but only directly assessed rows may enter qualified
supervisor coverage.

## Deterministic decisions

Policy evaluates each affected cell/strategy independently before any global
decision. A systemic pause requires evidence that the failure crosses the
contract's explicit global rule.

Supported decision states are semantically equivalent to:

- `healthy`;
- `insufficient_evidence`;
- `pause_for_diagnosis`;
- `awaiting_review`;
- `revision_canary_required`;
- `revision_passed`;
- `revision_failed`;
- `escalate`;
- `completed`;
- `failed`;
- `cancelled`.

Immediate weakness compares the first eligible window with absolute contract
thresholds. Longitudinal drift compares a later window with both absolute
thresholds and a pinned qualified baseline using explicit integer or
fixed-precision deltas. Insufficient support cannot become drift merely because
the point estimate looks poor.

The decision records normalized issues, observed and required values, exact
scope, evidence fingerprints, whether the failure is immediate or temporal,
and the next permitted actions. Pi cannot create or rewrite a decision.

## Bounded Pi diagnosis

Pi is invoked only after a deterministic pause that authorizes diagnosis. One
versioned capability set exposes tools equivalent to:

- `inspect_quality_contract`;
- `inspect_quality_window`;
- `inspect_failure_breakdown`;
- `inspect_current_prompt_guidance`;
- `preview_prompt_revision`;
- `submit_prompt_revision`;
- `finish_supervision`.

The host validates every call against the run, paused scope, immutable evidence,
cancellation flag, and remaining budget. Pi receives no shell, filesystem,
database, arbitrary network, generation backend, evaluator backend, raw secret,
or sealed evaluation evidence. Raw generated rows are excluded from V1 Pi tools;
the agent works from bounded failure summaries and protected guidance.

Every model and tool intent is durable before I/O. Outcomes are succeeded,
failed, or interrupted. An interrupted external call consumes the applicable
attempt budget and is never silently replayed under the same identity.

Diagnosis names one of a bounded set of likely causes: prompt guidance,
semantic conflict, dimension/strategy conflict, generator inadequacy,
evaluator disagreement, insufficient evidence, repetition/mode collapse,
authenticity failure, strategy failure, or not safely repairable. The agent may
escalate without proposing a revision.

## Prompt-guidance revisions

The base system prompt, output schema, trusted target label/dimensions,
construction graph, semantic authority, quality thresholds, budgets, and safety
instructions are protected. Pi proposes only a bounded patch to a versioned
generation-guidance slot rendered by the existing prompt builder.

One immutable revision proposal pins the parent prompt/guidance fingerprints,
triggering decision/window, affected cells/strategies, diagnosis, normalized
instruction patch, expected measurable improvement, protected-field proof,
runtime/model/tool-protocol identity, usage, revision sequence, and fingerprint.

Reviews are append-only approve, reject, or request-revision decisions. An
approval is valid only for the exact proposal fingerprint. Optional unattended
activation requires the exact revision kind, scope, thresholds, and remaining
budgets to fit a persisted finite pre-authorization envelope.

A revision never mutates an active or historical generation job. After approval
it creates a new prompt version and a new canary child. A canary passes only when
the deterministic contract finds sufficient directly assessed evidence and no
blocking issue. Failed-canary rows never count toward qualified coverage.

## Durable orchestration

A supervisor run owns a finite append-only sequence:

```text
generation segment -> quality audit/window -> deterministic decision
  -> optional diagnosis -> revision review -> canary -> activation
  -> next generation segment
```

Before external work, the supervisor atomically reserves an exact child kind,
logical input key, child ID, and attempt. Generation segments use ordinary
absolute-coverage plans/jobs; quality observations use ordinary immutable audit
evidence. The supervisor stores links, not copies of child business logic.

Parent cancellation is persisted before new reservations close, then forwarded
only to linked running children. Recovery reuses a resumable interrupted child
or records an explicit replacement for a non-resumable child. It cannot infer
approval, reactivate a failed revision, duplicate qualified coverage, replay an
uncertain paid call, or reset consumed budgets.

Accepted rows from an earlier prompt version remain when directly qualified by
the contract. Invalid, weak, quarantined, and failed-canary rows remain immutable
evidence and do not count toward qualified supervisor coverage. Nothing is
silently deleted, relabeled, or rewritten.

## Post-training evidence boundary

A later remediation phase may supply approved abstract development-error
archetypes and strategy proposals through existing exposure governance. This
capability does not run training/evaluation or inspect predictions.

Sealed-acceptance row content, predictions, findings, slices, and acceptance
evidence are structurally ineligible for supervisor contracts, Pi requests,
prompt revisions, and canary decisions.

## Persistence and provenance

SQLite persists contracts, runs, finite usage, segment/child links, strategy
assignments and coverage, row/window observations, decisions, Pi calls/tool
calls, diagnoses, revision proposals/reviews/versions, canary evidence and
activation, escalation, cancellation, and recovery facts.

Every accepted supervised row traces through:

```text
row -> generation attempt/job -> prompt version -> optional strategy assignment
    -> quality assessment/window -> supervisor decision/run -> quality contract
```

Doctor deeply verifies fingerprints, append-only chains, exact child ownership,
counter reconciliation, strategy conservation, prompt protection, approval,
canary activation, qualified coverage, and sealed-evidence exclusion. Tampering
fails closed.

## CLI contract

The CLI supports:

```text
synth supervisor contract-preview|contract-create|contract-show ...
synth supervisor start|status|watch|cancel|recover ...
synth supervisor windows|issues|strategy-coverage ...
synth supervisor revision-show|revision-review ...
synth supervisor canary-status ...
synth provenance generation-supervisor-run <RUN_ID>
synth doctor
```

Human output explains why a scope paused, what evidence changed, what the
revision may alter, and why a canary passed or failed. JSON output remains one
stable value on stdout. No TUI, HTTP, or graphical UI is added.

## Verification

Ordinary verification uses deterministic generation/evaluator fakes and
scripted Pi turns. It covers immediate trivial-row failure, later repetition
drift, leakage, shortcut concentration, authenticity, difficulty and strategy
failures, scoped pausing, minimum support, strategy count conservation,
generator/evaluator relationship, protected prompt fields, immutable revision
history, failed and successful canaries, remaining absolute coverage,
cancellation/recovery, every finite budget, sealed-evidence rejection,
migration/Doctor/provenance integrity, and a fresh-database CLI process flow.

No ordinary test uses network access, credentials, paid calls, a model download,
GPU, or an external Pi provider.

## Non-goals

This capability does not add UI/API work, automatic label/schema/dimension or
semantic changes, generator self-approval, sealed-evidence adaptation,
training/evaluation/promotion, semantic or embedding deduplication, distributed
workers, reinforcement learning, bandits, open-ended prompt search, silent row
mutation, or provider-owned core types.

## Completion criterion

The capability is complete when a user can start one finite supervised
generation run, observe immediate weakness or later drift from persisted facts,
inspect the scoped deterministic pause, obtain and review a bounded Pi guidance
revision, validate it with an independently assessed canary, resume only the
remaining absolute coverage under a new immutable prompt version, inspect
per-cell/per-strategy quality, recover safely after interruption, and trace each
qualified row through the complete strategy, prompt, generation, and quality
chain.
