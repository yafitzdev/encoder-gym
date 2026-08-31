# Platform architecture

## Dependency direction

```text
generation-cli ───────────────┐
generation-server ─────────────┼──> generation-core
generation-sqlite ────────────┤
generation-fake ───────────────┤
generation-openai-compatible ─┘

generation-core ──> no infrastructure or presentation crate
```

Later slices follow the same inward dependency rule:

```text
apps ───────────────────────────────────────────────────────────┐
SQLite and model adapters ──────────────────────────────────────┤
                                                                ↓
dataset-core → training-core → evaluation-core → analysis-core → optimization-core
```

Cross-cutting contracts remain small and inward-facing:

```text
apps / SQLite adapter ──> project-config
apps / SQLite adapter ──> recovery-core
slice cores / adapters ──> artifact-core
agentic cores / Pi process adapter ──> agent-runtime-core
generation-core / workflow-core ──> semantic-catalog
dataset-import ─────────> dataset-core + generation-core
workflow-core ──────────> narrow artifact contracts from slice cores
project-preparation ─────> project-config + workflow-core + slice artifact shapes
research adapters / apps ─> research-core
generation-core ──────────> research-core (resolved authenticity context only)
dataset architect adapters / apps ─> dataset-architect-core
dataset-architect-core ────> generation/allocation and governed evidence contracts
quality evaluator adapters / apps ─> dataset-quality-core
dataset-quality-core ─────> immutable source-row and semantic guidance contracts
```

`artifact-core` only canonicalizes fingerprint inputs and describes provenance
trees. `recovery-core` only describes process leases and interruption records.
Neither crate orchestrates slice business logic.

`agent-runtime-core` is the small product-neutral interactive boundary shared by
bounded agentic capabilities. A request selects one versioned capability set;
the Pi sidecar owns the corresponding tool schemas while each product runner
still validates and executes every request. The generic contract grants no tool
by itself and contains no research, planning, persistence, or Pi types.

`semantic-catalog` owns immutable reusable/dataset-scoped profiles, append-only
bindings, deterministic layered resolution, and the provider-neutral resolved
context. It imports only `artifact-core`; it knows no prompt, provider, SQLite,
CLI, workflow, or dataset implementation. Generation prompt construction and
the bounded workflow advisor consume its resolved context through their own
contracts. SQLite stores the artifacts in the existing local database.

`research-core` separately owns strict research briefs, finite budgets,
evidence/claim integrity, durable lifecycle rules, authenticity profiles,
append-only reviews, approved bindings, and the normalized resolved context
consumed by generation. It imports only provider-neutral libraries and
`artifact-core`. Pi, search, fetching, SQLite, process protocols, CLI parsing,
and prompt rendering remain adapters. Pi is the required reasoning/tool-loop
runtime, but Pi types never cross the core boundary.

`dataset-architect-core` owns immutable planning briefs, bounded run policy,
agent proposal validation, strategy directives, append-only reviews, and the
approved application contract. It consumes normalized semantic, authenticity,
coverage, deterministic-allocation, and governed diagnostic artifacts. It does
not own generation-planning mathematics, prompt transport, persistence, Pi, or
sealed evidence. An architect proposal is untrusted until the ordinary initial
allocator reproduces it and an operator approves it.

`dataset-quality-core` owns immutable source-set audit plans, explicit quality
policies, normalized evaluator requests and assessments, deterministic verdicts,
append-only row/manifest reviews, curation proposals, and approved manifests.
It consumes normalized immutable source rows and optional semantic/authenticity
guidance. It does not own source-row acceptance, generation, snapshot splitting,
provider transport, SQLite, training, evaluation, or workflow transitions.

Dataset Management remains unchanged. The application verifies an approved
manifest, passes only its included rows to the ordinary snapshot builder, and
atomically persists a curation application beside the snapshot. Legacy
snapshots without such an application retain their existing fingerprints and
behavior.

