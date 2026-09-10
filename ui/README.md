# Encoder Gym desktop

A local, multi-project Electron workspace for importing encoder baselines and
datasets and inspecting recorded development evidence. The app starts with an
empty project collection. Nomos is an opt-in recorded example, not the product
identity or a default connection.

## Launch

Use Node.js 22.19 or newer, npm, and the repository's Rust toolchain. From the repository root:

```powershell
cd ui
npm ci
npm start
```

`npm start` builds the Rust workspace CLI and opens the desktop app. Close an older running instance
before launching changed code; a second normal launch focuses that instance.

## Project folders and page responsibilities

Use **New project** (the sidebar plus) to choose a local checkpoint, preview its
format/size, name the project, and choose a parent location. Gym copies the
checkpoint into a new owned folder and creates its manifest, registry database,
and artifact directories. The source stays untouched. Hugging Face is deferred.
The confirmation shows the full new folder path and copy size. Fields scroll
separately from the action/feedback area, including in small windows. Task
description is optional. Failed operations keep the dialog open with recovery
guidance and expandable technical details; re-select a file if it changed after
preview. No source files are overwritten.

Use **Open project** for an existing Gym workspace containing `encoder-gym.json`.
Arbitrary repositories are not accepted. Earlier read-only journal folders
remain available under **Earlier experiment folders → Connect legacy journals**.
They are labelled Legacy and are not silently converted into managed projects.

- **Project folders:** persistent entries in the sidebar. A newly opened project
  starts on Models. Switching projects resumes each project's last page during
  the session; clicking the already-active folder returns to Models.
- **Models:** the baseline reference and all unique candidate identities,
  grouped by compatible baseline, benchmarks, metric contract, and evaluation
  policy. Search and filter, select a benchmark, sort by its primary score,
  or compare up to three candidates from one setup.
  **All models** returns from details to the list's previous scroll position
  and originating model control. Back/forward also preserve that context.
- **Candidate detail:** exact development checks, training configuration and
  immutable input references, model artifact, and historical attempts.
- **Datasets (managed projects):** import native JSONL with an explicit purpose,
  preview record counts and partitions, and inspect copied-file provenance.
  Imports are not admitted training snapshots and do not create class labels,
  splits, runs, or evaluation results. Training imports reject held-out rows.
  File counts, size, and intended use are visible in the list; expand **File
  details and provenance** for original/copy locations and content identities.
- **Runs:** immutable experiment records. A run owns its candidate attempts,
  activity, recorded budgets, provenance, and final decision. Completion does
  not mean the candidate was accepted.
- **Benchmarks:** what is measured and why evaluation setups are separate.
- **Activity:** immutable project commands grouped by action UUID, with event
  UUIDs, progress, outcomes, linked artifact identities, and JSONL export.
- **Project settings:** app name, durable organizational ID, folder association,
  evidence source, rename/reconnect, and removal from the collection.

Removing an entry never deletes project files. Reconnect a moved folder using
**Locate folder**; this requires the same managed identity at its new path.
Managed identity also survives reopening in a different app profile. Duplicate registration
selects the existing entry. Empty projects and registered baselines with no
runs have dedicated states; missing, corrupt, and unsupported evidence have
recovery guidance rather than borrowing another project's results.

The folder collection and selected project persist in `projects.json` under
Electron's user-data directory (normally `%APPDATA%/@encoder-gym/ui` on Windows).
Theme and sidebar width also persist. Search, filters, comparison selection,
and page history are isolated per project during the session; those view
settings are not persisted across app restarts. A corrupt collection is
reported without overwriting the original file.
Opening/locating failures stay on the page until dismissed or another folder
operation begins. Technical diagnostics are expandable. During file verification,
the current page's actions are unavailable, but switching projects is allowed;
a late verification result cannot overwrite a newer read of the same project.

## Local models, imported datasets, and legacy evidence

Managed onboarding calls the project-owned `synth workspace` commands. It
accepts self-contained BERT-family safetensors encoder bundles, preserving
supported sentence-transformer modules. It does not execute model code or
prove trainer compatibility. Unsupported formats, custom code, pickle weights,
and links/junctions are rejected. See [the workspace guide](../docs/managed-workspaces.md).

Project settings offers **Verify project files**: the backend rehashes model
and dataset artifacts and checks counts and native training provenance. Routine
opening checks metadata binding and file sizes. Imports are independently
copied and content-addressed. Same-content imports with matching purpose and
provenance are idempotent; conflicting purpose/provenance is rejected.

