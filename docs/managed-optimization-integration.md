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

Project settings owns a two-step runtime connection. Native pickers mint opaque
main-process tokens for the isolated checkout and Python executable. A passive
CLI preview verifies the clean/no-remote runtime, exact active-baseline match,
Python 3.11/3.12 compatibility, and every dependency group needed for training,
retrieval evaluation, local agent evaluation, and diagnostics. Only a ready
preview token can reach the fixed binding intent. Binding repeats all checks,
then creates or verifies the contained scientific store and appends the binding;
preview never writes, contacts a provider, trains, or evaluates.

An incomplete but version-compatible interpreter exposes a separate explicit
repair boundary. The blocked preview token—not renderer-authored paths or
packages—authorizes `workspace prepare-nomos-python`. The CLI repeats runtime
and baseline verification, derives missing modules from the compiled capability
groups, maps them to a fixed package allowlist, and invokes the selected
interpreter's pip without a shell, input prompts, source distributions, or
version-check traffic. The UI confirms that this mutates the selected Python
environment and may contact its configured package index. Captured installer
output is never relayed into renderer diagnostics because it may contain an
authenticated index URL. A fresh offline preview must pass before binding.

Historical Nomos databases and journals are never automatically bound. Project
settings may explicitly select an existing Encoder Gym production database
through a separate opaque native-picker token. Preview opens it read-only,
requires the exact current migration set and complete SQLite integrity, finds
the current runtime project by its immutable source fingerprint, asks the
compiled adapter to reproduce that project, and returns only a row-free artifact
inventory. Binding repeats those checks and uses SQLite `VACUUM INTO` to create
a transactionally consistent standalone image including committed WAL state.
The image is hashed and atomically published below managed `runs/` under its
content identity; the binding records its SHA-256 and bytes. The source database
is neither attached nor modified. Individual historical artifacts remain
subject to their existing deep owner verification before use.

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

Managed preparation no longer requires the normal desktop user to author or
pick that TOML manifest. Readiness asks the scientific store for approved native
delta selections belonging to the exact bound execution-project snapshot, then
retains only the one whose proposal is unexpired and whose pinned benchmark
generation is still active, unused, fresh, and at the same journal head. The
screen shows that reviewed hypothesis, qualified delta-row count, candidate
count, expiry, and finite budgets before mutation. The fixed
`workspace prepare-optimization` intent calls the existing production-repair
snapshot builder, including native delta replay, and writes a strict
content-addressed manifest below `runs/definitions/`. Existing identical
snapshots and definitions are adopted idempotently; conflicting bytes are never
overwritten. Preparation performs no training, evaluation, provider call, or
sealed exposure. The Electron main process validates that the returned path is
inside the selected managed workspace and gives the renderer only an opaque
project-scoped token plus the resolved readiness view. Manual manifest selection
remains a compatibility/debugging boundary, not the primary journey.

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

The 2026-09-09 binding preflight copied the real managed workspace to a temporary
directory, upgraded only that copy, and matched its 134,211,908-byte baseline to
isolated revision `4450ab3f1de8a1fc64bcbe5d77c67d0fb0f99af9`. The selected
Python 3.12 environment passed training, retrieval, and diagnostic capability
checks but lacks `onnxruntime_genai`, so local agent evaluation—and therefore
binding—is truthfully blocked. The actual managed workspace and isolated runtime
were not changed. Project settings exposes the older workspace's required model-
history upgrade separately rather than hiding it inside runtime setup.

A second temporary-copy check supplied an explicit no-op marker for the one
missing Python capability, bound the runtime, and reopened readiness in a fresh
CLI process. All four scientific checks—binding, store, runtime, and persisted
project—remained ready. This check exposed and fixed two integration defects:
Windows extended paths are now stripped before SQLite URL construction, and
runtime verification now loads the immutable project identity from the bound
store before asking the adapter to reproduce its content. It no longer compares
against the adapter's intentionally ephemeral inspection ID.

The 2026-09-09 explicit-history preflight selected
`encoder-gym-repair.sqlite` while operating only on a temporary managed-workspace
copy. The current migration set, SQLite integrity, latest runtime project and
source revision reproduced exactly. Its row-free inventory contains three
project snapshots, two protocols, two experiment runs, two benchmark
generations, one diagnosis, two repair proposals, two approved native-delta
selections, one training snapshot, and one completed optimization run. The
preview correctly remained blocked only by the separately reported missing
`onnxruntime_genai` Python capability. The actual managed workspace, isolated
runtime, and selected history database were unchanged.

A separate full import lifecycle check used the same deliberate no-op capability
marker as the earlier binding test, again only against a temporary managed copy.
SQLite produced a 7,180,288-byte standalone history image with fingerprint
`sha256:12586ad5146d9a95901748fa914931c7cb8acef2421654d73579c592c6ec6fdd`.
The contained copy reopened in a fresh process and all four scientific binding,
store, runtime, and project checks were ready. A before/after SHA-256 check
proved the selected source database unchanged. The temporary workspace and
marker were removed after verification.

A later preparation preflight operated only on another temporary managed copy
with the same contained history image and temporary import-only capability
marker. It found one current project-scoped successor authority and resolved
the reviewed `conservative-genuine-finetune-v2` hypothesis: one candidate,
7,200 seconds maximum training, zero external calls, and at most one separately
authorized sealed use. It reused logical training snapshot
`2602ef88-db56-4591-8a4c-2581c5613726`, published an immutable 792-byte
definition under `runs/definitions/`, and found no existing optimization for
that exact manifest. No trainer, evaluator, provider, or sealed suite ran.
Native artifact/delta replay took 817 seconds on this machine. The preparation
UI therefore describes it as an indeterminate, one-time integrity operation;
it never displays a fabricated percentage. Redundant post-write graph reloads
and duplicate managed/native project verification were removed, leaving the
production-repair owner replay as the sole expensive boundary.

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
