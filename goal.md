# Goal — Controlled End-to-End Encoder Workflow and Evaluation Governance

Build the missing product layer that turns the existing local encoder-development
components into one coherent, resumable, CLI-first workflow.

The repository already contains independently useful implementations for:

- synthetic-data planning, generation, validation, deduplication, and coverage;
- immutable dataset snapshots and imports;
- linear and local BERT training with immutable checkpoints;
- evaluation, persisted predictions, statistical comparison, and model selection;
- deterministic error analysis;
- constrained optimization proposals, reviews, applications, and campaigns;
- SQLite persistence, provenance, recovery, and doctor checks.

Do not rebuild these slices. Audit them, preserve their public contracts, and
compose them through narrow workflow-owned interfaces.

The product journey to implement is:

```text
dataset definition + labels + arbitrary categorical dimensions
  -> exact initial row budget and allocation policy
  -> generation, validation, deduplication, and coverage completion
  -> immutable dataset snapshot
  -> encoder V1 training
  -> development benchmark evaluation
  -> deterministic metrics and error analysis
  -> optional advisory LLM assessment
  -> bounded data/training proposal
  -> explicit approval or bounded pre-authorization
  -> dataset diff and immutable snapshot V2
  -> encoder V2 training
  -> compatible paired evaluation and comparison
  -> repeat within finite limits or stop
  -> explicitly requested sealed acceptance evaluation
```

This is a long-range goal. Continue until the completion criterion is genuinely
satisfied. Do not stop after adding domain models, migrations, command stubs, or
one happy-path demonstration.

The guiding rules are:

- automation is a durable finite state machine, not an autonomous agent;
- deterministic acceptance policy decides pass/fail, never an LLM opinion;
- development evidence may guide iteration, sealed evidence may not;
- every external call and generated row stays inside explicit user budgets;
- every mutation-producing iteration is explicitly approved or covered by a
  finite persisted pre-authorization envelope;
- historical datasets, runs, evaluations, decisions, and workflow attempts are
  immutable;
- no graphical UI or HTTP expansion is part of this goal.

## 1. Architecture and product-boundary audit

Before implementation, read all platform and slice specifications, architecture,
development, operations, recovery, provenance, evaluation, analysis, optimization,
and configuration documentation. Inspect the actual CLI, runners, persistence
ports, leases, migrations, and end-to-end tests.

Produce an internal compatibility map covering:

- the command or application workflow that creates each artifact;
- the immutable identity and fingerprint of every artifact;
- the ports needed to start, inspect, cancel, recover, or link each workflow;
- current generation, training, and evaluation state transitions;
- which evidence Error Analysis and Optimization currently consume;
- existing campaign compatibility and outcome rules;
- failure, retry, cancellation, and interruption semantics;
- the exact modules that would become dependency or ownership hotspots.

Update `docs/platform-spec.md`, `docs/architecture.md`, and the relevant slice
specifications before adding automatic cross-slice execution. Add a focused
workflow/evaluation-governance specification. Update `AGENTS.md` only as needed to
permit this explicitly bounded orchestrator while retaining the prohibitions on
unbounded autonomy, cloud execution, and multiple workers.

Prefer a dedicated `workflow-core` crate if the audit confirms it gives the
state machine, policies, and ports one clear owner. Keep SQLite implementation in
an adapter and CLI assembly in the CLI application. Do not add generic
`services.rs`, `utils.rs`, or a central object that reaches into every database
table directly.

## 2. Exact initial dataset-budget allocation

Add a pure deterministic allocator for the initial dataset. It converts:

- labels;
- arbitrary categorical dimensions;
- an exact desired accepted-row total;
- current accepted coverage, when present;
- an allocation policy;
- optional minimums, maximums, exclusions, and reserved iteration budget;

into explicit absolute targets for ordinary Slice 1 generation cells.

Support at least these policies:

