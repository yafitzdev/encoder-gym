# Encoder Gym desktop

A local, multi-project Electron workspace for inspecting encoder baselines,
candidates, and their recorded development evidence. The app starts with an
empty project collection. Nomos is an opt-in recorded example, not the product
identity or a default connection.

## Launch

Use Node.js 22.19 or newer and npm. From the repository root:

```powershell
cd ui
npm ci
npm start
```

`npm start` builds and opens the desktop app. Close an older running instance
before launching changed code; a second normal launch focuses that instance.

## Project folders and page responsibilities

Use **Add project folder** to register an existing local directory. The native
folder picker also supports creating an empty directory. Registration only
adds an app entry; it does not initialize a model, database, or experiment.

- **Project folders:** persistent entries in the sidebar. Selecting a folder
  opens its Models page. Previously registered folders remain available.
- **Models:** the baseline reference and all unique candidate identities,
  grouped by compatible baseline, benchmarks, metric contract, and evaluation
  policy. Search and filter, select a benchmark, sort by its primary score,
  or compare up to three candidates from one setup.
- **Candidate detail:** exact development checks, training configuration and
  immutable input references, model artifact, and historical attempts.
- **Runs:** immutable experiment records. A run owns its candidate attempts,
  activity, recorded budgets, provenance, and final decision. Completion does
  not mean the candidate was accepted.
- **Benchmarks:** what is measured and why evaluation setups are separate.
- **Project settings:** app name, durable organizational ID, folder association,
  evidence source, rename/reconnect, and removal from the collection.

Removing an entry never deletes project files. Reconnect a moved folder using
**Locate folder**; this preserves its organizational ID. Duplicate registration
selects the existing entry. Empty projects and registered baselines with no
runs have dedicated states; missing, corrupt, and unsupported evidence have
recovery guidance rather than borrowing another project's results.

The folder collection and selected project persist in `projects.json` under
Electron's user-data directory (normally `%APPDATA%/@encoder-gym/ui` on Windows).
Theme and sidebar width also persist. Search, filters, comparison selection,
and page history are isolated per project during the session; those view
settings are not persisted across app restarts. A corrupt collection is
reported without overwriting the original file.

## Supported evidence and boundaries

The local reader opens `.sqlite`, `.sqlite3`, and `.db` files directly inside
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
Training, generation, evaluation, approvals, and promotion remain explicit
CLI operations. The desktop never starts them or contacts external services.

Use `synth experiment --help` to inspect that command family. Run detail offers
CLI help and exact recorded IDs/source databases, not an assumed runnable
command for an unsupported adapter. `synth` must be installed or invoked from
the built Rust binary's path.

## Implementation boundaries

- `src/main.ts`: hardened Electron composition, folder picker, IPC handlers.
- `src/preload.ts`: typed bridge; renderer cannot read arbitrary filesystem paths.
- `src/project-registry.ts`: atomic, app-owned folder metadata only.
- `src/projects.ts`: project identity/content types and stale-response guard.
- `src/evidence/read-workspace.ts`: read-only SQLite-to-presentation projection.
- `src/workspace.ts`: presentation contract, independent of SQLite and Electron.
- `src/renderer/`: vanilla TypeScript views, navigation and interaction state.
- `src/evidence/nomos-snapshot.json`: explicitly labelled historical example.
- `tests/fixtures/`: deterministic synthetic data, never registered in a normal
  user profile or presented as real trained projects.

No Rust core policy, acceptance algorithm, or backend contract is duplicated in
the renderer. The browser-only fallback is an in-memory example preview and
cannot register local folders; actual folder workflows require Electron.

## Validation

```powershell
npm run check   # TypeScript, build, deterministic Node tests
npm run smoke   # Build and actual Electron interaction + restart checks
```

The smoke test launches two separate hidden Electron processes sharing an
isolated temporary profile. It registers classification and similarity fixture
folders, switches and reopens them after process restart, and checks rename,
empty/baseline-only states, moved-folder recovery, non-destructive removal,
project-local search/selection, generic metrics, historical attempt provenance,
comparison, keyboard tabs, theme/width preferences, and responsive layout.
The test controls the folder picker's returned selection; the native OS dialog
itself is not automated. Experiment database bytes are checked unchanged.

Screenshots are written to ignored `ui/qa/`. Checked viewport sizes include
1440, 1280, 1024, 760, and 390 CSS pixels; the normal desktop window minimum
is 760 pixels. Small-screen tables scroll within their own containers.
Temporary fixture profiles remain available for debugging. Tests require no
Nomos checkout, model downloads, GPU, credentials, or external services.

Run the repository's four Rust gates as well: `cargo fmt-check`,
`cargo check-all`, `cargo lint`, and `cargo test-all`. Any deliberate real
workspace check must be separate, opt-in, and read-only.
