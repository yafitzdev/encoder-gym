# Controlled workflow and evaluation-governance specification

## Objective

Compose the independently useful platform slices into one durable, finite,
CLI-first encoder-development workflow while preserving their ownership and
replaceability boundaries.

A user must be able to define an exact initial accepted-row budget, automatically
run initial generation through development evaluation, inspect deterministic and
optional advisory evidence, approve or finitely pre-authorize a bounded
iteration, compare encoder versions, and run a separately authorized sealed
acceptance assessment.

The workflow is a persisted state machine, not an autonomous agent. Every
provider call, generated row, iteration, training choice, disclosure, and
adaptation decision is constrained by an immutable resolved workflow definition.

## Existing compatibility map

The workflow must reuse the current artifact-producing paths below.

| Artifact | Current owner and creation path | Durable state / identity | Workflow use |
|---|---|---|---|
| Dataset definition | `synthetic-data-core` through `DatasetStore`; dataset/config CLI | UUID, labels, arbitrary dimensions, task | Source identity for allocation and every later cell |
| Generation plan | `explicit_target_plan` or equal shortcut through `PlanStore` | UUID and immutable absolute cell targets | Initial allocator and approved optimization application create normal plans |
| Generation job/rows | `JobRunner` through `GenerationStore` and `GenerationBackend` | queued/running/completed/failed/cancelled; row/job/provider provenance | Workflow starts and observes a bounded existing runner; it never generates directly |
| Dataset snapshot | `dataset-core::build_snapshot` through `SnapshotStore` | UUID, source row IDs, deterministic split, fingerprint | Immutable training/evaluation source and parent of later snapshot lineage |
| Training run/checkpoint | `TrainingRunner` through `TrainingBackend`, `TrainingStore`, and `CheckpointSink` | queued/running/completed/failed/cancelled; artifact checksum and fingerprint | Workflow requests a configured run and links its verified final checkpoint |
| Evaluation run/predictions | `EvaluationRunner` through `Predictor`, `EvaluationExampleSource`, and `EvaluationStore` | queued/running/completed/failed/cancelled; cohort/protocol/input fingerprints | Development or sealed suite execution under role/disclosure policy |
| Comparison/selection | pure `evaluation-core` comparison and immutable reports | compatible cohort/protocol fingerprints | V1/V2 evidence and deterministic acceptance input |
| Analysis report | `analysis-core::run_analysis` through analysis evidence/store ports | immutable report/protocol/source fingerprints and normalized findings | Development-only diagnostic input |
| Optimization proposal/review/application | `optimization-core` plus `OptimizationStore` | immutable evidence/protocol/proposal; append-only review; idempotent application | Produces bounded normal plan or training candidate after governance validation |
| Campaign/outcome | optimization campaign compatibility functions and store | immutable decision plus append-only links and outcome | Existing experiment-lineage evidence, not the workflow scheduler |
| Recovery record/lease | `recovery-core::RecoveryStore` | process kind/ID, PID/start identity, append-only resolution | Workflow stages extend this ownership model without weakening slice recovery |

Current command assembly lives in `synthetic-data-cli`. `synthetic-data-sqlite`
implements the ports in feature-oriented modules. The new workflow core must not
call CLI commands or SQL; the CLI assembles workflow-facing adapters around the
same application functions and runners.

## Ownership and dependency rules

`workflow-core` owns:

- exact initial-budget allocation policy and result;
- cohort roles, exposure/disclosure policy, and deterministic acceptance;
- benchmark-suite identity and compatibility policy;
- workflow definition, run, iteration, stage, attempt, budget, approval envelope,
  and stop decision;
- narrow ports for creating, starting, querying, cancelling, and linking ordinary
  artifacts;
- advisor request/assessment contracts and validation policy;
- pure transition, compatibility, budget-consumption, and stop logic.

It does not own:

- generation prompts or provider transport;
- row validation, deduplication, or coverage calculation;
- snapshot membership or split algorithms;
- trainer, predictor, checkpoint, or tensor implementation;
- evaluation metrics or paired statistics;
- error aggregation or optimization scoring/allocation;
- SQLite queries, filesystem layout, CLI parsing, or presentation.

Core dependencies point inward. Adapter-specific types never cross a workflow
port. Complete slice artifacts remain stored by their existing owner; workflow
storage retains identities, fingerprints, bounded compatibility facts, and stage
results.

## Initial allocation

An initial allocation resolves one exact accepted-row total into explicit
absolute Slice 1 cell targets. Policies are balanced, weighted, minimum-then-
weighted, or fully explicit. Existing accepted coverage, exclusions, finite
cell bounds, and an optional reserved iteration budget are explicit inputs.

Feasible allocation conserves the requested total with deterministic integer
rounding and canonical cell tie-breaking. Infeasible allocation returns a
structured remainder and never silently changes a constraint. Preview is
read-only. Persistence records normalized policy, inputs, results, and
fingerprint before normal plan creation.

## Evaluation roles and sealing

Every benchmark cohort has one current role derived from append-only decisions:

- training;
- development/validation;
- diagnostic/challenge;
- sealed acceptance;
- external benchmark.

Training, analysis, optimization, advisor prompting, and workflow iteration must
prove role compatibility at their application boundary. Sealed row contents,
labels, predictions, error examples, and slice details are ineligible for
adaptation. Sealed evaluation is a separately authorized action and defaults to
policy-approved aggregate disclosure.

