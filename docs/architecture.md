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
workflow-core ──────────> resolved generation-supervision blueprint
project-preparation ─────> project-config + workflow-core + slice artifact shapes
research adapters / apps ─> research-core
generation-core ──────────> research-core (resolved authenticity context only)
benchmark architect adapters / apps ─> benchmark-architect-core
benchmark-architect-core ──> benchmark/evaluation and aggregate research contracts
encoder-experiment adapters / apps ─> encoder-experiment-core
encoder-experiment-runner ──────────> encoder-experiment-core
encoder-experiment-sqlite ──────────> encoder-experiment-core
encoder-repair adapters / apps ─────> encoder-repair-core
encoder-repair-core ────────────────> encoder-experiment-core
dataset architect adapters / apps ─> dataset-architect-core
dataset-architect-core ────> generation/allocation and governed evidence contracts
quality evaluator adapters / apps ─> dataset-quality-core
dataset-quality-core ─────> immutable source-row and semantic guidance contracts
supervisor adapters / apps ─> generation-supervisor-core
generation-supervisor-core ─> generation + quality + approved context contracts
generation-supervisor-runner ─> generation-supervisor-core + agent-runtime-core
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

`benchmark-architect-core` owns immutable benchmark-design briefs, bounded run
policy, research-evidence bindings, deterministic blueprint validation,
append-only reviews, exposure-aware freshness requirements, and the approved
acquisition handoff. It consumes only task semantics and aggregate benchmark,
qualification, and exposure summaries. It does not own benchmark rows,
snapshots, suite construction, contamination, qualification authority,
evaluation, SQLite, Pi, or provider transport. Its handoff cannot become a
workflow authority without the existing ordinary benchmark gates.

`encoder-experiment-core` owns task-neutral immutable production-project,
input, model, candidate, metric, comparison, and finite-budget contracts. A
statically composed task adapter owns native row parsing, training, prediction,
safe model transformations, and metric normalization. An adapter may compose
multiple native evaluators into one complete declared metric contract when a
component-level score does not represent production behavior. Every support
model, reference checkpoint, suite configuration, and native output remains a
content-verified project dependency below the isolated workspace. The core
contains no paths, subprocesses, Python,
CUDA, SQLite, or Nomos types. It extends production task coverage without
rewriting or weakening the independent text-classification slices.

`encoder-experiment-runner` reserves each external action in an append-only,
hash-chained journal before invoking the compiled adapter. It derives status by
replaying that journal, selects only from development assessments, and requires
an explicit sealed authorization event. `encoder-experiment-sqlite` stores
snapshots, protocols, and compare-and-append journal events in dedicated tables;
it does not enter the existing synthetic-data persistence module. Adapter
re-entry adopts only a complete output at the exact immutable candidate or
evaluation path, allowing process recovery without overwriting artifacts.

`encoder-repair-core` owns the provider-neutral adaptive evidence boundary for
production encoder tasks. Native adapters may normalize complete development
rows into text-free observations, but the core accepts only exact development
report bindings and represents no sealed evidence role. It deterministically
derives shared and suite-specific weakness, fixed/regressed/persistent rows,
rank movement, abstention support, minimum-support limitations, and exact
candidate gate trade-offs. It contains no native row types, paths, subprocesses,
SQLite, CLI, model inference, generation, or training logic.
`encoder-experiment-sqlite` implements its narrow append-only evidence-store
port beside the existing experiment and campaign journals. Stable source and
derivation fingerprints are idempotency keys; the adapter deeply verifies the
exact persisted development report, project, campaign, run, normalized
observation bindings, and deterministic diagnosis on every diagnosis read.
The Nomos adapter implements the separate development-observation backend port:
it verifies historical manifest-pinned inputs and exact model trees, executes a
version-pinned native observer, and returns only complete text-free normalized
observations in a content-addressed development artifact. Sealed suites are
unrepresentable in the request and rejected again by the native adapter.
`synth production-repair diagnose` is the thin composition root: it replays one
campaign-linked experiment, collects baseline and candidate observations for
every development suite, persists each set idempotently, and derives one
immutable comparative diagnosis. `show`, `doctor`, and `evidence` read through
the same deep-verifying persistence contracts.

