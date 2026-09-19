# Platform architecture

## Managed project custody

`project-workspace-core` owns portable project identities, baseline inventories,
dataset custody metadata, the project activity-event contract, and pure validation. `project-workspace-local` owns
safe local copying, format inspection, hashing and project-bound SQLite.
The workspace core's model/dataset link consumes only Dataset Management's
immutable version artifact. Local persistence verifies manifest/source custody
and pins that version without rewriting model artifacts or native run snapshots.
`synth workspace` uses that adapter without opening the global synthetic-data
store. Imported assets do not imply admitted snapshot membership, training
execution, evaluation policy, or scientific approval. Existing slices retain
those responsibilities. See `features/workspaces/managed-workspaces-spec.md` and
`features/workspaces/managed-workspaces.md`.

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

`encoder-optimization-core` owns the bounded input-first Agent inspection and
edit-proposal contracts. `encoder-optimization-runner` hosts one reserved Pi
turn at a time through `agent-runtime-core`; it validates inspected evidence and
row references rather than trusting model output. `project-workspace-local`
implements its append-only call journal and cumulative reservation accounting.
The Nomos adapter owns development-only saved diagnostic inspection and compact
native training-row projection. Its version-2 inspection builds a deterministic
dataset landscape by scanning every pinned training row, grouping rows by the
same dimensions emitted by the native evaluator, and joining coverage to current
and original-baseline development metrics. The Agent sees ranked aggregate
clusters first and can then request content-hash-selected representative rows;
the generic runner never learns Nomos row or metric shapes. Native validation,
dataset publication, training and deterministic benchmark decisions remain with
their existing slice owners.
The generation slice also owns a narrow structured-output transport port for
task-native schemas. Its existing OpenAI-compatible adapter implements one
bounded request without hidden retries; Nomos owns prompt construction and
native admission. The optimization runner reserves and dispatches bounded
concurrent batches through that port. The project adapter journals attempts,
usage and admission outcomes, then composes ordinary imports and dataset
fork/revision operations into a replayable publication receipt. It does not
reinterpret native labels or grant training qualification. The CLI composition
below reuses these components without copying their policy or implementations.

`project-workspace-core::optimization_iteration` binds prepared run inputs to
exact development report references and an `encoder-optimization-core` Agent
scope. The local adapter stores that first-iteration record append-only before
Agent dispatch; the CLI loads its original scientific protocol through the
normal read-only store. The serialized record excludes the protocol payload
and sealed scores. Further iterations must extend this contract with recorded
completion/selection lineage rather than accepting arbitrary scope replacements.

The workspace CLI's `optimization_runs::agent_dataset` composes this first input
record with the real Agent runner, generator and dataset-publication adapters.
Its inspection adapter loads the exact native development reports and verifies
the selected training sources once per invocation, serving bounded in-memory
pages thereafter. Provider transport is resolved only from the run's pinned
catalog; no new discovery or mutable role-default lookup occurs. The command
returns a dataset publication, not training authority. `prepare-candidate`
continues through iteration-owned native materialization and task-owned
full-population clearance. The fixed native audit reuses Nomos's validator and
input renderer and emits only aggregate facts; its request and clearance pin
the complete dataset and benchmark inputs. This step checks relevant code/data
identities without rereading model weights. `complete-iteration` continues this
same composition through native training and all development suites. The
workspace core owns immutable training and journal-derived result bindings;
the local adapter checks publication lineage, exact sample membership and
append-only persistence. Quick-test sampling follows full-population clearance,
and the native adapter verifies effective training settings in its receipt.
Each adaptive protocol has zero sealed allowance. Reservation accepts Agent
authority, but legacy materialization still rejects it. This first-cycle CLI
path is also reused by the finite `drive-agent` CLI coordinator. New Overview
launches explicitly request its core-owned Agent defaults; historical fixed-run
launches keep their original executor.

`project-workspace-core::optimization_loop` owns completion, cumulative edit
charges and selection using the existing experiment-core development ranking.
The local completion table is append-only and links an exact proposal call,
iteration inputs and result. Ordered reads reproduce selection from persisted
facts; advancing also verifies each original scientific journal through the
normal store. Later Agent inputs bind the preceding result and best eligible
full dataset without changing the original baseline, benchmark or model
starting checkpoint. The CLI bounds continuation by the immutable iteration
limit and reuses the same inspection, generation, qualification, training and
registration functions. Overview consumes the same ordered iteration/report
custody, with independent historical-stage navigation and ordinary artifact links.
The offline connected Electron acceptance invokes this production composition
through real IPC and Pi sessions, not a substituted desktop drive.