Project settings also configures separate generation and advisor authorities.
Their endpoint, model, environment fallback, and finite request/token/cost
limits are append-only project records. Submitted keys are encrypted by the
operating system in the Electron profile and are never returned to the
renderer. `SYNTH_OPENAI_API_KEY` and `SYNTH_ADVISOR_API_KEY` remain explicit
fallbacks when desktop credential encryption is unavailable or intentionally
not used. Availability checks are offline and make no provider request.

Scientific runtime setup also stays behind native picker tokens. The desktop
first previews the isolated checkout and Python executable through the fixed
`workspace preview-nomos-binding` intent. The preview checks the exact baseline,
clean/no-remote source state, supported Python version, and required execution
capabilities without writing or contacting a provider. **Connect runtime** is
enabled only for a ready preview; the binding command rechecks the same facts
before creating or verifying the contained scientific store. The same dialog
can explicitly select existing Encoder Gym scientific history. The main process
keeps that native path behind a project-scoped token; preview checks its exact
schema, SQLite integrity, and current runtime project, then connection creates a
transactionally consistent, content-addressed copy below the managed `runs/`
folder. It never attaches or changes the selected database in place.

When preview finds missing Python modules, the same dialog can repair the
explicitly selected environment after a second confirmation. The main process
accepts only the blocked preview token and invokes the fixed
`workspace prepare-nomos-python` intent. Package names are compiled into the
backend mapping, pip is non-interactive and binary-only, and the dialog states
that the chosen environment will change and network downloads may occur. If pip
is missing, the same app-owned intent restores it from Python's bundled offline
`ensurepip` module before installing the allowlisted packages; the user never
has to assemble a terminal command. The runtime is automatically re-previewed
after installation; storing a provider credential does not authorize this
operation.

`project.sqlite` is the workspace custody registry, not a legacy slice-run
database. Source datasets still need normal task-compatible admission, snapshot
and evaluation contracts before training. **Start optimization** now shows the
derived project readiness report and accepts a reviewed optimization definition
through a native file picker. It can reserve a real bound optimization and
advance it one persisted stage at a time. It does not turn imported source files
into training authority, infer a scientific store, or fabricate missing history.

Every meaningful managed-project command from the desktop records an immutable
activity action before it executes and a terminal outcome afterward. Long native
runs add bounded progress milestones. The Activity page verifies and replays
those hash-chained records and can export the complete stream as JSONL. Action
events carry UUIDs and safe artifact references only; credential values and
sealed or row-level evidence never enter the journal. Automatic polling,
navigation, window controls, and clipboard operations are intentionally not
project activity. Scientific journals remain the authority for exact training,
evaluation, gate, and decision evidence linked by those references.

The **legacy** reader opens `.sqlite`, `.sqlite3`, and `.db` files directly inside
the selected directory, read-only. It projects the existing
`encoder_experiment_projects`, `encoder_experiment_protocols`, and
`encoder_experiment_events` contracts; optimization links are optional.
It does not recursively discover databases or import arbitrary training logs,
model directories, or the other platform slices' database formats.

Models, metric names/directions, primary metrics, input references, checks, and
outcomes come from records. Historical attempts remain attached to their
candidate without merging evidence from separate attempts. Scores from
incompatible setups are separated; missing evidence is not zero and an
improvement does not override a failed requirement.

The renderer receives only development metrics and row-free provenance.
Sealed scores, rows, predictions, and native diagnostics are excluded. A run
may show the archival final-acceptance outcome, separately from development.
The reader checks report bindings and journal continuity, not native artifact
fingerprint reproduction. Loading records does not establish deployment,
promotion, current authority, or a successful CLI Doctor verification.

**Backend limitation:** the current `synth experiment` execution/status CLI is
wired to the Nomos adapter. The presentation and folder registry are generic,
but adding another encoder does not supply an execution adapter. Other
encoders need compatible backend integration producing the supported contracts.
The independent platform-slice CLI workflows remain available separately;
their records are not silently converted into experiment journals by this UI.
The desktop can configure separate encrypted provider credentials, resolve one
current approved repair into its immutable snapshot/run definition, execute the
fixed managed lifecycle, and explicitly promote a sealed-accepted checkpoint.
It does not invent the upstream scientific approval: if no reviewed hypothesis,
native delta, compatible suites, or successor benchmark exists, readiness says
which owning scientific step is missing. A managed run starts only when the
same immutable definition accepted by `synth workspace optimize` resolves.
External and sealed work remain separately authorized workflow boundaries.

Use `synth experiment --help` to inspect that command family. Run detail offers
CLI help and exact recorded IDs/source databases, not an assumed runnable
command for an unsupported adapter. `synth` must be installed or invoked from
the built Rust binary's path.

## Implementation boundaries