After approval, `dataset-architect-core` compiles the advisory output into two
ordinary immutable inputs: a `GenerationPlan` and a provider-neutral
`ResolvedGenerationStrategyContext`. `synthetic-data-core` owns that normalized
strategy shape and prompt scoping. A generation job pins it beside semantics,
authenticity, construction, prompt-template, and backend identities. The job
runner never imports or invokes the architect runner.

`workflow-core` owns only cross-slice policy: initial finite allocation,
evaluation roles and exposure rules, benchmark acceptance contracts, immutable
benchmark-bundle authority, durable workflow state, approval envelopes, stop
decisions, and the ports required to request or inspect ordinary slice
artifacts. A bundle binds the development and optional sealed suite fingerprints
to one zero-tolerance report over their exact cohort union. It must not import
SQLite, CLI, provider, Candle, Axum, or adapter types. The CLI application
assembles the concrete slice runners and workflow ports; SQLite implements
workflow persistence in feature-owned adapter modules.

Historical bundle reads verify the immutable pinned suite, role-decision,
report, snapshot, and member evidence without pretending that a past role
decision is still current. Executable workflow loading adds the stricter
requirement that each pinned role decision is the current active decision. The
CLI repeats that check before every running stage and derives all development or
sealed suite use from the loaded bundle authority.

The generation core owns deterministic Cartesian cardinality and expansion,
including the local cell-count safety invariant. Workflow allocation consumes
those cells, compiles partial operator selectors into exact constraints, and
derives explainable group summaries. CLI parsing, SQLite persistence, and
provenance traversal remain adapters around those pure operations.

The same core owns the provider-neutral hybrid row-construction compiler. A
fingerprinted field-recipe graph deterministically prepares row seeds, exposes
only unresolved semantic fields to prompt construction, and merges normalized
backend output without trusting it for cell identity. The job runner records
both provider requests and local deterministic batches through the durable
attempt contract. SQLite stores plans and field-level traces but does not
evaluate recipes; provider adapters receive normalized requests and do not know
how coverage, persistence, or deterministic fields work.

`project-preparation` owns the strict operator manifest and its pure compiler.
It may construct ordinary domain requests and an atomic persistence bundle, but
it cannot execute generation, snapshotting, training, evaluation, analysis, or
optimization. The compiler receives already-loaded immutable snapshot evidence
through its own input shape; it has no SQLite or CLI dependency. It computes
suite-local reports plus a separate zero-tolerance global report, builds the
benchmark bundle, and pins that bundle in both the workflow definition and
preparation summary. SQLite owns the one transaction that inserts the compiled
ordinary artifacts, report, bundle, definition, and small preparation record.
Repeating the same manifest returns that record by stable manifest fingerprint
instead of creating another project.

The optional pilot-bootstrap contract in the same core describes local cohort
source declarations and the resolved ordinary import/snapshot artifacts needed
by preparation. File reading and JSONL/CSV mapping stay in `dataset-import` and
the CLI adapter. SQLite atomically inserts those source artifacts, the existing
preparation bundle, and a small content-fingerprinted bootstrap record. The
bootstrap layer does not introduce an alternate importer, snapshot builder,
benchmark compiler, or workflow engine.

The arrows between core crates describe artifact consumption, not access to
another slice's implementation. Training consumes normalized snapshot examples;
it does not query generation jobs.

Each slice core owns its domain models, pure logic, application workflows, and
the ports required from infrastructure. Adapter crates implement those ports.
Executable applications assemble concrete adapters and invoke core workflows.

Dependencies must point inward. Core code must never import an adapter or
application executable.

## Workspace members

### `synthetic-data-core`

Owns Slice 1 business rules and project-owned interfaces. Its modules are
organized by a clear reason to change rather than by generic technical labels.

### `semantic-catalog`

Owns reusable semantic profile and explicit binding contracts. It validates and
fingerprints immutable versions, resolves reusable guidance before
dataset-specific overrides, and exposes a small persistence port. It does not
perform automatic attachment or model calls.