Repair-plan history is a passive projection, not another executor. Optimization
core reuses the protocol-specific inspection replay contract to validate saved
decisions and aggregates only their exact generation slots. The local workspace
adapter captures iteration bindings and receipts in one short SQLite snapshot,
releases its read lock, then verifies custody and publication against ordinary
Dataset Management membership and source lineage. It never opens another
connection under that snapshot, avoiding a reader/pending-writer lock cycle.
The CLI asks the Nomos adapter to project bounded saved cluster/development
evidence before it
crosses the desktop boundary; opaque native inspection content is never a UI
contract. Requested edits, native admission, actual publication and later
development acceptance remain separate facts. No read path migrates, recovers,
dispatches a provider or changes run authority.

`project-workspace-core::optimization_final` binds post-loop final consent to
the last completion and its development-selected full-data checkpoint. It
consumes the existing completion and scientific replay contracts, not a new
selection algorithm. `project-workspace-local::optimization_final` revalidates
all completed journals and persists one append-only grant per root, with
schema upgrade and grant creation under the same SQLite write transaction.
The CLI's separate preview/authorize/read commands use the pinned scientific
store and benchmark. Preview is read-only even before the consent migration;
the serialized handoff contains identities, never sealed scores or rows. It
does not alter the adaptive protocols, dispatch final evaluation or promote.
Final execution/recovery is composed separately by the CLI. The experiment-core
backend port provides read-only completed-evaluation recovery; Nomos shares its
normalization path with dispatch but never fills missing evidence in recovery
mode. The workspace core binds reserved report/assessment IDs and reuses
experiment-core assessment policy. The local adapter atomically reserves one
dispatch and stores a row-free immutable final receipt. Native scientific reports
remain outside presentation custody; completed CLI reads reconstruct their
normalized identities and deterministic verdict. The original adaptive journal
and zero-sealed protocol remain unchanged.

`project-workspace-core::optimization_final_promotion` validates the selected
development result, full-data training binding, final decision and exact
registered model lineage. The explicit `promote-final-agent` CLI command
reconstructs final acceptance from saved native evidence and uses the existing
compare-and-append baseline promotion contract. It reuses the registered model;
it neither dispatches evaluation nor changes scientific history. A typed final
decision locator lets ordinary runtime rebinding resolve that accepted model.
The desktop's strict, row-free final bridge pins a main-process review token,
keeps consent retry identity, shares the project execution lock and propagates
Stop through preflight and dispatch. No Optimize or read path invokes it
automatically. The Report panel explicitly reviews and authorizes one evaluation,
recovers the exact saved grant after restart, and separately offers promotion only
after final acceptance. Unknown outcomes cannot acquire another allowance.
Quick-test reports exclude these controls, using the persisted launch authority.

`project-workspace-core::optimization_execution` owns only the coordinator's
execution-attempt transitions and a terminal-iteration reference, not another
optimization algorithm. The project adapter persists that hash-chained journal
beside the unchanged fixed-recipe root journal. Ordinary root reads validate
the completion/iteration lineage through non-recursive database readers before
projecting Agent-specific states. The CLI acquires the existing exclusive
execution lease before recovering an abandoned attempt. Provider reservations
also reject closed execution attempts inside their database transaction. Stop
adds ordered requested/paused transitions without changing permanent Cancel.
Each Stop uses a stable command UUID; retries reuse that exact event. Distinct
commands change the execution head even while paused. Resume pins the observed
head, and the local transaction compares that same snapshot before starting,
so a stale Resume cannot consume a newer Stop. Retrying an old Stop after a
successful Resume cannot stop the new attempt.
The CLI holds the same lease while unwinding work; its explicit reconcile
command obtains that lease before recording interruption and never starts work.
Scoped cancellation probes stop local/native file reads and owned native child
processes. The experiment backend's provider-neutral Stop predicate prevents
new scientific steps and preserves resumable journal state on interruption,
rather than recording an artificial candidate failure. No subprocess, SQLite
or provider types enter core. The renderer supplies the paused head it displayed
through typed IPC; main-process refresh never substitutes a newer Stop head.
The separate post-loop final handoff never reopens adaptive execution. Connected
desktop acceptance covers Stop/restart/Resume and final-receipt recovery; Windows
process tests additionally kill the real coordinator at nine artifact boundaries.

`encoder-experiment-core::training_budget` owns cumulative native-training time
and its narrow reserve/finish port. `project-workspace-local` implements this
port with append-only reservations and completions linked to immutable iteration
training custody. The CLI attaches it to the ordinary Nomos backend after that
custody is recorded. Native dispatch uses only the remaining grant and settles
elapsed time after the owned process has stopped; unknown outcomes retain the
full grant. Reusing verified artifacts bypasses dispatch, not input validation.
No candidate/protocol identity changes on retry. The read-only `training-time`
command reports the same persisted facts. Budget/accounting errors propagate
without a false scientific rejection. Native time-limit and exhaustion outcomes
close the root lifecycle with a distinct non-retrying budget-stop projection;
accounting-integrity failures remain recoverable execution failures.

