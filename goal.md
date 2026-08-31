# Goal — Generation Quality Supervisor

Build a bounded, evidence-driven generation supervisor that continuously checks
whether synthetic rows are useful, detects both immediate weakness and later
quality drift, pauses affected generation work, proposes a versioned prompt
repair through Pi, validates the repair on a small canary, and resumes only
when the evidence satisfies an immutable quality contract.

This is not merely structural validation. A row may be valid JSON, carry an
allowed label, and still be poor training data because it is trivial,
repetitive, synthetic-looking, shortcut-heavy, insufficiently discriminative,
or inconsistent with its intended difficulty, authenticity, or generation
strategy.

Do not add a graphical UI or new HTTP surface. Build the complete core,
persistence, deterministic fakes, tests, recovery, provenance, and CLI first.
Before implementation, write `docs/generation-quality-supervisor-spec.md` and
update the platform and architecture documentation so this new bounded
capability has an explicit contract.

Follow `AGENTS.md`, `docs/platform-spec.md`, `docs/architecture.md`,
`docs/development.md`, `docs/slice-1-spec.md`,
`docs/research-agent-spec.md`, `docs/dataset-architect-spec.md`, and
`docs/dataset-quality-spec.md`.

## Product objective

Turn one large, blind generation run into a finite sequence of monitored
generation segments:

```text
approved dataset, semantic, authenticity, and strategy contracts
  -> immutable generation-quality contract
  -> small initial generation segment
  -> independent row and batch observations
  -> deterministic quality decision
       -> healthy: continue within the same approved contract
       -> weak/drifting: pause the affected scope
  -> bounded Pi diagnosis and prompt-revision proposal
  -> deterministic validation and explicit review/pre-authorization
  -> small canary using the immutable revision
       -> passed: continue remaining absolute coverage
       -> failed: reject revision, retry within the finite budget, or escalate
  -> complete dataset with prompt-version and quality provenance per row
```

The supervisor must detect both:

1. **Immediate quality failure** — the first observed batch is already weaker
   than the approved contract.
2. **Longitudinal quality drift** — generation begins acceptably and later
   becomes easier, more repetitive, less authentic, less diverse, less
   strategy-compliant, or otherwise worse.

Treat both as generation-quality violations even though only the second is
strictly temporal drift.

## Central authority rule

Pi is a bounded diagnostic and prompt-revision advisor. It is never the quality
authority.

Deterministic application code owns:

- quality-contract validation;
- observation normalization and evidence integrity;
- thresholds, windows, minimum support, and stop decisions;
- generation and revision budgets;
- canary pass/fail decisions;
- legal lifecycle transitions;
- approval and pre-authorization boundaries;
- which rows count toward accepted coverage.

The generator must not grade itself. Record the relationship between generator
and evaluator identities. Use existing deterministic validators for facts they
can decide, and a separately configured quality evaluator only for semantic
judgments. Reuse the existing blind evaluator and independent-review concepts
where their contracts fit; do not copy their logic into a second monolithic
validator.

## Quality contract

Define one immutable, versioned `GenerationQualityContract` or equivalent. The
exact name may differ, but the contract must pin:

- dataset, plan, accepted-coverage, semantic-context, authenticity-context,
  construction-plan, and strategy-context identities and fingerprints;
- generator and evaluator backend/model/protocol identities;
- row-level criteria, batch-level criteria, thresholds, and issue codes;
- minimum evidence before a quality or drift decision is permitted;
- canary size and monitoring-window policy;
- permitted actions and affected scope;
- maximum segments, prompt revisions, canaries, evaluator requests, attempts,
  tokens, duration, and optional cost;
- approval mode: explicit review or a finite persisted pre-authorization
  envelope;
- the complete normalized prompt-revision policy;
- its own reproducible fingerprint.

Defaults and presets are only authoring conveniences. Persist the fully
resolved values. The agent may not weaken or rewrite the contract, thresholds,
budgets, label semantics, trusted dimensions, or approval policy.

The contract should be assembled from explicit operator settings plus pinned
semantic, authenticity, and approved generation-strategy facts. If an expected
property is not defined well enough to measure, report it as unavailable rather
than inventing a target.

## Quality signals

Use small composable observations. Do not create one giant
`assess_generation_quality()` function.

### Row-level signals

At minimum support:

- structural and schema validity;
- non-empty text and valid trusted cell identity;
- exact and normalized duplicate detection;
- label fidelity and margin over competing labels;
- categorical-dimension adherence;
- intended difficulty or challenge adherence when defined;
- authenticity adherence when an approved profile is pinned;
- generation-strategy adherence when a row has a strategy assignment;
- label-name leakage and other shortcut/artifact risk;
- evaluator confidence and normalized issue codes.

### Batch-level signals

At minimum support:

- accepted/rejected/quarantined rate;
- exact and normalized duplicate rate;
- lexical and structural repetition indicators;
- text-length and other measurable authenticity distributions;
- intended difficulty/challenge distribution;
- strategy target versus observed/accepted distribution;
- label- or cell-specific shortcut concentration;
- missing rare, boundary, ambiguity, or noise patterns required by the
  contract;