- balanced coverage across every eligible cell;
- user-supplied label and dimension-value weights;
- minimum coverage per cell followed by weighted remainder allocation;
- a fully explicit per-cell allocation;
- optional reservation of part of the total budget for later iterations.

The allocator must:

- conserve the requested total exactly when constraints are feasible;
- use deterministic integer rounding and canonical tie-breaking;
- account for already accepted rows without requesting negative work;
- reject unknown labels, dimensions, values, or duplicate canonical cells;
- report structured infeasibility and unallocated budget;
- preview every cell target before persistence or provider calls;
- create a normal explicit Slice 1 generation plan without bypassing its planner;
- avoid claiming that an evidence-free initial allocation is statistically
  optimal.

Treat targets as accepted-row coverage. Configure a finite attempt ceiling so
validation failures cannot cause unbounded generation. Expose requested,
attempted, accepted, rejected, remaining, and exhausted counts.

## 3. Dataset roles and evaluation-governance model

Introduce project-owned evaluation roles for immutable snapshot cohorts. At
minimum distinguish:

- training;
- development/validation;
- diagnostic/challenge;
- sealed acceptance;
- external benchmark.

A role assignment and every later role transition must be persisted and
append-only. Historical evaluations keep the role and policy that applied when
they ran.

Implement a system-enforced evidence boundary:

- training may consume only approved training membership;
- iterative evaluation, Error Analysis, Optimization, and the LLM advisor may
  consume development/diagnostic evidence;
- sealed-acceptance rows, labels, predictions, error examples, and slice details
  must never enter analysis, optimization, prompt construction, or workflow
  iteration decisions;
- sealed evaluation is a separate explicit workflow action, not an automatic
  stage in the iterative loop;
- default sealed output should disclose only policy-approved aggregate results;
- inspecting sealed row-level errors or using its result to change a model must
  append an exposure and retire or demote that cohort from unbiased-acceptance
  use;
- repeated aggregate exposure must be counted and surfaced as an adaptive-
  overfitting risk rather than silently treated as independent evidence.

Add an immutable exposure ledger recording:

- cohort/snapshot and role;
- evaluation and protocol;
- requesting workflow/iteration;
- purpose: development, diagnosis, comparison, acceptance, or manual inspection;
- disclosure level: aggregate, slices, predictions, or row content;
- whether the evidence was eligible for adaptation;
- timestamp and append-only retirement/demotion decisions.

The platform is local and cannot prevent a user from opening SQLite manually.
Document that sealing is an application-enforced scientific-governance boundary,
not encryption or access control.

## 4. Leakage and adaptive-overfitting defenses

Add practical protections around dataset construction and evaluation:

- exact and normalized-text overlap checks across train, development, diagnostic,
  sealed, and external cohorts;
- optional source/entity/group identifiers so related examples can be split as a
  unit;
- deterministic group-aware splitting where group identity exists;
- provenance checks that detect the same source row in incompatible cohorts;
- a contamination report persisted before a benchmark suite becomes eligible;
- policy-controlled thresholds that block execution or require explicit override;
- warnings when a development cohort has been repeatedly optimized against;
- support for multiple deterministic development cohorts or folds so iteration
  does not depend on one static validation sample;
- a fresh-cohort or retirement path when evaluation evidence has become adaptive.

Do not describe nested validation, multiple cohorts, bootstrap intervals, or
significance testing as magical protection. Any evidence used to guide changes is
adaptive evidence. Make that fact visible in status, comparison, and promotion
commands.

## 5. Immutable benchmark suites and acceptance contracts

Create a benchmark-suite model that groups compatible evaluation cohorts and
their policies. A suite may contain internal immutable snapshot cohorts and
locally imported external classification benchmarks.

Each suite must bind:

- task and label vocabulary;
- cohort identity, evaluation role, and split;
- required predictor/checkpoint compatibility;
- metric definitions and evaluation protocol fingerprints;
- overall, per-label, and arbitrary-dimension slice thresholds;
- minimum support requirements;
- allowed regression tolerances against a named baseline;
- confidence-interval or paired-comparison requirements where applicable;
- disclosure and adaptation eligibility;
- contamination status;
- an immutable suite fingerprint.

Acceptance must be deterministic and structured. Produce pass, fail,
inconclusive, and invalid states with exact reasons. An LLM narrative cannot
override the contract.

Allow a development suite to drive iterations and a sealed suite to decide an
explicit release/milestone assessment. Do not build a remote benchmark catalog,
download arbitrary benchmark code, or execute user-supplied scripts.

## 6. Workflow domain and finite state machine

Define small workflow-owned domain objects only where useful, such as:

- `WorkflowDefinition`;
- `WorkflowRun`;
- `WorkflowIteration`;
- `WorkflowStage` and `WorkflowStageAttempt`;
- `WorkflowBudget`;
- `WorkflowPolicy`;
- `ApprovalEnvelope`;
- `StopDecision`;
- artifact-link summaries owned by the workflow boundary.

The exact names may differ. Do not copy complete domain models from every slice
into the workflow crate.

Support durable stages resembling:

1. initial allocation and plan creation;
2. generation;
3. snapshot materialization;
4. training;
5. development benchmark evaluation;
6. deterministic acceptance assessment;
7. error analysis;
8. optional LLM advisory assessment;
9. optimization proposal creation;
10. approval wait or pre-authorization validation;
11. proposal application and dataset-diff generation;
12. next snapshot, training, evaluation, comparison, and follow-up analysis;
13. iteration stop/continue decision;
14. completed, failed, cancelled, exhausted, or awaiting-user state.

Model transitions explicitly and reject illegal transitions. Persist the
workflow definition resolved at start so mutable project defaults cannot change
a running or historical workflow.

The workflow must link ordinary artifacts created by existing slices. It must
not invent alternative generation plans, snapshots, training runs, evaluations,
analysis reports, or optimization proposals.

## 7. Orchestration and execution semantics

Starting a workflow is explicit authorization for its configured initial
generation, snapshot, training, and development-evaluation stages. Those stages
may chain automatically after successful completion.

Iteration supports two governance modes:

- `review-each-iteration`: pause after a proposal and require an explicit
  compatible approval before applying it or spending more provider budget;
- `preauthorized-bounded`: continue only while a persisted user-supplied envelope
  permits the exact action, row budget, advisor-call budget, training candidate,
  iteration count, and acceptance policy.

Pre-authorization is not permission for an LLM to expand scope. Any action
outside the envelope pauses for review.

Enforce finite limits including:

- maximum iterations;
- maximum initial and cumulative accepted rows;
- maximum generation attempts and provider requests;
- maximum advisor calls and optional token ceilings;
- permitted generation backend/model;
- permitted training backend and explicit finite configuration space;
- maximum wall-clock stage attempts where practical;
- minimum improvement and maximum tolerated regression;
- stop on invalid, incompatible, stale, or inconclusive evidence according to
  persisted policy.

Do not promise monetary cost enforcement unless provider pricing is explicit and
versioned. Track observable request/token usage and enforce count/token limits.

Stage execution must be:

- idempotent;
- restartable after process interruption;
- cancellable between bounded batches;
- protected by the existing process-identity lease model;
- single-worker and local;
- transactional when recording a stage result and artifact link;
- safe against duplicate commands and concurrent resume attempts;
- explicit about retryable versus terminal failures.

If a slice artifact was durably created before a crash, recovery must discover,
validate, and link it rather than creating a duplicate. Never silently replay a
completed provider call, training run, or sealed evaluation.

## 8. Advisory LLM analysis through a replaceable port

Add an optional project-owned advisor contract independent from any provider SDK.
Conceptually:

```text
AnalysisAdvisor
  generate_advisory(AdvisoryRequest) -> AdvisoryAssessment
```