### `research-core`

Owns authenticity-research policy and immutable handoff contracts. It treats
external content as untrusted evidence, requires finite persisted budgets and
explicit profile approval, and exposes replaceable agent/search/fetch/store
ports. It neither generates dataset rows nor knows a generation backend.

### `dataset-architect-core`

Owns the provider-neutral Dataset Architect boundary. Pi may inspect pinned
facts, compare deterministic allocation previews, estimate a generation budget,
and submit one explicit proposal. The core rejects missing or duplicate cells,
infeasible totals, unknown strategy selectors, stale coverage, and any sealed
or retired diagnostic evidence before a normal generation plan can be created.

### `dataset-quality-core`

Owns the provider-neutral Dataset Qualification boundary. It fingerprints the
complete candidate source set, validates exact blind label and dimension score
shapes, derives verdicts from integer policy thresholds, and compiles reviewed
evidence into a complete immutable selection. Unevaluated rows are excluded in
V1. A quality evaluator cannot rewrite data or decide snapshot membership.

### `synthetic-data-sqlite`

Implements persistence ports with SQLite and owns migrations and SQL-specific
mapping. It must not decide what should be generated.

### `synthetic-data-openai-compatible`

Implements the generation-backend port through an OpenAI-compatible HTTP API.
It owns provider transport and response normalization, but not prompt policy,
validation, coverage, or persistence.

### `synthetic-data-test-support`

Re-exports deterministic test adapters and provides fixtures shared by
integration tests. It is not part of production decision-making.

### `synthetic-data-fake`

Implements the backend port with deterministic local rows. It is a production
adapter so the CLI and server can exercise every workflow without network
access, and it remains useful as a test dependency.

### `synthetic-data-cli`

Parses terminal input, invokes application workflows, renders results and
progress, and converts process signals into cancellation requests. Business
logic does not belong here.

### `synthetic-data-server`

Exposes the same application workflows through a small Axum API and serves the
primitive static developer UI. It assembles one in-process worker guarded by a
single permit; it does not contain business rules or provider-specific types.
This executable remains a Slice 1 surface while later slices are developed
through their cores and the CLI.

### `dataset-core`

Owns immutable snapshot models, deterministic stratified split assignment,
snapshot statistics/export, and accepted-row/source and snapshot-store ports.
It knows no generation backend or SQLite type.

### `dataset-import`

Streams JSONL and CSV records through configurable field mapping and composable
validation. It is a format adapter; accepted/rejected row ownership and import
provenance remain in `dataset-core`, and SQLite batching remains in the
persistence adapter.

### `project-config`

Owns strict, versioned TOML shapes, defaults, CLI override resolution, domain
validation, and deterministic configuration fingerprints. It stores only the
name of an API-key environment variable, never the key itself.

### `artifact-core` and `recovery-core`

`artifact-core` provides canonical SHA-256 fingerprints and the provenance port.
`recovery-core` provides workflow-kind, process-lease, interruption, and
resolution contracts. SQLite implements both without leaking SQL or process
inspection into slice domains.

### `training-core`, `training-linear`, and `training-transformer`

`training-core` owns run/checkpoint models, the trainer and predictor ports, and
the durable runner. Its example-source contract permits bounded reads without
coupling a backend to SQLite or a filesystem format. `training-linear` is the
replaceable deterministic CPU adapter and owns the local immutable checkpoint
store. `training-transformer` is the Candle/Tokenizers adapter for the exact
local BERT bundle described in ADR 001. Candle tensors, tokenizer encodings,
BERT configuration types, and optimizer internals do not cross into core.

The CLI pages immutable snapshot members into an ephemeral local spool. The
transformer backend keeps member IDs for deterministic ordering and loads only
the active text batch; it never holds the complete tokenized dataset.

### `evaluation-core`

Owns persisted evaluation runs and predictions, exact classification metrics,
dimension slicing, comparisons, and the predictor-driven runner.

