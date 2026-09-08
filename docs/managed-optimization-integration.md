# Managed optimization integration

Status: implementation contract for the desktop launch-readiness goal.

## Purpose

This document defines how a managed Encoder Gym project becomes the operator
surface for the existing finite encoder-optimization workflow. It does not add
a parallel workflow. The existing `synth encoder optimize` contracts remain the
scientific execution authority; the managed workspace supplies project scope,
custody, durable bindings, and a typed desktop application boundary.

Nomos is the first compiled task adapter. The project-facing concepts remain
task-neutral so another statically composed encoder adapter can implement the
same boundary without adopting Nomos row shapes or metrics.

## Current gap

Managed workspaces currently prove only custody:

- `encoder-gym.json` owns immutable project identity and the copied baseline
  inventory;
- `project.sqlite` owns imported-file metadata and its binding to that manifest;
- `models/`, `datasets/`, `runs/`, and `evaluations/` provide contained artifact
  locations but do not manufacture any slice artifact;
- the desktop reads the baseline and imports, but cannot create snapshots,
  suites, repair authority, workflow definitions, or runs.

The current Nomos managed workspace therefore truthfully has one imported
baseline and two imported training sources, but no candidates, approved repair
snapshot, executable benchmark authority, optimization run, or managed
evaluation evidence.

The compiled Nomos adapter has an additional dependency that custody cannot
infer. `NomosBackend` opens a clean, no-remote, isolated Git checkout containing
the experiment marker, native manifest, pinned Python modules, baseline and
support-model trees, native datasets, and suite configuration. The managed
Nomos workspace is not that runtime. `fitz-tool` is a source repository and must
never be selected as a substitute. A real launch remains unavailable until an
explicit, verified runtime and scientific-store binding exists.

## Object and ownership map

| User concept | Authoritative owner | Managed-project responsibility |
| --- | --- | --- |
| Project identity and folder | `project-workspace-core/local` | Own the durable local scope and contained paths. |
| Imported checkpoint custody | `project-workspace-core/local` | Verify immutable bytes and register a model artifact reference. |
| Imported dataset custody | `project-workspace-core/local` | Preserve native bytes and provenance; never imply admission. |
| Accepted source rows | Slice 1 / import contracts | Link only through the existing source-row identity. |
| Reviewed training membership | Dataset qualification and `dataset-core` | Display and select immutable approved snapshots. |
| Training checkpoint | `training-core`, or `encoder-experiment-core` for production adapters | Catalog a reference; do not copy trainer policy. |
| Evaluation suite/run | `evaluation-core`, workflow governance, or production experiment contracts | Project compatible row-free evidence. |
| Diagnosis and repair proposal | `analysis/optimization` or `encoder-repair-core` | Present review boundaries and next actions. |
| Finite production optimization | `encoder-campaign-core` plus the existing CLI composition | Bind, preview, launch, supervise, and recover it. |
| Final promotion decision | Existing workflow/experiment acceptance contracts | Advance the active baseline only from a verified promotable decision. |

`project.sqlite` remains the project registry. It may store project-level
references, active-baseline history, readiness facts, and the binding to slice
stores. It must not duplicate snapshots, predictions, metrics, workflow
journals, approvals, or promotion policy owned by scientific stores.

## Managed project records

### Model catalog

A project model artifact is an immutable project-level reference with:

- a project-unique artifact ID;
- display name, format, byte count, content fingerprint, and contained path;
- origin (`imported`, `trained`, or `transformed`);
- optional parent model, producing run, training snapshot, trainer identity,
  effective configuration fingerprint, tokenizer/processor fingerprint, and
  source revision;
- creation time and an integrity state derived from current verification.

The imported baseline is registered when a new workspace is created. Existing
workspaces receive the same record through a versioned, explicit, idempotent
upgrade. Unknown historical training provenance remains absent.

Evaluations are references to separate evidence and never fields on the model.

### Baseline revisions

Baseline and candidate are relationships, not mutable artifact flags. The
project stores an append-only baseline revision chain and one active revision
pointer:

- initialization points to the imported model;
- promotion points to the exact accepted immutable candidate and existing
  scientific decision;
- restoration creates another revision pointing to an older artifact;
- every revision records actor, reason, prior revision, and decision reference;
- compare-and-append plus the active pointer update occur in one transaction.

There is exactly one active baseline after initialization. The former baseline
remains historical and is not automatically made a candidate. An active
baseline cannot be demoted without an atomic replacement.

### Scientific binding

The project has at most one active versioned scientific binding. It records:

- the managed project ID and baseline revision it was validated against;
- the compiled adapter key and protocol/configuration fingerprint;
- the adapter-runtime location and verified project snapshot identity;
- the SQLite scientific-store location and schema identity;
- contained artifact roots used by the existing contracts;
- creation/verification times and an immutable specification fingerprint.

Paths are stored as contained project-relative paths wherever the existing
contract permits. A compiled adapter runtime that cannot yet be packaged under
the managed root is a declared external execution dependency, never provenance
inferred from the original source path. It must be an explicitly selected,
validated isolated workspace. Moving the managed project makes that binding
unavailable until it is revalidated; custody remains portable and intact.

For Nomos, the desktop boundary passes the validated managed root, scientific
store, isolated adapter runtime, and fixed adapter key separately. The existing
CLI containment check must be extended to accept the managed scientific store
as an explicitly authorized root; it must not weaken the adapter's clean,
no-remote isolated-runtime checks. Arbitrary shell commands and arbitrary
adapter code remain impossible.

Historical Nomos databases and journals are not automatically bound. Importing
legacy evidence requires a future explicit verifier and is outside this goal.

## Readiness model

Readiness is a derived report, not a stored boolean. Each check has a stable key,
category, state (`ready`, `action_required`, `blocked`, `stale`, or
`unavailable`), plain-language evidence, and at most one real next action.

The report evaluates:

1. workspace and baseline integrity;
2. active-baseline uniqueness and adapter compatibility;
3. scientific binding and store/runtime health;
4. imported-source qualification and approved training snapshot;
5. development and sealed suite/bundle authority and global contamination;
6. current repair proposal, delta review, and logical training snapshot;
7. finite candidate, iteration, external-call, and sealed-exposure budgets;
8. provider choices and secret availability without secret values;
9. stale/superseded/revealed authority and successor-benchmark requirements;
10. recoverable or active workflow state.

The overall state is runnable only when the exact immutable preview can be
resolved by the existing optimize application. A failure to resolve is surfaced
as its owning missing prerequisite; the renderer does not reinterpret it.

Nomos imported training files do not satisfy snapshot or repair-delta approval.
The proven negative run and its revealed historical evidence do not satisfy a
new run. A new run needs a fresh reviewed hypothesis, approved native delta,
logical training snapshot, and active unused successor benchmark generation.

## Credential and authorization boundary

Non-secret settings store provider kind, endpoint, model name, and bounded
defaults. Secrets have separate authorities:

- generation: existing `SYNTH_OPENAI_API_KEY` fallback;
- advisor/agentic work: existing `SYNTH_ADVISOR_API_KEY` fallback;
- evaluator: only when the configured evaluator requires one.

The Electron main process owns secret submission and availability checks. It
uses the operating-system credential service when available and environment
variables as documented fallback. IPC returns only `missing`, `available`, or
`unavailable`; secret values never cross into the renderer and are redacted
from errors, logs, reports, tests, and screenshots.

Implemented desktop storage uses Electron `safeStorage` after application
startup (DPAPI on Windows, Keychain on macOS, and the selected supported secret
backend on Linux). Only encrypted base64 blobs keyed by managed-project ID and
provider role are written to the app profile; writes are atomic and mode 0600
where supported. If OS encryption is unavailable, saving fails and the explicit
environment fallback remains usable. Password fields are write-only, never
prefilled, and disappear with the dialog. Main-process readiness overlays only
availability/source onto the CLI's non-required provider checks; it never sends
the secret into an ordinary readiness subprocess.

Provider health checks are offline configuration checks by default. A live
probe is a separate explicit operation that states its network and cost effect.
Starting a run requires a persisted finite authorization envelope; merely
storing a key never authorizes a call.

## Desktop execution boundary

The renderer sends typed intents such as preview, start, resume, authorize
sealed, cancel, and inspect. It never sends command arrays, paths outside a
selected project/binding, SQL, environment maps, or shell text.

Implemented boundary: the main process owns native manifest selection and keeps
the chosen path behind a project-scoped, process-local token. It exposes fixed
readiness, upgrade, preview/start, status/inspection, review, resume,
authorization, cancellation, Doctor, provenance, and report intents. Every
intent reopens the registered workspace and the Rust composition root
revalidates its active baseline, binding, runtime project, and scientific store.
Mutating intents are exclusive per project; a second concurrent start/resume or
authorization is rejected. CLI launch uses `execFile` without a shell and JSON
stdout. Durable child-process correlation and credential injection remain the
next main-process lifecycle component.

The Electron main-process adapter:

1. reopens and verifies the selected managed project;
2. loads the active scientific binding and recomputes readiness;
3. builds fixed arguments for the supported CLI/application command;
4. launches one child process without a shell and with an allowlisted
   environment;