The same core now owns finite repair proposals, without owning their native
execution. A proposal distinguishes the historical project that produced the
diagnosis from the current clean execution project, then pins exact diagnosed
slices, absolute native-row targets, data and training actions, base training
inputs, a zero-or-bounded external-call budget, a task-neutral quality policy,
finite candidate hypotheses, and the active renewable benchmark journal head.
Append-only reviews bind the exact proposal specification. An application is
only an idempotent reservation keyed by the approved specification; native row
qualification and training remain later adapter-owned stages. SQLite deeply
replays both historical diagnosis evidence and the pinned benchmark journal,
while `production-repair propose`, `proposal-show`, `proposal-doctor`, `review`,
and `apply` provide the CLI boundary.

The core also owns the row-free qualification contract for a proposal's native
repair delta. A compiled task adapter may inspect native payload internally,
but it returns only canonical row, content, normalized-content, source, group,
and lineage fingerprints plus task-validation and contamination counts. It also
returns the row-free identity, row count, and identity-set fingerprint of every
base-training, development, and sealed reference used by the audit. The core
requires those references to cover the exact proposal training inputs, requires
one complete assessment for every exact proposal target, derives deterministic
include/exclude decisions from the pinned native quality policy, and permits an
immutable selection only after an eligible report and the latest append-only
human approval. Proposal review freezes at application; delta review freezes at
selection. Every delta artifact, report, review, and selection must be created
before proposal expiry. The core does not reinterpret native retrieval rows as
classification labels and does not retain their text or tool identities.

`encoder-experiment-sqlite` implements this separate quality-store port with
append-only candidate-set, report, review, and selection tables. Deep reads
replay the proposal and application lineage, reproduce all artifact
fingerprints, verify normalized storage envelopes, and freeze the review chain
after selection. The task adapter remains responsible for constructing the
native artifact and proving its hashes against the actual files; later
candidate-snapshot and training stages may consume only the approved selection.
The Nomos implementation is a separate compiled adapter module. It binds its
identity to the deterministic generator, native validator, retrieval text
renderer, and registry sources; writes one immutable content-addressed request;
executes a fixed Python module under the proposal's finite time budget; and
deeply verifies the output manifest, delta, audit evidence, exact project
population, and normalized row evidence before persistence. `delta-build`,
`delta-show`, `delta-doctor`, `delta-review`, and `delta-select` expose this
boundary without adding native payload to the CLI or database.

The next provider-neutral artifact is an immutable logical repair-training
snapshot. It does not concatenate or duplicate large native files. Instead it
binds the exact execution project and baseline, every audited base-training
artifact and identity-set fingerprint, the zero-exclusion approved delta and
its exact selected membership, row counts, and the complete approval lineage.
The core currently fails closed on a partially selected native artifact because
the Nomos trainer consumes whole JSONL inputs. SQLite stores this manifest
append-only and deeply replays all of its dependencies. `training-snapshot-build`,
`training-snapshot-show`, and `training-snapshot-doctor` are the thin CLI
boundary; build and doctor also require the native content-addressed delta to
reproduce from the current isolated project.

`encoder-campaign-core::optimization` is the thin durable parent contract for
one post-review improvement cycle. Its immutable definition binds the exact
project, proposal, native-delta selection, training snapshot, renewable
benchmark authority, metric source, finite budget, selection rule, and
candidate set. Its small hash-chained journal reserves campaign, protocol, and
experiment-run identities before execution. `encoder-experiment-sqlite` stores
that definition and lifecycle append-only. Routine status validates immutable
storage envelopes; launch, native side effects, and Doctor retain the deeper
dependency and checkout checks. `synth encoder optimize` is the CLI composition
root. It advances only one reserved stage per invocation, treats sealed use as
an explicit pause, and derives reports and row-free provenance from persisted
facts. Existing production-repair, experiment, benchmark-generation, and
production-campaign commands remain independently useful.

`dataset-quality-core` owns immutable source-set audit plans, explicit quality
policies, normalized evaluator requests and assessments, deterministic verdicts,
append-only row/manifest reviews, curation proposals, and approved manifests.
It consumes normalized immutable source rows and optional semantic/authenticity
guidance. It does not own source-row acceptance, generation, snapshot splitting,
provider transport, SQLite, training, evaluation, or workflow transitions.

`generation-supervisor-core` owns immutable generation-quality contracts,
deterministic strategy assignments, normalized row/window observations, scoped
quality and drift decisions, protected prompt-guidance revisions, append-only
reviews, canary activation, finite lifecycle/budgets, and child-execution ports.
It composes narrow artifacts from generation and dataset quality but owns
neither implementation. It contains no Pi, SQLite, CLI, HTTP, or provider types.