The local execution lease also records exact PID/start-time identities for
observed child processes. Stale-owner takeover terminates and waits for those
exact descendants before replacing the lease, while status reports a surviving
child as orphaned rather than claiming that work has stopped. This recovery
mechanism remains operational custody in the CLI adapter and does not leak
process types into domain contracts. Stale-owner inspection and replacement
are serialized by a separate per-run SQLite write lock. That lock database is
not deleted, avoiding split lock identities between concurrent recoverers.
On Windows, the CLI joins one anonymous kill-on-close Job Object before starting
application work. Its non-inherited handle lives until process teardown; job
membership is inherited at child creation, including by grandchildren. Thus
abrupt coordinator death also stops unobserved or reparented children. After
the finite Agent coordinator returns, it drains remaining job members before
closing the root attempt or acknowledging Stop. Cleanup opens process handles
and rechecks membership, avoiding numeric-PID termination races. Legacy lease
cleanup verifies creation time on the same handle it terminates. Child identity
files are synced and published atomically, never exposed half-written. Scientific
stages drain their contained descendants before releasing their lease. An
initialized, empty Job Object is authoritative: stale recorded PIDs cannot turn
unrelated status readers into purported surviving workers. One-turn Pi sessions
are explicitly reaped on terminal messages rather than relying on Drop cleanup.
This is an operational CLI adapter, not a domain dependency or another worker.
Other platforms still use recorded-child recovery without the Windows
spawn-time guarantee. The Windows regression interrupts dataset publication,
qualification, training completion, both development reports, model registration,
iteration result/completion and root completion, and verifies completed-work reuse.

The CLI now transfers each iteration's verified training output into ordinary
model custody, including development-rejected candidates and trained outputs
whose later evaluation could not persist a result. Stop still unwinds without
starting another custody operation. The model's producing
run pins the scientific training-completion event, and its dataset link verifies
the exact trainer version and ordered native contents without changing source
row identities. Benchmark result projection follows the root preparation,
iteration and training receipts into the derived scientific project; it does
not invent a runtime binding or search arbitrary sibling databases. Normal
development-journal replay supplies scores and original verdicts, including
completed suites before iteration-summary persistence. Model registration and
report viewing neither promote a candidate nor authorize final holdout.

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
root. `resume` advances one reserved stage; `drive` persists routine-execution
authorization and repeats that same stage handler under one process lease. A
core-owned decreasing lifecycle rank bounds continuation; persisted cancellation,
explicit sealed approval and existing child recovery still own their boundaries.
Neither command selects new inputs or expands budgets. It derives reports and row-free provenance from persisted
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

Its separate `versions` module owns native-schema-neutral dataset branches,
stable source-row references, immutable ordered membership, and row-level
add/remove/replace changes. This does not reinterpret native retrieval rows as
classification labels and does not grant qualification or training authority.
The managed workspace adapter persists these versions and verifies the source
imports. Existing snapshot splitting and curation contracts remain unchanged.
See `features/datasets/dataset-versions-spec.md`.

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
local BERT bundle described in
[`features/training/transformer-training.md`](features/training/transformer-training.md).
Candle tensors, tokenizer encodings,
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
artifacts without a protocol.

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

The original HTTP graphical surface covers Slice 1. The separately authorized
Encoder Gym Studio lives in `ui/`: a hardened Electron shell, typed IPC
bridge, read-only experiment-journal adapter, and vanilla TypeScript renderer.
It organizes persistent local project folders and presents baseline/candidate
comparisons, run history, benchmark context, and project settings.

The app's organizational folder metadata is separate from immutable domain
snapshots. Evidence reads use read-only SQLite connections. Explicit desktop
commands invoke the same owned CLI contracts as scripts; they do not mutate
scientific records through renderer state. Each meaningful project command is
also enclosed by a project-database activity action with a UUID, immutable
started/progress/terminal events, and safe artifact references. Scientific journal
projections remain row-free and never expose sealed scores. The separately
authorized dataset viewer may request bounded native training rows and diffs
through a fixed project-scoped CLI contract; source purpose, content identity,
and membership are verified before returning rows. These values never enter
activity logs, scientific projections, or sealed/adaptive channels. Core training,
evaluation, gate decisions, and optimization policy remain owned by their Rust
contracts; the adapter projects their recorded outcomes rather than recomputing
them. Report binding checks are not a replacement for native Doctor verification.