- `src/main.ts`: hardened Electron composition, folder picker, IPC handlers.
- `src/preload.ts`: typed bridge; renderer cannot read arbitrary filesystem paths.
- `src/project-registry.ts`: atomic, app-owned folder metadata only.
- `src/managed-backend.ts`: fixed Rust CLI adapter and native-picker tokens.
- `src/managed-control.ts`: typed readiness and finite optimization intents.
- `src/project-activity.ts`: typed immutable project-action/event contract.
- `src/credential-store.ts`: project/role-scoped encrypted secrets and fallback availability.
- `src/managed-workspace.ts`: row-free managed-workspace presentation contract.
- `src/renderer/onboarding.ts`: checkpoint-copy and JSONL-import dialogs.
- `src/projects.ts`: project identity/content types and stale-response guard.
- `src/evidence/read-workspace.ts`: read-only SQLite-to-presentation projection.
- `src/workspace.ts`: presentation contract, independent of SQLite and Electron.
- `src/renderer/`: vanilla TypeScript views, navigation and interaction state.
- `src/renderer/activity-page.ts`: action replay, UUID inspection, and JSONL export.
- `src/evidence/nomos-snapshot.json`: explicitly labelled historical example.
- `tests/fixtures/`: deterministic synthetic data, never registered in a normal
  user profile or presented as real trained projects.

No Rust core policy, acceptance algorithm, or backend contract is duplicated in
the renderer. The browser-only fallback is an in-memory example preview and
cannot register local folders; actual folder workflows require Electron.

For a bound project with one current approved repair, the Start optimization
page derives and explains that authority. Its primary preparation button uses a
fixed project-scoped intent rather than opening a TOML picker. The backend
replays the approved delta, creates or adopts the immutable logical training
snapshot, and publishes a content-addressed definition inside the managed
workspace. The renderer receives only an opaque token and resolved preview. No
model work, provider call, or sealed evaluation happens during preparation.
After restart, the main process rediscovers only the exact definition implied
by current persisted authority, reissues a fresh opaque token, and keeps the
path out of renderer state. Restoring the screen does not repeat native replay.

## Validation

```powershell
npm run check   # Rust CLI build, TypeScript, UI build, deterministic Node tests
npm run smoke   # Legacy + managed actual Electron interaction and restart checks
```

The smoke test launches four hidden Electron processes: legacy and managed
flows each get their own temporary profile and an independent restart. Legacy
checks register classification and similarity fixture
folders, switches and reopens them after process restart, and checks rename,
empty/baseline-only states, moved-folder recovery, non-destructive removal,
project-local search/selection, generic metrics, historical attempt provenance,
comparison, keyboard tabs, theme/width preferences, and responsive layout.
The test controls the folder picker's returned selection; the native OS dialog
itself is not automated. Experiment database bytes are checked unchanged.

Managed checks exercise New, checkpoint preview/cancel, copied baselines,
dataset preview/import and held-out rejection, two-project isolation, invalid
Open, moved-folder recovery with identity checks, verification, forget/reopen,
and responsive dialogs/dataset pages. A deterministic lifecycle fixture drives
the real renderer/preload/main IPC through approved-repair preparation, finite
preview, reservation, native-stage resume, sealed authorization, accepted
candidate status, and explicit baseline promotion. Additional checks cover
760×560 dialog actions and errors, Enter/Tab/Escape using hidden Chromium input,
slow-import duplicate/dismissal guards, long project names, persistent Open
failures, and revision-fenced verification after switching away and back.
Tiny custody-format model fixtures never
claim to be trained or executable encoders.

For an explicitly requested real-project read-only check, select a managed
project in the saved library and run `npm run verify:current`. It uses temporary
Chromium state, reads the actual saved library without rewriting it, verifies
model/data files through the backend, and captures Models/Data/Settings plus the
real Start-page readiness. Its structured result lists required blockers, any
exact run/authority, configured provider roles, pre-authorization external
calls, preparation state, and persistence. It accepts either the truthful
preparation action or a recovered prepared definition; in the latter state it
also requires the reservation action to be visible without scrolling. A
second verified reopen proves the saved library,
managed manifest, custody database, and workspace facts did not change.

Screenshots are written to ignored `ui/qa/`. Checked viewport sizes include
1440, 1280, 1024, 760, and 390 CSS pixels; the normal desktop window minimum
is 760 pixels. Small-screen tables scroll within their own containers.
Temporary fixture profiles remain available for debugging. Tests require no
Nomos checkout, model downloads, GPU, credentials, or external services.

Run the repository's four Rust gates as well: `cargo fmt-check`,
`cargo check-all`, `cargo lint`, and `cargo test-all`. Any deliberate real
workspace check must be separate, opt-in, and read-only.