`generation-supervisor-runner` owns the bounded Pi diagnostic/tool loop. Its
application-owned capability set exposes only persisted quality summaries and
protected guidance preview/submission. The existing generic agent runtime and
Pi process adapter remain replaceable infrastructure. A supervisor run links
ordinary generation segments and quality evidence; it never mutates a job or
copies generation/evaluator business logic.

`generation-supervisor-core` also compiles the small outcome-oriented controls
used by governed workflows into a fully resolved provider-neutral blueprint.
`workflow-core` pins that blueprint and selects legal stages; it does not own or
reinterpret quality thresholds. The CLI resolves the exact plan/coverage-bound
contract, reserves the supervisor run as a workflow child, and hands completion
to the existing qualification finalizer and curation stages.

After a run reaches complete directly qualified coverage, the supervisor runner
may compile one immutable qualification handoff. The handoff selects exact
source-row fingerprints and retains every excluded observation and reason. A
local replay adapter rebinds the selected persisted assessments into the normal
Dataset Qualification audit/report/curation contracts with zero provider I/O.
It does not approve membership. The existing append-only manifest review and
ordinary snapshot builder remain the sole training-data admission boundary.
Snapshot provenance links back through the qualification application, handoff,
supervisor run/contract, prompt/strategy evidence, and original assessments.

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
benchmark-bundle authority, the versioned `TrainingInputProtocol` and
`TrainingBenchmarkCheck` firewall, durable workflow state, approval envelopes,
stop decisions, and the ports required to request or inspect ordinary slice
artifacts. A bundle binds the development and optional sealed suite fingerprints
to one zero-tolerance report over their exact cohort union. A training check
then binds one immutable snapshot's exact trainer-visible population to that
bundle and a separately persisted strict combined contamination report. It must
not import SQLite, CLI, provider, Candle, Axum, or adapter types. The CLI
application assembles the concrete slice runners and workflow ports; SQLite
implements workflow persistence in feature-owned adapter modules.

Historical bundle reads verify the immutable pinned suite, role-decision,
report, snapshot, and member evidence without pretending that a past role
decision is still current. Executable workflow loading adds the stricter
requirement that each pinned role decision is the current active decision. The
CLI repeats that check before every running stage and derives all development or
sealed suite use from the loaded bundle authority.

The workflow application constructs or reloads the training check before it
queries for a reusable completed training run and before it starts a trainer.
For `train_and_validation_v1`, every non-empty train or validation split is
represented by one active internal cohort with a pinned `Training` role
decision; test members are outside that versioned trainer-input contract. The
SQLite adapter atomically persists any new training cohorts and roles, the
combined contamination report, and the immutable check. Executable reload
revalidates current roles, while historical reload retains the originally
pinned evidence. A clean check is linked to later evaluation, sealed assessment,
and promotion stages so checkpoint reuse cannot bypass the firewall.

The training runner seals the example source before a backend receives it. It
captures one fingerprint per ordered example, reproduces the input binding,
and wraps subsequent backend reads in per-position fingerprint checks. A
mutable or nondeterministic source therefore cannot pass verification and then
serve different text to the trainer. Training persistence independently
enforces queued-only creation, legal one-batch lifecycle transitions, and an
exact final checkpoint before completion. Promotion persistence derives the
decision again from the selected final checkpoint, its completed run and input
binding, the clean check, and both checkpoint-bound assessments.

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

### `benchmark-architect-core`

Owns the provider-neutral Benchmark Architect boundary. Pi may research
permitted sources, inspect only aggregate current-benchmark facts, preview one
explicit blueprint through deterministic validators, and submit advice. The
core rejects unsafe sealed access, invalid protocols/contracts, statistically
unsupported cohorts, unbounded freshness, and unsupported evidence references.
An approved proposal creates only an immutable acquisition handoff.

### `dataset-quality-core`

Owns the provider-neutral Dataset Qualification boundary. It fingerprints the
complete candidate source set, validates exact blind label and dimension score
shapes, derives verdicts from integer policy thresholds, and compiles reviewed
evidence into a complete immutable selection. Unevaluated rows are excluded in
V1. A quality evaluator cannot rewrite data or decide snapshot membership.

### `encoder-repair-core`