### `analysis-core`

Owns the normalized analysis protocol, collision-safe finding identities,
bounded streaming aggregation, uncertainty summaries, overlap-aware ranking,
paired-comparison diagnosis, immutable reports, and the read ports it needs
from persisted evaluation evidence. Its first pass uses memory proportional to
observed groups and configured evidence limits; a second bounded pass assigns
each error to its earliest ranked finding for exact marginal and cumulative
coverage. It has no model or SQLite dependency.

The project-owned `DiagnosticContract` is the only analysis shape Slice 6 needs
for cell recommendations. It carries canonical cell identities, complete
bounded metrics, the latest append-only review disposition, evaluation and
comparison identity, and reproducible per-cell and whole-contract
fingerprints. Optimization does not consume SQL rows, CLI types, or
aggregation internals.

### `optimization-core`

Owns the immutable decision protocol/evidence envelope, auditable scoring,
finite constrained allocation, normalized data/review recommendations, typed
bounded training candidates, scenario comparison, append-only review rules,
safe plan conversion, campaign compatibility, and deterministic outcome
assessment. It is advisory: persistence and explicit CLI application are
separate from generation and training execution.

### `workflow-core`

This cross-slice core is introduced by the controlled-workflow phase. It owns a
finite state machine and governance policies, not the implementation of any
slice. Its `BenchmarkBundle` constructor is the authority for global benchmark
disjointness: the development and sealed suite identities and their
snapshot/split evidence must be distinct, and the exact combined cohort set must
have a clean zero-tolerance contamination report with no override. Workflow
artifact links contain identities, fingerprints, bounded state, and
compatibility facts rather than copied datasets, predictions, checkpoints, or
proposal payloads.

The dependency direction is deliberately outward from the workflow core's
ports:

```text
workflow-core
  -> project-owned generation/snapshot/training/evaluation/analysis/optimization
     artifact shapes or narrow workflow-facing summaries

generation-cli + generation-sqlite
  -> workflow-core ports
  -> concrete existing slice runners/stores
```

The workflow core may decide which legal stage comes next. It cannot construct
a provider request, assign snapshot members, train tensors, calculate evaluation
metrics, aggregate predictions, score optimization evidence, or write SQL.

Slice 6.1 decisions begin with a canonical, validated optimization protocol.
Every new proposal embeds the resolved protocol and its fingerprint so later
scoring, allocation, approval, and application behavior cannot drift with
mutable defaults. Historical proposal payloads remain explicit legacy
artifacts without a protocol. See [ADR 004](adr-004-immutable-optimization-protocol.md).

Before scoring, `OptimizationEvidence` binds that diagnostic contract to the
immutable dataset definition, source snapshot, canonical accepted-cell
coverage, optional comparison, optimization protocol, and, when requested,
the exact training-configuration space. This optimization-owned envelope is
fingerprinted and contains no persistence or presentation types.

`optimization-core` consumes narrow artifact shapes from generation, dataset,
training, evaluation, and analysis cores. It does not import their runners,
SQLite adapters, CLI parsing, backend implementations, or provider types. A
training configuration space is bound to a completed run, final checkpoint,
snapshot, and supported registered backend. Campaigns only validate and link
already-created artifact identities; they cannot schedule or poll workflows.

SQLite stores the complete immutable JSON artifacts plus normalized protocol,
coverage, decision, recommendation, training-candidate, review, application,
scenario, campaign-link, and outcome relationships needed for filtering and
integrity audits. Decision-grade application atomically inserts the ordinary
generation plan and its application marker.

### `project-preparation`

This core turns an operator-authored manifest into a deterministic preview or a
validated preparation bundle. The manifest embeds the strict project
configuration, exact workflow budgets/policies, and development/diagnostic/
sealed cohort source declarations. Benchmark sources are immutable persisted
snapshot IDs and splits; the application loads their source datasets and
members before invoking the compiler.

