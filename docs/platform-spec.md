# Encoder-development platform specification

## Product objective

Build a local, single-user platform that turns explicit synthetic-data plans
into reproducible text-classification experiments:

```text
Data Generation
      ↓
Dataset Management
      ↓
Encoder Training
      ↓
Evaluation
      ↓
Error Analysis
      ↓
Optimization
```

The slices remain independently useful. A separate bounded workflow layer may
compose their normal application contracts into a durable local run:

```text
Workflow Orchestration
  -> generation plan/job
  -> immutable snapshot
  -> immutable training-to-benchmark check
  -> training run/checkpoint
  -> development benchmark evaluation
  -> analysis and optional advisory interpretation
  -> reviewed or finitely pre-authorized optimization iteration
  -> explicit sealed acceptance and promotion
```

The workflow layer links slice artifacts; it does not replace their planners,
runners, persistence ports, or provenance.

A declarative preparation layer may compile one strict local manifest into the
ordinary project configuration, evaluation cohorts, contamination reports,
benchmark suites, immutable benchmark-bundle authority, and resolved workflow
definition required to start that workflow. The bundle pins the development and
optional sealed suites plus one clean, zero-tolerance contamination report over
their exact combined cohort population. Preparation is read-only during preview
and atomic during creation. It derives identities and fingerprints from
persisted snapshot evidence; it does not import hidden defaults, run a slice, or
weaken any governance check.

Before governed training starts, the workflow creates or revalidates an
immutable `TrainingBenchmarkCheck` between the exact trainer-visible population
and that benchmark bundle. The current version treats the snapshot's train and
validation members as model-influencing input, persists the combined strict
contamination report, and blocks before either backend startup or completed-run
reuse unless the check is clean. Each later immutable candidate snapshot gets
its own check, and the selected check is pinned through final promotion and
provenance rather than being inferred from a historical model name.

This firewall and deterministic benchmark qualification are the implemented
early milestones of [Benchmark Stewardship](benchmark-stewardship-spec.md).
Preparation now requires an explicit approval review and pins the approved
qualification into the executable workflow authority. A later bounded advisory
benchmark architect may not
weaken deterministic eligibility, expose sealed row content for adaptation, or
turn advisory output into approval authority.

A pilot bootstrap layer may resolve local JSONL/CSV cohort declarations into
ordinary completed imports and immutable all-test snapshots before invoking the
same preparation compiler. Bootstrap preview is read-only. Bootstrap creation
persists its source artifacts and the existing preparation bundle in one local
transaction, fingerprints source contents for idempotency, and never starts the
workflow or calls a provider.

Every arrow crosses an explicit, persisted contract. A later slice consumes
immutable artifacts from the earlier slice and never reaches into its internal
implementation.

An optional authenticity-research capability may run before synthetic data
generation. A bounded Pi agent studies permitted external sources through
application-owned tools and produces an evidence-backed, human-approved,
immutable authenticity profile. Generation consumes only the explicitly bound
resolved profile; it never performs live research. See
[Authenticity Research Agent](research-agent-spec.md).

An optional dataset-architecture capability may then turn the dataset schema,
semantic and authenticity context, current coverage, operator priorities, and
eligible development diagnostics into a reviewable allocation and generation-
strategy proposal. Pi explores proposals through read-only application tools;
the existing deterministic initial allocator remains the feasibility authority.
Only an explicitly approved, non-stale proposal can create a normal Slice 1
plan. See [Dataset Architect](dataset-architect-spec.md).

An optional dataset-qualification capability may audit structurally accepted
generated or imported rows before they become training data. A replaceable
evaluator produces bounded evidence; deterministic policy and explicit human
review produce an immutable curation manifest. Dataset Management remains the
owner of snapshot splitting, while an atomic application links the approved
manifest to the resulting snapshot. See
[Dataset Qualification and Curation](dataset-quality-spec.md).

## Shared invariants

- IDs are globally unique and timestamps use UTC.
- Dataset snapshots and model artifacts are immutable after completion.
- Derived records retain source IDs and configuration sufficient to reproduce
  the operation.
- User-visible counts and metrics are calculated from persisted facts.
- Backend contracts are owned by the platform, not copied from providers.
- A deterministic local implementation and fake are available for ordinary
  development and tests.
- Long-running operations use durable run states and never block an HTTP
  request.
- CLI workflows are complete before equivalent API and UI workflows are added.
- Evidence used for adaptation is explicitly distinguished from sealed
  acceptance evidence. Sealed rows and predictions never enter analysis,
  optimization, prompts, or iteration decisions.
- Automatic cross-slice execution is finite, budgeted, persisted, idempotent,
  cancellable, and either explicitly approved per iteration or constrained by
  a persisted pre-authorization envelope.
- Every long-running workflow stage durably reserves its exact child execution
  identity before startup; cancellation and recovery target those links rather
  than inferring children from mutable plan or presentation state.
- Deterministic benchmark contracts decide pass, fail, inconclusive, or invalid;
  an LLM may provide advisory interpretation but never acceptance authority.
- Every new workflow definition binds one immutable benchmark bundle. Its
  development and optional sealed suites are globally disjoint, and an override
  cannot authorize contamination in their strict global report.
- Every governed initial or iterative training run or completed-run reuse
  requires one clean immutable training-to-benchmark check for its exact
  snapshot, trainer-input protocol, and benchmark bundle.

## Slice definitions

- [Slice 1 — Synthetic Data Generation](slice-1-spec.md)
- [Slice 2 — Dataset Management](slice-2-spec.md)
- [Slice 3 — Encoder Training and Checkpoints](slice-3-spec.md)
- [Slice 4 — Evaluation](slice-4-spec.md)
- [Slice 5 — Error Analysis](slice-5-spec.md)
- [Slice 6 — Optimization](slice-6-spec.md)
- [Authenticity Research Agent](research-agent-spec.md)
- [Dataset Architect](dataset-architect-spec.md)
- [Dataset Qualification and Curation](dataset-quality-spec.md)
- [Benchmark Stewardship](benchmark-stewardship-spec.md)

Cross-slice composition is specified separately in
[Controlled Workflow and Evaluation Governance](workflow-governance-spec.md).

## Current non-goals

The local platform does not include authentication, multi-user collaboration,
cloud deployment, distributed execution, multiple workers, unbounded autonomous
agents, bandits, reinforcement learning, automatic external spending outside a
persisted finite budget, or semantic deduplication. Optimization remains
deterministic recommendation logic. The workflow layer may execute a bounded,
pre-authorized state machine but never an open-ended agent loop.

The training-to-benchmark firewall governs only data visible through the
platform's persisted snapshots and cohorts. It checks exact source-row identity,
exact text, normalized text, and an optional declared group identity. It is not
semantic or embedding deduplication, cannot prove that paraphrases are
independent, and cannot prove that benchmark content was absent from a base
model's pretraining data or from training performed outside the platform.

## Platform completion criterion

The platform is complete when a user can generate labeled examples, freeze an
immutable and reproducibly split snapshot, train a replaceable local classifier
backend with checkpoints, evaluate a chosen checkpoint, inspect persisted
errors by label and arbitrary dimensions, obtain an explicit optimization
proposal, and turn that proposal into a new generation plan without rewriting
an earlier slice. The complete local product additionally permits a user to
define an exact initial row budget, run the slices through a durable governed
workflow, iterate from development evidence within finite limits, and perform a
separate sealed acceptance assessment without leaking sealed evidence back into
adaptation.