Owns immutable complete development-observation sets and reproducible
comparative diagnosis and reviewed finite repair proposals for
non-classification production encoder experiments.
Observation sets retain exact project, campaign, run, model, suite, report,
observer, and source-artifact authority while excluding row text and native
labels. Diagnosis consumes only those development artifacts and immutable
aggregate assessments. Repair proposals pin historical and current project
identities, exact weakness slices, actions, inputs, budgets, quality policy,
candidate hypotheses, expiry, and the active benchmark-generation head.
Application remains an immutable review-gated reservation, not native data or
training execution. Its separate native-delta quality contract owns payload-free
row evidence, deterministic zero-or-bounded contamination policy, append-only
review, and an immutable approved selection. Native schema validation and file
inspection remain behind the compiled task adapter.

### `generation-supervisor-core`

Owns the optional generation-quality supervision contract: exact quality
requirements, strategy assignment and coverage, deterministic immediate-
weakness and longitudinal-drift decisions, immutable prompt-guidance revision
and review, canary activation, finite run state, and persistence/child-executor
ports.

### `generation-supervisor-runner`

Runs the bounded Pi diagnostic capability through `agent-runtime-core`,
validates every tool request against persisted supervisor authority, and emits
only normalized core revision or escalation artifacts. It contains no
persistence or generation-backend implementation.

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

The governed training-to-benchmark check is deliberately outside
`training-core`: it is cross-slice policy owned by `workflow-core` and assembled
by the workflow application. `training-core` owns only an opaque
`TrainingInputBinding`: protocol, population identity/count, a digest reproduced
from the exact ordered labels and train/validation examples visible through the
backend port, and an external authority identity/fingerprint. The runner
requires the request and persisted run to carry the identical binding and
reproduces the backend-facing digest before startup. Trainer adapters receive
only the authorized training examples and never benchmark rows, suite types, or
contamination reports. The gate runs before both backend construction and
completed-run reuse.

Standalone CLI training pages immutable snapshot members into an ephemeral
local spool. Governed training first loads and verifies complete snapshot
membership, reproduces the check's population fingerprint, and builds the spool
from that same in-memory member set so a database reread cannot change the
trainer input. The
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

Its `TrainingBenchmarkCheck` constructor separately validates the immutable
train-plus-validation population against that bundle under the same group
identity and default zero-tolerance policy. The resulting clean or blocked
artifact pins snapshot, population, training cohorts and roles, bundle,
benchmark cohort set, combined report, protocol version, and fingerprint. A
blocked check remains durable evidence but cannot authorize training.

Its benchmark-qualification policy separately answers whether the exact bundle
population can support its declared decisions. The pure calculation consumes
suite contracts and snapshot members, derives only aggregate support,
distribution, source-composition, duplication, text-bound, and uncertainty
facts, and emits normalized blocking issues and honest limitation warnings.
It reuses evaluation slice identities and the platform text normalizer; it does
not calculate model metrics, approve a benchmark, or treat balanced labels as
proof of representativeness.

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

The workflow attempt separately owns immutable child-execution links. A link is
written before the reserved generation job, quality audit, training run, or
per-cohort evaluation begins and records an ordered logical slot plus exact
child ID. SQLite permits insertion only for the current uncancelled running
attempt and rejects updates or deletion. Cancellation closes the parent first,
then targets these links; it no longer discovers work by broad plan queries.
Recovery can therefore resume the linked generation job in place or identify
the exact interrupted training/evaluation child and its replacement.

Snapshots, registered base-model bundles, resolved configurations, evaluation
inputs, analysis protocols/reports, optimization evidence/proposals, reviews,
benchmark suites, global and training-combined contamination reports, benchmark
bundles, training-benchmark checks, campaign links, promotions, and outcomes
have deterministic SHA-256 fingerprints. A workflow definition pins its exact
bundle binding, the bundle pins suite and global-report identities, and each
training check pins the exact trainer-visible snapshot population plus its
combined report. New promotions pin the selected clean check ID and fingerprint.
Provenance therefore resolves a training check to its snapshot, bundle, and
combined report, and resolves a promotion through that check; legacy promotions
without the nullable pin remain readable and simply lack that edge. Floating
point inputs are normalized through their persisted JSON representation before
hashing so identities reproduce after reload. Checkpoint
bytes are checksum-verified before every production load. Provenance traces are
built from persisted foreign keys and source-row provenance, not presentation
state.

Workflow provenance treats stage links as shallow immutable references instead
of recursively embedding each linked artifact's complete ancestry. Tracing the
linked artifact still resolves its full owner-specific graph. This keeps a
workflow run usable as an index over a long iterative history without creating
an exponentially repeated provenance tree.

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