Every evaluation or inspection appends an exposure containing purpose,
disclosure level, adaptation eligibility, workflow/iteration identity, and
timestamp. Row-level inspection or adaptive use retires/demotes a sealed cohort
before another iteration. Repeated aggregate exposure is visible as adaptive-
overfitting risk.

Sealing is scientific workflow governance inside the application. It is not
encryption and cannot prevent direct inspection of a local database by its
owner.

## Contamination and grouping

Before a benchmark suite becomes eligible, persist a contamination report that
checks exact source identity, exact text, normalized text, and optional source/
entity/group identity across cohorts. Group-aware splitting keeps related rows
together when group identity exists. Policy determines whether findings block,
warn, or require an explicit append-only override.

For local classification datasets, `snapshot create --group-dimension` treats
an arbitrary categorical dimension as the optional group identity and assigns
the whole group to one split deterministically. Persisted overlap findings keep
evidence fingerprints instead of raw text.

Evidence used to guide a model is adaptive regardless of whether it comes from
one holdout, multiple folds, confidence intervals, or statistical tests. The
platform reports exposure and retirement; it does not promise that repeated
development evaluation remains unbiased.

## Benchmark suites and acceptance

An immutable benchmark suite binds label vocabulary, cohort roles and splits,
evaluation protocols, metric thresholds, slice/support requirements, compatible
baseline and regression tolerance, disclosure, adaptation eligibility,
contamination status, and fingerprint.

Deterministic assessment returns `pass`, `fail`, `inconclusive`, or `invalid`
with structured reasons. Development suites may drive iteration. Sealed suites
may support final milestone assessment only and never become workflow adaptation
evidence.

The local definition format supports overall, per-label, and arbitrary slice
metric requirements, minimum support, paired baseline regression tolerances,
confidence-bound requirements, optional McNemar significance, model-format
compatibility, and explicit disclosure policy. Identical persisted assessment
inputs are idempotent at the CLI boundary.

## Workflow lifecycle

A resolved workflow proceeds through legal durable stages:

```text
allocation -> generation -> snapshot -> training -> development evaluation
  -> acceptance -> analysis -> optional advisor -> proposal
  -> awaiting approval / pre-authorization check
  -> applied diff -> next snapshot -> next training/evaluation/comparison
  -> stop or another finite iteration
```

Terminal or paused states include completed, failed, cancelled, exhausted,
inconclusive, and awaiting user. Final sealed evaluation and promotion are
explicit post-iteration actions.

Every stage start and outcome is a separate fingerprint-linked attempt event.
The mutable run row is only a concurrency-protected projection of that history;
it cannot advance unless the caller owns the expected latest event. Retryable
failures pause for recovery while non-retryable failures are terminal.

Initial `start` authorization may chain allocation through development analysis.
Later mutation-producing work either pauses for one compatible append-only
approval or fits an exact persisted pre-authorization envelope. No LLM response
can expand that envelope.

Every stage is idempotent, cancellable at existing batch boundaries, lease-safe,
and restartable without duplicating a verified artifact. Recovery cannot infer
approval, replay a completed provider request, or repeat a sealed evaluation.
Execution errors become explicit failed attempts; retryable failures pause and
`workflow resume` starts the same stage with an incremented attempt number up to
the persisted ceiling. Every new candidate currently uses the resolved
`fresh` training policy; checkpoint continuation is not silently inferred.

Development evaluation, diagnosis, advising, optimization, and paired
comparison each append their own idempotent exposure fact. A configured
fresh-cohort threshold produces a deterministic stop reason before further
adaptive iteration, rather than treating repeated holdout use as fresh evidence.

## Optional advisor boundary

The workflow owns a provider-neutral structured `AnalysisAdvisor` contract. A
deterministic fake supports ordinary tests and an OpenAI-compatible adapter is
replaceable infrastructure. Prompt construction is versioned and separate from
transport. Raw credentials remain environment-only.

Advisor input is bounded, policy-approved development evidence. Sending raw text
requires explicit persisted egress authorization; aggregate-only mode is
supported. Sealed evidence is rejected before prompt construction.

Advisor output is untrusted and validated against labels, dimensions, cell and
finding identities, evidence, finite bounds, and remaining action space. It may
provide hypotheses and cautions but cannot decide acceptance, allocate counts,
apply a proposal, or start a stage.

## Finite budgets and stop policy

The immutable workflow definition bounds iterations, accepted rows, generation
attempts/requests, advisor calls/tokens, backend/model choices, finite training
configuration choices, retries, and permitted regressions. Observable usage is
tracked; monetary cost is not promised without versioned provider pricing.

Stop decisions reproduce from benchmark assessment, paired comparison, proposal
eligibility, consumed budgets, and governance state. A successful development
stop means ready for explicit final assessment, not proven final quality.

Promotion is immutable history linking checkpoint, training snapshot,
development assessment, optional sealed assessment, suite, policy, exposures,
and provenance. There is no mutable unversioned `best model` decision.

## CLI and UI boundary

The complete workflow is scriptable through CLI preview/create/start/status/
watch/approve/resume/cancel/finalize/promote and exposure-inspection commands.
JSON stdout remains one machine-readable value; progress and warnings use
stderr. No TUI, HTTP addition, or graphical workflow UI belongs to this phase.

## Completion criterion

This phase is complete only when the offline CLI end-to-end test executes an
exact initial allocation, V1 generation/snapshot/training/development assessment,
approved dataset diff, V2 training and compatible comparison, deterministic stop,
explicit aggregate-only sealed assessment, promotion decision, recovery,
provenance, exposure inspection, and doctor without network or credentials.
