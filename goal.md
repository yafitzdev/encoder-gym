# Goal — Make managed projects ready to launch real encoder optimization runs

Implement the complete, honest desktop journey required to prepare, start,
supervise, recover, inspect, and finish a real bounded encoder optimization run.
Use Nomos as the first real task adapter and integration target, while keeping
the project, artifact, evaluation, and workflow concepts generic enough for
other encoder projects.

This is a vertical product-integration goal, not another visual-polish pass.
The platform already contains independently useful generation, dataset,
training, evaluation, analysis, optimization, and controlled-workflow
contracts. Preserve their ownership and connect them to managed projects and
the desktop application without copying their business logic into the GUI.

The user must not need to copy opaque IDs, edit TOML manifests, choose SQLite
URLs, or assemble CLI commands to run the normal workflow. The application must
make scientific readiness, costs, authorization, progress, evidence, and the
next safe action understandable before execution begins.

## Current reality

Preserve the managed-workspace, local dataset-import, project-navigation, and
real Nomos onboarding work already delivered.

Do not confuse custody with readiness:

- The managed Nomos workspace has a verified imported baseline and two imported
  source files containing approximately 6,800 rows.
- The platform now has an immutable model catalog, audited baseline revisions,
  explicit scientific-store/runtime bindings, history import, derived launch
  readiness, managed preparation, fixed run intents, durable progress, and
  sealed-accepted checkpoint promotion. Encrypted desktop credentials are
  resolved only for native execution and injected under fixed project-role
  child-environment names. These capabilities remain generic project
  infrastructure and do not fabricate missing Nomos facts.
- The operator explicitly approved the real Nomos setup on 2026-09-09. The
  managed registry now has an immutable model catalog; the selected Python
  3.12 environment provides every compiled Nomos capability, including
  `onnxruntime_genai`; and the clean isolated runtime at revision
  `4450ab3f1de8a1fc64bcbe5d77c67d0fb0f99af9` is bound to a verified,
  content-addressed copy of the selected historical SQLite database.
- Historical experiment rows remain scientific run/evaluation evidence. They
  do not become managed candidate artifacts merely because their database was
  imported. The real Models page therefore truthfully shows one baseline and
  no managed candidates.
- The operator explicitly authorized real Nomos preparation on 2026-09-09. The
  owner replay reused training snapshot
  `2602ef88-db56-4591-8a4c-2581c5613726`, made zero external calls, and
  published the 792-byte definition with fingerprint
  `sha256:f9a4a9e61d2baaec06ae9f2ea5737592ff08accfddc2ea3b0d8d62ed8b73b0a0`
  below the managed `runs/definitions/` directory. It covers one reviewed
  candidate over 192 approved delta rows, with 7,200 seconds maximum training,
  two development evaluations, one separately authorized sealed use, and zero
  external calls. Preparation took about 28 minutes on the real workspace. The
  exact run is now ready and runnable, but no run identity has been reserved.
- The desktop can recover, launch, finish, and promote that real bounded
  workflow from the prepared immutable definition.
  The final reservation review now keeps the human objective visible and names
  the candidate recipe, development suites, sealed suite, execution ceilings,
  external-call allowance, and exact persistence boundary. Sealed use requires
  a separate single-use confirmation after development eligibility.
  After promotion, explicit Nomos rebind now resolves only the exact
  sealed-accepted source checkpoint, keeps runtime code immutable, creates a
  successor scientific project snapshot for the new baseline, and extends the
  existing scientific store instead of silently starting unrelated history.
- The first post-preparation real Electron reopen exposed an identity-boundary
  defect hidden by fixtures that reused one UUID: the prepared definition names
  the scientific project, not the managed folder. The main-process adapter now
  verifies it against the bound runtime snapshot identity, while retaining the
  managed ID for project scoping. A distinct-ID regression and the real
  read-only verifier both pass. The real reservation action is visible in the
  initial viewport after the essential limits and evidence boundary, before
  optional recipe and immutable-ID disclosures.
- Once reserved, the desktop now puts the active run and its one safe next
  action above the frozen launch definition. Run supervision distinguishes
  completed model/evaluation records from reserved and unreserved capacity,
  exposes the latest durable transition across the parent, campaign, and
  experiment journals, and labels the wall-clock journal span separately from
  native compute time. Failed or uncertain work is never presented as unused
  budget merely because it lacks a completion record.