Implement:

- a deterministic fake advisor for ordinary tests and offline workflows;
- one OpenAI-compatible adapter, reusing transport conventions where helpful
  without coupling analysis to the generation backend;
- prompt construction as an inspectable versioned component separate from HTTP;
- strict structured-output parsing and composable validation;
- persisted provider/model/prompt/version/parameters/usage metadata;
- environment-variable credentials only, never persisted raw secrets.

The normalized request may contain only policy-approved development evidence:

- task/schema and canonical labels/dimensions;
- deterministic aggregate metrics and comparison facts;
- bounded representative development errors;
- coverage and validation summaries;
- reviewed analysis findings;
- the finite action space and remaining workflow budget.

Never send sealed evidence. Require an explicit persisted data-egress policy
before sending raw imported or generated text to an external provider. Support an
aggregate-only mode.

Treat every model response as untrusted. Validate:

- referenced labels, dimensions, values, cells, findings, and evaluations;
- claimed evidence against persisted facts;
- finite confidence values and bounded list sizes;
- proposed action kinds against the allowed action space;
- absence of direct commands, arbitrary code, credentials, or provider-specific
  types in core artifacts.

An advisory assessment may provide:

- a concise evidence-linked interpretation;
- suspected data-quality problems;
- coverage-gap and confusion hypotheses;
- candidate cells for more data or manual review;
- cautions and uncertainty;
- a recommendation to stop, inspect, or consider another bounded experiment.

It must not:

- be the authoritative good/bad decision;
- directly create a generation plan, mutate a dataset, start training, or expose
  sealed evidence;
- claim root cause or guaranteed improvement;
- invent target counts outside deterministic planning and allocation;
- bypass Optimization review, application, or workflow budgets.

## 9. Immutable dataset diffs and iteration lineage

Represent every iteration as new artifacts rather than mutation:

```text
snapshot V1
  + accepted rows from approved generation-plan delta
  = immutable snapshot V2
```

Preserve lineage from each added row and target cell through:

- advisor assessment, when used;
- deterministic analysis evidence;
- optimization proposal and scenario;
- approval or pre-authorization decision;
- applied generation plan and jobs;
- resulting snapshot;
- V2 training run/checkpoint;
- compatible V1/V2 evaluations and paired comparison;
- follow-up analysis and workflow stop decision.

Do not delete allegedly bad historical rows automatically. Data-removal,
relabeling, or schema-change recommendations remain review-only unless a future
explicit specification adds immutable correction workflows.

Training V2 may start fresh or continue from a compatible checkpoint according
to an explicit persisted policy. Never silently change that choice between
iterations.

## 10. Deterministic stop, promotion, and final evaluation

Stop/continue decisions must combine the persisted acceptance contract,
statistical comparison, workflow budget, and governance mode.

Support deterministic reasons including:

- development acceptance thresholds satisfied;
- statistically supported improvement without forbidden regression;
- regression or invalid comparison;
- insufficient or inconclusive evidence;
- no eligible recommendations;
- minimum-improvement threshold not met;
- maximum iterations, rows, attempts, advisor calls, or token budget reached;
- stale or incompatible evidence;
- awaiting approval;
- user cancellation.

The command that declares a candidate ready must not imply final generalization.
Final acceptance requires a separate explicit command against a sealed suite.
That command records an exposure and cannot feed the current workflow back into
analysis or optimization.

If the user inspects final errors or chooses another iteration based on final
results, require an explicit cohort retirement/demotion record before continuing.

Model promotion is an immutable record linking the selected checkpoint, training
snapshot, development assessment, optional sealed assessment, benchmark suite,
and exact policy. Do not overwrite a global `best model` pointer without history.

## 11. Persistence and migrations

Use append-only migrations and normalized relationships required for integrity,
filtering, pagination, provenance, and doctor checks.

Persist at least:

- initial allocation policies, constraints, previews, and results;
- snapshot cohort roles and append-only role decisions;
- contamination reports and overrides;
- benchmark suites, cohort membership, protocols, thresholds, and fingerprints;
- exposure-ledger entries;
- immutable workflow definitions and resolved budgets;
- workflow runs, iterations, stages, attempts, leases, and state transitions;
- idempotency keys and linked slice-artifact identities;
- approval envelopes and consumption counters;
- advisor requests/assessments, prompt identity, validation, and usage metadata;
- deterministic acceptance and stop decisions;
- dataset-diff lineage;
- immutable promotion/final-assessment records.

Do not duplicate complete datasets, predictions, model tensors, or earlier slice
artifacts in workflow tables. Store their identities, fingerprints, compatibility
facts, and bounded workflow-owned summaries.

Add migration upgrade tests and run `PRAGMA foreign_key_check`. Preserve every
existing database and legacy artifact.

## 12. CLI-first user workflow

Expose the complete behavior through focused, scriptable CLI commands before any
new API or graphical work.

Provide behavior equivalent to:

```text
synth allocation preview --project project.toml --total-rows 20000
synth benchmark create --definition benchmark.toml
synth benchmark validate <SUITE_ID>
synth workflow create --definition workflow.toml
synth workflow plan <WORKFLOW_ID>
synth workflow start <WORKFLOW_ID>
synth workflow status <RUN_ID>
synth workflow watch <RUN_ID>
synth workflow approve <RUN_ID> <PROPOSAL_ID>
synth workflow resume <RUN_ID>
synth workflow cancel <RUN_ID>
synth workflow iterations <RUN_ID>
synth workflow explain-stop <RUN_ID>
synth workflow finalize <RUN_ID> --sealed-suite <SUITE_ID>
synth workflow promote <RUN_ID> <CHECKPOINT_ID>
synth exposure list --suite <SUITE_ID>
```

Exact names may differ to fit existing conventions.

Requirements:

- `plan` previews all provider calls, row targets, stages, limits, development
  cohorts, and approval gates without mutation;
- `start` returns a durable run identity and does not hide long work inside an
  HTTP request;
- `status` and `watch` display persisted stage and budget progress;
- JSON stdout remains machine-readable; warnings/progress use stderr;
- list operations are filtered, sorted, and bounded;
- `cancel` is durable and observed between batches;
- `resume` is idempotent and lease-safe;
- commands expose why a stage paused, failed, stopped, or needs approval;
- raw API keys never appear in arguments, TOML, SQLite, output, or logs.

Do not add a TUI. Do not extend the existing Slice 1 server or graphical surface.
The CLI is the product surface for this goal and the future UI must call the same
application boundary.

## 13. Provenance, doctor, and recovery

Extend provenance so a promoted encoder or final assessment traces through:

```text
promotion/final assessment
  -> sealed exposure + benchmark suite + acceptance contract
  -> workflow run and stop decision
    -> iteration N checkpoint/evaluation/comparison
      -> snapshot N and dataset diff
        -> approved proposal + analysis + optional advisor assessment
          -> previous evaluation/checkpoint/snapshot
    -> initial allocation and generation plan
      -> dataset definition and generation backend metadata
```

Extend doctor to recompute and verify:

- allocation totals, constraints, and canonical cell targets;
- benchmark-suite, cohort-role, contamination, and acceptance fingerprints;
- sealed-evidence exclusion from analysis, optimization, and advisor requests;
- exposure-ledger consistency and retirement decisions;
- workflow definition, budget, stage, transition, and artifact-link integrity;
- approval-envelope scope and consumed limits;
- idempotency and absence of duplicate stage artifacts;
- advisor prompt/request/assessment fingerprints and referenced evidence;
- comparison compatibility and deterministic stop decisions;
- dataset-diff and V1/V2 provenance;
- final-assessment and promotion compatibility;
- migration and foreign-key integrity.