5. parses exactly one JSON stdout value while treating stderr as redacted
   diagnostics/progress;
6. persists the child identity and correlates events to project, run, stage,
   and request IDs;
7. rejects duplicate starts and stale renderer requests;
8. reconciles the durable scientific state after interruption or restart.

The application owns state, not process output. After any process exit, the
main process reloads status from the scientific store. Cancellation first uses
the workflow's persisted cancel transition. A synchronous native stage may
finish before cancellation is observed; the UI says so and never invents a
preemptive percentage.

## Launch journey

`Start optimization` opens a project-scoped preparation route rather than
starting work immediately.

1. **Readiness** lists the exact missing prerequisites and resumes any existing
   nonterminal run before offering a new one.
2. **Prepare** selects or creates the hypothesis, approved delta/training
   snapshot, benchmark generation, candidate space, providers, and finite
   budgets through their owning contracts.
3. **Preview** resolves the same immutable definition used by start and shows
   baseline, snapshot, suites, stages, provider boundaries, worst-case calls,
   sealed boundary, and artifacts that will be written.
4. **Authorize and start** records the bounded external authorization and
   idempotently reserves one optimization run.
5. **Run detail** shows persisted stages, journals, budgets, redacted logs, and
   the exact safe next command. Resume advances at most one durable stage.
6. **Sealed authorization** appears only for the development-selected eligible
   candidate and identifies that candidate and the one-use consequence.
7. **Decision** shows retain/reject/promote, supporting comparable evidence,
   and limitations. A promotable decision may atomically create a new managed
   baseline revision.

No default can silently select sealed evidence, expand a budget, replace a
baseline, or reuse stale authority.

## Page responsibilities

- **Models**: active baseline revision, immutable artifacts, candidates grouped
  by their exact comparison baseline/setup, and promotion evidence. It does not
  display dataset inventory as substitute content.
- **Data**: imported sources, qualification, immutable snapshots, purposes, and
  model/run usage.
- **Runs**: optimization and owned child runs, durable stage, budget use,
  failure/recovery, and safe next action.
- **Evaluation**: suites/protocols and exact model/cohort/protocol evidence,
  compatibility, exposure, and deterministic decision.
- **Project settings**: adapter and scientific binding, provider settings,
  secret availability, workspace integrity, and administrative repair actions.

Model, run, and evaluation details are deeper routes. They expose provenance
without requiring the user to navigate raw hashes first. Empty pages state the
true missing object and link to its real creation/readiness action.

## Migration and recovery

Managed-registry migrations are versioned and idempotent. Opening remains
read-only; an explicit upgrade operation applies new migrations and initializes
the imported model and first baseline revision from the already verified
manifest in one transaction. It never edits the manifest or scientific stores.

A scientific binding is append-only by specification fingerprint with one
active pointer. Rebinding records why the prior binding was superseded. Missing
external runtime paths make execution unavailable without invalidating custody.
Scientific-store migrations remain owned by their existing adapters.

Run recovery follows persisted journals and reserved child IDs. The project
registry may cache display summaries, but those are discarded and rebuilt when
they disagree with the owner store. It never marks a run successful based on a
process exit code alone.

## Verification strategy

Deterministic fixtures must cover:

- existing-workspace upgrade, idempotency, and rollback;
- model immutability, provenance, and exactly-one-baseline invariants;
- compare-and-append promotion, stale revision rejection, and restoration;
- binding containment, adapter/store mismatch, missing runtime, and rebind;
- readiness from persisted facts, including stale and revealed authority;
- typed IPC validation, process identity, duplicate start, restart, cancellation,
  and redaction;
- development rejection without sealed exposure, sealed rejection, promotion,
  and terminal recovery using the fake adapter;
- at least one non-Nomos project proving task-neutral page/readiness shapes;
- real Electron interaction for setup, preview, start, resume, sealed pause, and
  decision, all against offline fixtures.

The real managed Nomos workspace is read-only during development verification.
An explicit final preflight may inspect its binding and scientific authority but
must make no provider call, training run, sealed evaluation, spend, or artifact
mutation. A real run starts only after the user sees and authorizes its final
immutable preview.

## Delivery order

1. project model catalog and baseline-revision domain/persistence;
2. explicit scientific binding and derived readiness report;
3. CLI commands for upgrade, binding, readiness, and fixed managed launch;
4. safe credential/settings boundary;
5. typed Electron process lifecycle and recovery;
6. project preparation, run, evaluation, and promotion journeys;
7. deterministic vertical acceptance and read-only Nomos preflight.

Each stage keeps the existing slices independently useful, runs the repository
gates, and lands as a coherent working commit.