The desktop exposes only the finite managed-workspace execution intents, not
all platform slice formats. The current experiment CLI is composed with the
Nomos adapter; generic folder organization does not add another execution
backend. Training, evaluation, approval, and orchestration continue through
the same explicit CLI contracts. See
`../ui/README.md` for supported evidence, integration boundaries, and tests.

The shared project benchmark catalog references an `encoder-experiment-core`
definition that excludes model/training/run identity and pins suite membership,
roles, the existing metric contract, and adapter-normalized evaluator context.
`project-workspace-core` depends on this domain contract for its numbered
immutable versions; it does not implement metric or acceptance policy.
`project-workspace-local` owns catalog persistence and source-binding checks.
CLI composition verifies original scientific protocols and asks the native
adapter to normalize evaluation settings without opening native test files.
Catalog versions do not replace benchmark generations, qualification, exposure
or run authorization. See `features/evaluation/project-benchmark-spec.md`.

Result projection also follows project-owned optimization receipts into derived
scientific projects created by dataset materialization. The CLI loads only the
exact linked child; the workspace core verifies its original setup, runtime,
baseline, benchmark, protocol and journal binding before using the common
development-only projector. It does not create a replacement scientific binding,
search arbitrary co-located projects, or rewrite the baseline context.

`optimization-run history` composes those same verified development projections
with ordered iteration inputs, training custody and completion receipts. Its
metadata links each iteration to its original model, input/full/trainer
dataset versions, saved model and original-baseline gate values. Development
gate verdicts and best-so-far selection remain distinct from final approval.
The desktop validates this closed response and joins child experiments by exact
identity. Stop/Resume authority reads do not depend on report availability.
Artifact links refresh the project inventory before ordinary viewer navigation,
fencing late responses against project/navigation changes without restarting work.
New launch presets pin a core-owned first-batch generation canary. The runner
uses ordinary reservations and saved outcomes before concurrent dispatch;
dataset publication independently requires a passed sample under that policy.
The task adapter supplies only a closed training-question preview, never native
registry/state payloads. Historical launches without the policy are unchanged.
The separate `optimization-run cases --iteration` read reuses this custody
validator, then loads the exact iteration's completed scientific journal and
original-baseline development reports. Nomos owns bounded saved-diagnostic
projection and case pairing; no provider/evaluator is invoked. Missing samples
are explicit, and unmatched cases never establish fixes or regressions. Desktop
case loading is user-triggered, cached per immutable iteration identity and fenced
against project refreshes; it is separate from execution/status polling.
Single-run custody reads replay only the requested root and its Agent history,
using the same checks as full listings. Checkpoint registration verifies the exact
source and managed copy (including retries) without auditing unrelated historical
model/data contents; its returned inventory is not marked fully verified. Explicit
workspace verification retains the complete audit. Post-evaluation candidate
registration, result recording and completion replay emit iteration-scoped System
progress so bookkeeping remains visible separately from native evaluation.
The run-history report similarly projects only that root's iteration scientific
children through the shared benchmark validator; the benchmark-wide viewer still
projects all runs. Identical qualified/trainer versions are verified once within
each custody read, while distinct Quick-test subsets retain independent checks.
Native progress carries an explicit iteration ordinal through typed telemetry
and durable activity references; the renderer never assigns iteration membership
from timestamps. Unscoped setup activity remains separately selectable.

Desktop run observations are monotonic across collection reads, live polls,
Stop replies and drive completion. The fixed root journal retains cancellation
precedence; within that root head the independent Agent sequence orders control
intent. A stale local controller cannot replace a newer paused view. Durable
paused/terminal states stop presentation spinners even while IPC is settling.
A reopened stored-running attempt is explicitly unverified, not proof of either
a live worker or a pause. Stop remains available without starting another worker;
explicit Resume still uses the existing CLI lease/reconciliation and head fence.

The project-owned optimization setup separately pins the active baseline
revision/model, a dataset-core version reference and a shared benchmark version.
The local adapter stores compare-and-append setup history; the CLI previews and
saves only verified project references. No rows, secrets, scientific workflow
journal or execution permission enter this record. A separate immutable launch
authority pins that setup and the current non-secret provider-catalog identity,
copies separate generation/advisor ceilings, and supplies core-owned finite
defaults plus one selected-candidate final evaluation. The local adapter stores
append-only launch history and the CLI audits the explicit mutation; neither
surface stores credentials or executes work. Automatic execution must consume
that exact authority and existing admission contracts; the post-review
compatibility workflow must not ignore the user's selection. See
`features/optimization/optimization-launch-spec.md`.