- per-cell and per-strategy quality rather than only global averages.

Prefer deterministic features for measurable properties. Provider judgments
must be normalized into bounded project-owned evidence before deterministic
policy consumes them.

Do not add embedding or semantic-similarity deduplication in this goal.

## Strategy execution

Make approved strategy guidance operational rather than leaving an LLM to
interpret approximate shares for an entire batch.

Compile per-cell strategy shares into deterministic, exact row or small-batch
assignments. Each assignment must pin:

- cell identity;
- strategy/directive identity;
- instructions and expected criterion;
- assignment sequence or deterministic seed;
- source strategy-context fingerprint.

Prompt construction receives only the strategies assigned to that row or
batch. Track persisted strategy coverage:

```text
strategy target / attempted / accepted / rejected / quarantined / remaining
```

The supervisor may detect that valid rows are too weak because hard-negative,
boundary, ambiguity, rare-pattern, authenticity, or other approved strategy
requirements are not actually being satisfied.

## Monitoring and decisions

Monitoring must be scoped. One failing cell or strategy should not stop an
unrelated healthy cell unless the evidence demonstrates a systemic problem.

A deterministic decision records one of:

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

Names may differ, but the lifecycle must distinguish evidence insufficiency,
quality failure, revision review, canary outcome, terminal budget exhaustion,
and cancellation.

Avoid a hard-coded "check every 500 rows" rule. Resolve an explicit policy with
an initial canary, rolling bounded windows, minimum sample support, and exact
thresholds. Small datasets and rare cells must not require an arbitrary large
batch before obvious failures can pause them.

Accepted rows from an earlier prompt version may remain if they satisfy the
contract. Weak, invalid, or failed-canary rows remain immutable evidence but do
not count toward qualified coverage. Never silently delete or relabel them.

## Bounded Pi supervisor

Use the existing `agent-runtime-core` boundary and a Pi process adapter. Pi
receives only an application-owned, versioned capability set with narrow tools
equivalent to:

- `inspect_quality_contract`;
- `inspect_quality_window`;
- `inspect_failure_breakdown`;
- `inspect_current_prompt_guidance`;
- `preview_prompt_revision`;
- `submit_prompt_revision`;
- `finish_supervision`.

Pi receives no shell, filesystem, database, arbitrary network, generation
backend, or sealed-evidence access. Tool calls are validated against the exact
persisted contract, paused scope, current evidence, remaining budget, and
cancellation state. Every external model-call intent is persisted before I/O.

The agent must diagnose whether the evidence suggests:

- weak or conflicting prompt guidance;
- semantic-definition ambiguity;
- contradictory dimensions or strategies;
- generator/model inadequacy;
- evaluator disagreement or insufficient evidence;
- repetition or mode collapse;
- authenticity or strategy failure;
- a problem that prompt revision cannot safely repair.

Repeatedly rewriting a prompt is not an acceptable fallback. When the likely
cause is outside prompt guidance, or when the finite revision budget is
exhausted, escalate with structured evidence.

## Prompt revision contract

Never overwrite a prompt string or mutate an active generation execution.

A `PromptRevisionProposal` or equivalent must be immutable and pin:

- parent prompt/template and guidance fingerprints;
- triggering quality decision and exact evidence window;
- affected cells and strategies;
- bounded diagnosis and issue codes;
- proposed instruction changes;
- expected measurable improvement;
- unchanged protected fields;
- agent/runtime/model/tool-protocol identity and usage;
- revision number and reproducible fingerprint.

Keep the base system prompt, output schema, trusted target label/dimensions,
semantic authority, safety instructions, and construction rules protected.
Prefer a bounded guidance patch rendered by the existing prompt builder over an
agent-authored replacement system prompt.

Revisions require an append-only review unless the exact change and scope fit a
persisted finite pre-authorization envelope. A revision can become active only
after deterministic validation and a successful canary.

The canary must run through a new immutable prompt version and a new ordinary
generation execution/segment. It must never retroactively change the execution
specification of an existing job. Remaining work continues from persisted
absolute accepted coverage so retries or new segments cannot over-generate.

## Supervisor orchestration

Keep the existing generation planner, job runner, prompt builder, backend port,
validators, dataset-quality evaluator, and persistence contracts independently
useful. The supervisor composes their normal contracts; it must not copy their
business logic.

Prefer a separate feature-owned core and adapter boundary if the capability
cannot remain cohesive inside existing generation modules. Do not introduce a
generic agent service, generic orchestration framework, or new god module.

A supervisor run should own a finite sequence of exact child links:

```text
supervisor run
  -> generation segment/job
  -> quality observation/audit
  -> quality decision
  -> optional prompt revision and review
  -> canary segment/job
  -> next normal segment/job
```

Reserve every child identity durably before starting it. Cancellation is
persisted first and forwarded only to exact linked children. Recovery must not
replay an uncertain paid call, infer approval, duplicate a segment, mutate a
prompt version, or reset a consumed budget.

## Post-training handoff