- Terminal runs now expose the existing aggregate owner report directly in the
  desktop: candidate checkpoint facts, per-development-suite baseline and
  candidate metrics, assessment identities, training-snapshot and repair
  provenance, journal counts, sealed-use status, and known evidence limits.
  Sealed rows and sealed metric values remain outside the renderer.
- The Runs collection now counts and presents the persisted optimization parent
  independently from its child experiment journal, keeps the current safe stage
  reachable after navigation, and passively recovers the parent status from
  readiness after restart. Linked parent/child records count as one run rather
  than disappearing before child creation or appearing twice afterward.

Treat these as product facts to resolve or explain, not presentation details to
hide behind optimistic status text.

## Product model to establish

### Project

A project is the durable scope for developing one encoder family. It owns the
human-readable identity, workspace custody, task adapter selection, provider
configuration, active baseline reference, candidate history, dataset sources,
snapshot history, evaluation policy, and optimization runs.

Define an explicit project-to-scientific-store binding. Keep the managed
`project.sqlite` custody database distinct from the scientific stores owned by
the existing slices. Never infer a legacy database merely because it exists
near a source repository.

### Model artifact

A model artifact is an immutable checkpoint plus the provenance needed to
understand and reproduce it. At minimum, record:

- checkpoint identity, format, location, integrity, and compatibility status;
- origin: imported, trained, or transformed;
- parent model and producing run when applicable;
- exact dataset snapshot used for training when applicable;
- trainer/backend identity, effective configuration, tokenizer or processor
  identity, code revision, and relevant environment facts;
- evaluation references without embedding evaluation results into the model.

An imported baseline may legitimately have no proven historical training
snapshot. Represent that as unknown provenance, not a fabricated association.

### Baseline, candidate, and promotion

Baseline and candidate are project relationships to immutable model artifacts,
not mutable flags stored inside the artifact.

- A project has exactly one active baseline revision once it is initialized.
- A candidate is interpreted against the exact baseline and run that produced
  or assessed it.
- Promotion atomically advances the project's active-baseline pointer and
  records an immutable, auditable revision with the accepted evidence.
- The previous baseline remains historical; do not automatically relabel it as
  a candidate. Restoring it creates a new audited baseline revision.
- Demotion of the active baseline is not an ordinary standalone action because
  it would leave the project without a reference. If an administrative override
  is needed, make it explicit, exceptional, and safe.

### Dataset, snapshot, suite, evaluation, and assessment

Keep these concepts distinct:

- A dataset import is a Gym-owned copy of source data with provenance.
- A dataset snapshot is an immutable, reviewed training or evaluation cohort.
- An evaluation suite or protocol defines how a model is measured.
- An evaluation run applies one model to one immutable cohort under one exact
  protocol and persists predictions, observations, metrics, and comparisons.
- JSON or JSONL may be an import/export representation; it is not the definition
  of an evaluation.
- An assessment applies acceptance policy to comparable evidence and produces a
  decision. A completed evaluation is not automatically a passing candidate.

Nomos requires a real task adapter with composite native evidence: retrieval
metrics such as MRR, Recall, and margin behavior, plus deterministic agent/tool
outcome checks where the accepted Nomos contract requires them. Do not reduce
Nomos to generic classification or impose Nomos metrics on every future project.

## 1. Write the integration design before broad implementation

Read the required platform, architecture, development, slice, managed-workspace,
controlled-workflow, and Nomos documents. Inspect the current domain objects,
stores, CLI commands, Electron boundary, fixtures, and the real managed Nomos
workspace in read-only mode.

Create and maintain `docs/managed-optimization-integration.md` as the concise
implementation contract. It must document:

- the cross-slice object and provenance map;
- which existing slice owns every fact and transition;
- project-to-store binding and migration behavior;
- model catalog, active baseline, candidate, and promotion semantics;
- readiness requirements and how each is derived;
- desktop-to-workflow IPC and process lifecycle;
- credential, authorization, budget, and sealed-evidence handling;
- failure, cancellation, restart, resume, and idempotency behavior;
- the final page responsibilities and end-to-end user journey.

Resolve contradictions in existing docs explicitly. Do not use the design
document to invent a second workflow beside `synth encoder optimize`.