Preview exposes exact allocation cells, provider-request ceilings, stage graph,
training choice, evidence disclosures, approval boundaries, and contamination
results without generating UUID-bearing artifacts or writing state. Creation
resolves all intermediate IDs and fingerprints, rejects duplicate evidence use,
unknown labels, empty splits, any global contamination, unsafe sealed
disclosure, and incompatible workflow budgets, then hands one atomic persistence
bundle containing the benchmark bundle and its workflow binding to the port. It
never shells out to existing CLI commands or copies slice algorithms.

## Durability and provenance

Generation, training, and evaluation runners register a lease containing the
owning PID and operating-system process start time. Startup reconciliation only
marks a `running` workflow interrupted when that exact process identity is no
longer alive. Generation is eligible for in-place resume because planning reads
persisted accepted coverage. Its immutable execution specification pins the
initial needs, backend identity, parameters, policy, prompt template, and
semantic context. When a dataset has an explicitly approved authenticity
binding, the same specification also pins a job-specific resolved context and
the normalized research-excerpt novelty guard. Only abstract profile guidance
enters prompt construction; evidence excerpts remain validation-only inputs.
Every provider call has an append-only lifecycle record;
attempt completion, rows, accepted source membership, and reconciled counters
are one SQLite transaction. An open request becomes `interrupted` during
recovery and still consumes the cumulative attempt ceiling because its remote
outcome is unknowable. Training and evaluation preserve interrupted history and
are not silently replayed under the same artifact identity.

Snapshots, registered base-model bundles, resolved configurations, evaluation
inputs, analysis protocols/reports, optimization evidence/proposals, reviews,
benchmark suites, global contamination reports, benchmark bundles, campaign
links, and outcomes have deterministic SHA-256 fingerprints. A workflow
definition pins its exact bundle binding, and the bundle pins suite and report
identities plus fingerprints. Floating point inputs are normalized through their
persisted JSON representation before hashing so identities reproduce after
reload. Checkpoint
bytes are checksum-verified before every production load. Provenance traces are
built from persisted foreign keys and source-row provenance, not presentation
state.

Analysis findings and representative prediction links are normalized in
SQLite. Human finding reviews are append-only children; changing review state
never changes or refingerprints the immutable report.

Transformer checkpoints are atomically written, immutable single-file
containers with a JSON manifest plus safetensors for model and AdamW state.
Continuation creates a new run linked to its parent checkpoint; interrupted
runs are never silently resumed under the same identity.

Cross-slice workflow stages use durable attempts and idempotency keys. A stage
that already produced a verified ordinary slice artifact links that artifact on
recovery rather than producing a duplicate. Workflow recovery never infers an
approval, repeats a sealed evaluation, or silently consumes a fresh provider
budget.

Snapshot cohorts carry append-only evaluation-role decisions and exposure
records. Development evidence may flow into analysis, optimization, and an
optional advisor; sealed evidence cannot. Sealing is an application-enforced
scientific-governance boundary for a local tool, not cryptographic access
control.

## Interface strategy

Rust traits define replaceability boundaries for generation and persistence.
They are application ports, not provider abstractions copied from external
SDKs. Async ports must remain usable behind `Arc<dyn Trait>`; use an explicitly
boxed future or a narrowly contained compatibility helper if native async trait
syntax is not dyn-compatible.

Dynamic-library plugin loading is not required. Generation, training, and
prediction implementations are adapter crates selected during application
composition.

The optional analysis advisor follows the same rule. `workflow-core` owns a
provider-neutral structured contract; a deterministic fake and an
OpenAI-compatible adapter live outside the core. Prompt policy remains separate
from transport, raw credentials remain environment-only, and advisor output is
untrusted advisory evidence.

## UI strategy

The existing graphical UI covers Slice 1 only. Slices 2–6 and cross-slice
workflow orchestration remain CLI-only until
the user explicitly starts a separate UI phase. Any future UI communicates only
with an API and never imports or reimplements core rules.