Design the supervisor contract so a later evidence-driven remediation phase can
provide approved development-error archetypes and strategy proposals. Do not
implement sealed-data adaptation or a new training/evaluation loop here.

Development evidence may later enter only through the existing governed
exposure contracts. Sealed benchmark row content, predictions, slices, or
acceptance evidence must never enter generation supervision, Pi tools, prompt
revision, or canary decisions.

## Persistence and provenance

Persist at minimum:

- quality contracts;
- supervisor runs and finite budgets;
- generation segments and exact child links;
- strategy assignments and strategy coverage;
- observation windows and normalized row/batch evidence;
- deterministic quality decisions;
- Pi calls, tool calls, usage, stop reasons, and diagnoses;
- prompt-revision proposals, reviews, and immutable prompt versions;
- canary inputs, outcomes, and activation decisions;
- terminal escalation, failure, cancellation, and recovery facts.

Every accepted generated row must remain traceable to its generation job,
attempt, prompt version, strategy assignment when present, generator identity,
quality evidence, and supervisor run. Doctor and provenance traversal must
verify the complete chain and fail closed on tampering.

## CLI boundary

Add a scriptable CLI flow for:

- contract preview/create/show;
- supervisor start/status/watch/cancel/recover;
- quality-window and issue inspection;
- revision show/review;
- canary status and outcome inspection;
- strategy coverage;
- escalation and provenance inspection.

Human output should explain why generation paused and what changed. JSON output
must remain one stable machine-readable value on stdout. No graphical UI or new
HTTP API belongs to this goal.

## Testing priorities

Most tests must use deterministic generation, evaluator, and scripted Pi fakes.
No ordinary test may require network access, credentials, paid calls, a model
download, or a GPU.

Prove at minimum:

- structurally valid but trivial rows trigger an immediate quality pause;
- initially good generation that later becomes repetitive triggers drift;
- label leakage, shortcut concentration, authenticity failure, strategy
  non-adherence, and difficulty mismatch are independently visible;
- one failing cell can pause without stopping unrelated healthy cells;
- insufficient evidence never fabricates a drift decision;
- exact strategy targets compile deterministically and conserve counts;
- the generator cannot act as its own independent evaluator unnoticed;
- Pi cannot alter thresholds, protected prompt fields, semantics, labels,
  dimensions, budgets, or approval rules;
- prompt V1 remains immutable when V2 is proposed;
- a failed V2 canary cannot become active or count toward coverage;
- a successful approved V2 canary resumes only remaining absolute coverage;
- accepted V1 rows remain and rejected/quarantined rows never count as accepted;
- cancellation and recovery preserve child identities and never replay an
  uncertain external call;
- maximum revisions, segments, calls, attempts, tokens, duration, and cost are
  enforced before new work;
- sealed evidence cannot cross the supervisor boundary;
- persistence, provenance, Doctor, migrations, and CLI work end to end;
- all Rust quality gates pass.

## Explicit non-goals

Do not add:

- GUI, TUI, or new HTTP/API work;
- unbounded autonomous prompt optimization;
- automatic changes to labels, schema, dimensions, semantic definitions,
  benchmark contracts, or quality thresholds;
- generator self-approval;
- access to sealed benchmark evidence;
- training, evaluation, promotion, or post-training remediation logic inside
  the supervisor;
- semantic/embedding deduplication;
- distributed execution or multiple workers;
- reinforcement learning, bandits, or open-ended prompt search;
- silent deletion, mutation, or relabeling of historical rows;
- provider types in core contracts.

## Suggested implementation sequence

1. Author the capability specification and dependency design.
2. Add immutable quality-contract and normalized observation domain types.
3. Add pure row/batch aggregation and deterministic decision logic.
4. Compile approved strategy shares into exact assignments and coverage.
5. Add deterministic fakes and focused unit tests.
6. Add immutable prompt-revision, review, and canary contracts.
7. Add the bounded Pi supervisor runner and narrow tool capability set.
8. Add supervisor lifecycle, finite segment orchestration, cancellation, and
   recovery.
9. Add SQLite persistence, migrations, deep integrity verification, provenance,
   and Doctor checks.
10. Add the complete CLI flow.
11. Integrate as an optional generation phase without changing legacy jobs.
12. Add offline CLI and workflow-level end-to-end tests and documentation.

After each coherent stage, run `cargo fmt-check`, `cargo check-all`,
`cargo lint`, and `cargo test-all`, review dependency directions and public
interfaces, and commit the working change when Git identity is configured.

## Completion criterion

This goal is complete when a user can start a finite supervised generation run,
observe weak-but-valid rows or later quality drift, see the persisted evidence
that caused a scoped pause, obtain a bounded Pi prompt-repair proposal, review
or pre-authorize it, validate it on an independently assessed canary, resume the
remaining absolute coverage under a new immutable prompt version, inspect
per-cell and per-strategy quality, recover safely after interruption, and trace
every accepted row through the complete strategy, prompt, generation, and
quality provenance chain.

Most importantly, the system must prevent a generation run from producing
thousands of technically valid but strategically weak examples merely because
the schema and labels were correct.