## 2. Make a managed project scientifically ready

Add a persisted, derived readiness model for a managed project. It must explain
what is ready, what is missing, what is stale, and what the user can do about it.
At minimum, establish or verify:

- active baseline identity, integrity, and backend compatibility;
- task adapter and source/code revision;
- imported data qualification versus mere custody;
- reviewed and approved training snapshot;
- development and sealed evaluation suites or benchmark generation;
- contamination and membership constraints;
- finite candidates, iterations, external calls, and sealed-exposure budgets;
- required human reviews, approvals, and staleness rules;
- configured providers and the availability—not the value—of required secrets;
- recoverable scientific-store and workflow state.

Every missing requirement shown in the application must have a real action or a
plain explanation. Do not provide bypass buttons that fabricate readiness.
Readiness must be recomputed from persisted facts and the currently selected
baseline revision, not cached presentation state.

Use the established successor-authority rules. A revealed or exhausted sealed
generation, stale proposal, or superseded baseline must not be reusable merely
because its records still exist.

## 3. Add safe provider and credential setup

Support separate credentials for separate authorities:

- a data-generation provider key;
- an advisor or agentic-work provider key;
- an evaluator key only if the selected evaluator requires one.

Persist non-secret provider choices, model names, endpoints, and bounded default
settings. Never persist raw keys in project manifests, SQLite, logs, renderer
state, reports, fixtures, screenshots, or Git.

Prefer the operating system's credential facility for desktop-managed secrets,
with documented environment-variable fallback compatible with the existing
`SYNTH_OPENAI_API_KEY`, `SYNTH_ADVISOR_API_KEY`, and evaluator selection.
The renderer may submit or replace a secret and show whether it is available,
but it must never read the secret back.

Ordinary readiness checks must not make paid or external calls. If a live
provider probe is useful, make it an explicit user action and clearly describe
its network and possible cost implications.

## 4. Launch the real bounded optimization workflow

Expose the existing controlled workflow through a typed, fixed application
boundary. The renderer must never supply arbitrary commands, executable paths,
SQL, or shell fragments. The main process or a dedicated adapter owns process
creation, argument construction, validation, redaction, lifecycle, and recovery.

From a managed project, the user must be able to:

1. Select **Start optimization** and see the exact baseline, hypothesis,
   snapshot, development suites, sealed suite, candidate space, finite budgets,
   providers, and missing requirements.
2. Complete real missing prerequisites through their owning slice.
3. Preview the immutable run definition and its expected external-call limits.
4. Explicitly authorize external work within those limits and start exactly one
   run without entering IDs or using a terminal.
5. Observe stage progress, persisted transitions, completed recorded work,
   reserved and unreserved capacity, journal timing, safe logs, and redacted
   diagnostics. Do not claim precise consumption for work that failed without
   a durable completion record.
6. Cancel where the underlying stage supports cancellation, resume recoverable
   work, and recover the truthful state after closing or restarting the app.
7. Authorize sealed evaluation only when a selected candidate is eligible.
8. Inspect the terminal outcome and all artifact, snapshot, evaluation,
   analysis, and decision provenance.
9. Promote a valid accepted result and immediately see the new active baseline
   revision throughout the project.

Represent synchronous or non-preemptible backend stages honestly. Never show a
fake cancel, fake percentage, or fake success. Prevent duplicate starts, stale
renderer updates, cross-project event leakage, and re-use of expired authority.

## 5. Present the domain without duplicate pages

Choose and implement the final navigation based on user questions, not backend
crate names. Merge or remove destinations whose responsibilities duplicate
another page. At minimum:

- **Models** answers: What is the active baseline? What model artifacts exist?
  Which candidates are comparable to it? What evidence supports promotion?
- **Data** answers: What sources are in custody? Which immutable snapshots exist?
  Which are reviewed, approved, or used by which model/run?
- **Runs** answers: What is running or finished? What stage is it in? What did it
  consume and produce? What action is safe now?
- **Evaluation** answers: What suites exist? Which exact model/snapshot/protocol
  combinations were measured? Are comparisons valid and what did policy decide?
- **Project settings** owns task adapter, store binding, providers, secret
  availability, budget defaults, workspace location, and integrity diagnostics.