Doctor must not call a provider, load model tensors, generate rows, run training,
evaluate a model, reveal sealed rows, or mutate workflow state.

Recovery must distinguish interrupted, retryable, awaiting-approval, exhausted,
cancelled, and terminal states. It must never infer approval or silently consume a
new external-call budget.

## 14. Testing and practical verification

Ordinary tests must require no network, credentials, provider credits, public
model download, Python runtime, GPU, or external service.

Add focused tests for:

- exact total-budget allocation under every policy;
- weighted rounding, ties, existing coverage, reservations, and infeasibility;
- arbitrary dimensions and collision-safe cell identities;
- cohort-role transitions and immutable history;
- group-aware splitting and cross-cohort contamination detection;
- sealed-evidence rejection at every analysis, optimization, advisor, export, and
  workflow boundary;
- exposure counting, disclosure levels, retirement, and repeated-use warnings;
- benchmark-suite validation and deterministic acceptance outcomes;
- workflow transition legality and resolved-definition fingerprints;
- stage idempotency, duplicate commands, and concurrent resume attempts;
- interruption between every major stage and recovery without duplicate work;
- cancellation and retry budgets;
- approval-envelope validation and consumption;
- fake-advisor determinism and OpenAI-compatible normalization through a local
  mock server;
- malicious, malformed, unsupported, and evidence-inconsistent advisor output;
- aggregate-only and explicit text-egress policies;
- iteration budget exhaustion and every stop reason;
- immutable dataset-diff and V1/V2 lineage;
- fresh versus continuation training policy;
- final sealed evaluation isolation and post-exposure retirement;
- promotion history and provenance;
- migration upgrades and legacy database compatibility;
- doctor detection of tampered allocations, exposure, stages, budgets, advisor
  evidence, decisions, and links.

Expand the offline CLI end-to-end test to execute a complete deterministic
two-version workflow:

1. define a dataset with arbitrary dimensions and an exact total budget;
2. preview and persist the initial allocation;
3. generate with the fake backend and materialize snapshot V1;
4. train encoder V1 automatically through the workflow;
5. evaluate a development suite, analyze errors, and call the fake advisor;
6. create a bounded proposal and pause for approval;
7. approve and resume generation of an explicit dataset diff;
8. materialize snapshot V2 and train encoder V2;
9. evaluate the same compatible development evidence and compare V1/V2;
10. record a deterministic stop decision;
11. explicitly run a sealed aggregate-only acceptance evaluation;
12. promote or reject the candidate according to the persisted contract;
13. inspect exposures, budget consumption, iteration history, and provenance;
14. interrupt and resume at least one independently tested stage; and
15. pass doctor and every standard repository gate.

Keep all existing generation, dataset, linear, transformer, evaluation,
comparison, analysis, optimization, campaign, API, and legacy migration tests
passing.

Live-provider smoke tests, if added, must be opt-in, spend-bounded, and excluded
from ordinary verification.

## 15. Documentation and operator experience

Update:

- README quick start;
- architecture and platform specification;
- relevant slice specifications;
- configuration and example TOML files;
- CLI guide;
- operations and recovery guides;
- provenance and migration documentation;
- evaluation/model-selection, error-analysis, and optimization guides;
- development workflow.

Add focused documentation for:

- the complete user journey;
- initial balanced versus weighted allocation;
- development, diagnostic, sealed, and external benchmark roles;
- why repeated holdout use creates adaptive overfitting;
- contamination and group-aware splitting;
- deterministic acceptance versus advisory LLM interpretation;
- external text-egress controls;
- review-each-iteration and bounded pre-authorization;
- workflow budgets, pause states, retries, cancellation, and recovery;
- dataset V1/V2 diff lineage;
- final assessment and model promotion;
- why a stopped development workflow is not automatically a final-quality model.

Examples must use fake/local components by default and clearly separate optional
provider configuration.

## Explicit non-goals

Do not implement:

- a graphical UI, TUI, or HTTP API expansion;
- authentication, multiple users, roles, or collaboration;
- cloud deployment, remote workers, distributed scheduling, or multiple workers;
- an unbounded autonomous agent or infinite closed loop;
- an LLM as final evaluator, acceptance authority, allocator, or executor;
- LLM access to sealed evidence;
- provider calls outside explicit persisted budgets;
- bandits, reinforcement learning, Bayesian optimization, population-based
  training, or unbounded hyperparameter search;
- dynamic execution of downloaded benchmarks or arbitrary user code;
- automatic relabeling, deletion, schema mutation, or historical artifact edits;
- semantic/embedding deduplication;
- causal claims from one iteration;
- a remote model registry or external experiment-tracking service;
- cryptographic access control for local sealed data;
- unrelated refactors or speculative framework abstractions.

## Working method and staged commits

Implement one coherent, working component at a time. Begin with domain and pure
policy logic, then persistence, CLI behavior, recovery/doctor, end-to-end tests,
and documentation.

Suggested stages:

1. architecture audit and specification update;
2. initial total-budget allocation;
3. cohort roles, contamination checks, and exposure ledger;
4. benchmark suites and deterministic acceptance contracts;
5. workflow state machine and persistence;
6. idempotent orchestration of initial generation through development analysis;
7. advisor port, fake, prompts, validation, and OpenAI-compatible adapter;
8. approval envelopes and bounded iterative execution;
9. dataset-diff, V1/V2 comparison, stop decisions, and promotion;
10. final sealed-evaluation isolation;
11. provenance, doctor, recovery, and migration hardening;
12. complete offline CLI end-to-end verification and documentation.

After every coherent stage run:

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

Review dependency direction, public interfaces, secret handling, sealed-evidence
boundaries, idempotency, and budget conservation after every stage.

Commit each coherent passing stage. Do not rewrite published history. Preserve
unrelated user changes. Never commit credentials, runtime databases, generated
model artifacts, or provider responses containing private data.

## Completion criterion

This goal is complete only when a user can:

1. define labels, arbitrary categorical dimensions, and an exact initial accepted-
   row budget;
2. preview and persist a deterministic balanced, weighted, constrained, or
   explicit allocation whose totals are auditable;
3. configure provider settings without persisting an API key;
4. start one durable workflow that generates the initial dataset, validates it,
   freezes snapshot V1, trains encoder V1, and evaluates a development benchmark;
5. watch persisted stage, coverage, usage, retry, and budget progress and safely
   cancel or resume the workflow;
6. obtain deterministic metrics, acceptance status, error analysis, and an
   optional evidence-linked LLM advisory without exposing sealed evidence;
7. inspect and approve a bounded dataset/training proposal or run inside a finite
   explicit pre-authorization envelope;
8. create a provenance-preserving dataset diff, immutable snapshot V2, encoder V2,
   compatible evaluation, paired comparison, and follow-up analysis;
9. repeat only within persisted limits and receive an auditable deterministic stop
   reason;
10. prove that training, analysis, optimization, prompts, and iteration decisions
    cannot consume sealed-acceptance rows or predictions;
11. explicitly evaluate the selected candidate against a sealed benchmark suite,
    record the exposure, and deterministically promote, reject, or mark it
    inconclusive;
12. inspect complete V1/V2 lineage, allocation decisions, approvals, advisor usage,
    exposures, budget consumption, and promotion provenance;
13. recover safely from interruption without duplicate provider calls or artifacts;
14. complete the two-version offline CLI workflow and pass doctor; and
15. pass `cargo fmt-check`, `cargo check-all`, `cargo lint`, and `cargo test-all`
    with all existing behavior preserved.

The architecture matters as much as automation. The finished system must make
the desired one-command journey convenient while keeping evidence roles,
external spending, iteration scope, acceptance decisions, and historical lineage
explicit and enforceable.