Do not show dataset inventory as filler on the Models page. For Nomos today, the
truthful model state is **one baseline, no candidates**. The primary action should
be **Start optimization** or a precise readiness action explaining why it cannot
start yet.

Model and evaluation detail views must make immutable identities and provenance
inspectable without leading with hashes and database terminology.

## 6. Preserve compatibility, evidence, and safety

Use versioned, crash-safe migrations. Existing managed projects must reopen
without recreation. Legacy experiments remain separate unless a later explicit
evidence-import contract validates and imports them.

Preserve:

- immutable source snapshots and derived artifacts;
- append-only decisions and baseline-revision history;
- deterministic acceptance and comparison compatibility;
- external-call, candidate, iteration, and sealed-exposure budgets;
- sealed-evidence isolation from generation, training, adaptive analysis, and
  advisor components;
- slice ownership and replaceable ports.

Do not add cloud deployment, authentication, multi-user collaboration,
distributed workers, arbitrary-code execution, autonomous agents, bandits,
reinforcement learning, semantic deduplication, or Hugging Face onboarding
unless a later specification explicitly authorizes it.

## 7. Verify with fixtures first and Nomos safely

Build deterministic offline fixtures that exercise the whole vertical path:

- managed-project migration and scientific-store binding;
- model import, integrity, catalog, and single-baseline invariants;
- dataset import through reviewed immutable snapshot;
- suite/protocol creation and evaluation persistence;
- optimization preview and start;
- development rejection without sealed exposure;
- eligible candidate, sealed rejection, and accepted promotion paths;
- atomic active-baseline revision and historical evidence;
- provider failure, stage failure, cancellation, restart, resume, and duplicate
  event/start idempotency;
- cross-project and stale-baseline isolation;
- credential redaction through logs, errors, IPC, reports, and screenshots;
- at least two generic project types without Nomos assumptions;
- actual Electron interaction for the critical journey.

Use the real managed Nomos project only for read-only inspection and an explicit
preflight that performs no training, external calls, sealed evaluation, spending,
or scientific mutation. Do not start a paid or destructive real run without the
user's explicit authorization after showing the final preview. Completion should
leave Nomos in the state where the user can confidently press **Start**.

## 8. Implement and commit in coherent stages

Follow the repository's required dependency direction:

1. domain objects and pure invariants;
2. persistence, migrations, and adapters;
3. CLI compatibility and orchestration integration;
4. typed IPC/main-process lifecycle;
5. renderer journeys and presentation.

For every coherent component, run the required `cargo fmt-check`,
`cargo check-all`, `cargo lint`, and `cargo test-all` checks, plus the UI
checks, build, focused tests, and real-renderer verification appropriate to the
change. Most tests must be deterministic and require no credentials, downloads,
GPU, external API, or paid service.

Commit each coherent working stage with a clear message. Preserve unrelated
changes and record important decisions and verification evidence in the
integration document.

## Done means

- Opening any managed project shows a truthful readiness result with actionable
  prerequisites and no conflation of imported data with runnable data.
- The user can configure separate generation and advisor credentials without
  exposing or committing them.
- The user can prepare or select a reviewed snapshot, evaluation suites,
  hypothesis, providers, and finite budgets through the desktop application.
- The user can preview and start a real Nomos optimization run without copying
  IDs, editing TOML, choosing databases, or using the CLI.
- The app truthfully survives restart, prevents duplicate execution, enforces
  authorization and budgets, and exposes recoverable failure information.
- Model artifacts remain immutable and link to their producing run and training
  snapshot; evaluations remain separate, comparable evidence.
- Exactly one active project baseline exists, and valid promotion advances it
  atomically with an auditable revision.
- The Models page no longer treats imported datasets as model content or implies
  that candidates exist when they do not.
- Deterministic end-to-end coverage proves rejection, sealed isolation,
  acceptance, promotion, recovery, redaction, and multi-project isolation.
- The real Nomos workspace is unchanged by verification, and its final preflight
  states exactly what will run, call externally, cost or consume, and persist.
- Documentation and committed stages match the delivered behavior, and no
  product or integration blocker remains between the user and a supported run.

Do not claim success because a Start button exists or a mock run animates. The
goal is complete only when that button launches the real bounded workflow using
persisted, reviewable scientific artifacts and the user can understand what will
happen before authorizing it.
