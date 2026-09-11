# Encoder workspace product contract

Agreed 10 September 2026. This supersedes page responsibilities in the earlier
managed-optimization integration design where they conflict.

## User-owned objects

A project owns models, training datasets, one versioned evaluation benchmark,
optimization runs, and provider settings. Optimize is a project action consuming
the exact baseline, dataset snapshot, and benchmark version. Internal strategy
names are configuration details, not model names or page headings.

Models and datasets share collection/card/history navigation but are different
domain objects: several models can train on the same dataset version. A model's
training version never changes when the project baseline changes. Imported
models with unknown training history must say so.

## Ownership

- Project workspace cores/adapters: model custody, baseline revisions, source
  copies, provider settings, and project activity.
- Dataset core and native training manifests: immutable membership/provenance.
  Dataset catalogs reference these facts; importing a file is not qualification.
- Experiment/workflow cores: benchmark contracts, evaluations, deterministic
  decisions, finite execution, and holdout exposure.
- Bounded agent runtime: permitted improvement actions using development
  evidence and finite provider budgets. No sealed input.
- CLI: application composition. Electron invokes fixed typed CLI operations.
- Renderer: read projections, navigation, and explicit user actions.

## Page contract

| Page | Collection | Detail |
| --- | --- | --- |
| Models | Baseline and every project model, including rejected outputs | Same viewer: overview, evaluation, configuration, provenance |
| Data | Base dataset and named variants | Rows, changes, snapshots, trained models |
| Evaluation | One project benchmark and its versions | Protocol, version membership, comparable model results |
| Runs | One row per optimization | Live work, outcome, artifacts, activity; optional technical record |
| Settings | Providers, limits, workspace | Focused configuration dialogs |

Optimize is visible in the project header. Artifact cards link to each other.
Navigation never depends on repair IDs, source filenames, or campaign/protocol
distinctions.

## Evidence and compatibility

Show completed outputs of this project's own optimization runs even when they
fail evaluation. Imported history alone does not establish project custody.
Completed outputs retain producing run, source model, dataset version, and
native artifact identity. Promotion is a separate baseline revision. Failed
attempts without checkpoints remain run activity, not fictitious models.

Dataset variants fork a selected snapshot. Diffs use stable row identities;
positions are display information only. Each immutable snapshot retains its
parent and change set. Training rows may enter an explicitly requested bounded
dataset viewer. This refines the previous blanket row-free renderer rule, while
scientific journal projections, sealed rows, and agent inputs retain isolation.

Benchmark versions pin membership and metric/protocol identities. Comparable
results require the same benchmark and evaluator configuration. New versions
preserve old results and do not reset holdout exposure.

Optimize must persist authorization and continue routine stages through the
existing optimizer. Preparation, generation, training, evaluation, and finishing
need recoverable IDs and activity events. Existing explicit holdout approval
remains a pause unless a persisted launch scope authorizes it. A bounded agent
extension is now explicitly requested; deterministic acceptance still owns the
decision. No unbounded work or inferred spending.

## Stages and acceptance evidence

1. Model inventory and navigation — implemented and verified on real Nomos.
   Unified viewers, rejected project outputs, project Optimize,
   and neutral run/model names are present. Verified dataset deep links are present;
   model-level baseline promotion/restoration controls remain outstanding.
2. Dataset versions and changes — backend, CLI, desktop collection and version
   viewer implemented. Immutable model links and imported-baseline dataset
   adoption and completed-candidate dataset adoption are implemented;
   new manually created versions are not historical training evidence.
3. Shared benchmark — model-independent definition, immutable project versions,
   and preview/adoption/inspection CLI implemented. Comparable results across
   owned bindings and typed desktop integration are implemented. Evaluation now
   offers one results matrix, version selection/history, protocol inspection,
   and guarded adoption from recorded authority. Initial test-data onboarding
   and pinning the selected version to automatic Optimize remain pending.
4. Automatic bounded optimization — the input-selection domain, persistence,
   CLI and desktop selection are implemented. The input-first launch scope now
   pins exact setup/provider revisions, separate provider ceilings, fixed finite
   defaults and one final evaluation behind a UUID authorization; its actual CLI
   persistence, redaction checks, and typed project-scoped Electron bridge pass.
   Automatic continuation of an exact existing run is implemented in the core
   and CLI. Renderer composition, agent/generation integration, managed automatic
   execution, output registration and complete-journey recovery remain pending.
   Verify real CLI composition using deterministic offline fakes.
5. Complete journey — pending. Renderer interactions for setup, Optimize,
   results, artifact inspection, and promotion; read-only Nomos verification.

### Baseline restoration contract — 11 September 2026

- A model that previously held baseline authority can be made active again by
  appending a restoration revision. The current revision and every model
  artifact remain unchanged in history; restoration never copies, moves,
  deletes, or rewrites a checkpoint.
- `workspace restore-baseline` requires the exact active revision, an earlier
  target revision, and a stable retry UUID. Stale requests and attempts to
  restore the already-active model fail closed. The operation records a
  payload-free `model.restore_baseline` activity chain.
- Model artifacts and baseline revisions now have database-level immutable
  update/delete guards. Core, local-adapter, and actual CLI process tests cover
  replay, stale state, exact history, and storage tampering. All four Rust gates
  pass.
- The model viewer still needs the compact Promote/Make baseline controls and
  the typed Electron restoration bridge. This backend stage does not change the
  real Nomos baseline or claim that input-first optimization execution exists.

No single stage establishes goal completion. Record validation as stages land
and keep outstanding requirements visible.

### Model inventory evidence — 10 September 2026

- All four Rust gates passed. UI typecheck/build and 52 unit tests passed.
  Offline Electron acceptance passed legacy and managed projects across two
  independent launches each, including common viewers and browser history.
- Completed-model registration is independent of evaluation acceptance.
  Registration replay preserves identity; later promotion reuses the existing
  artifact and appends a baseline revision. Tests cover changed provenance,
  unsupported custom transformer modules, and native/managed identity matching.
- The existing Nomos optimization output was verified and registered as
  `4cfde9aa-7a28-43bf-aa37-d372086c37a4`, linked to experiment
  `aad51ade-b04f-41c2-bbeb-4916fc1ae602` and training snapshot
  `2602ef88-db56-4591-8a4c-2581c5613726`. Its active baseline revision remains
  `a1aa8b5b-109c-4423-be7c-aa07c3e02caf`. This copied the already-completed
  checkpoint; no training, evaluation, provider call, or baseline change ran.
- The protected native import whitelist now accepts the exact newer standard
  transformer/pooling/normalization class paths emitted by the real trainer.
  Arbitrary custom code remains rejected.
- The read-only real-project Electron check opened both models in the common
  four-tab viewer, showed the rejected candidate's recorded evaluation, visited
  all project pages and optimization preparation, and confirmed byte-identical
  manifest/database/library state. Rendered captures are under `ui/qa/managed-current-*`.

### Dataset versioning progress — 10 September 2026

- `dataset-core::versions` owns native-schema-independent source references,
  logical row IDs, branches, immutable versions, and add/remove/replace diffs.
  Four pure tests cover reconstruction, replacement identity, malformed changes,
  source/project fencing, and tampered fingerprints.
- Managed project migration 6 adds append-only branches and versions. Creation,
  forks, and edits preserve exact parents and retry IDs. Paged rows and before/
  after diffs read verified training imports only. Three persistence tests cover
  retained history, stale edits, source corruption, protected data, and isolation.
- `workspace dataset` exposes list/create/fork/revise/inspect/rows/changes.
  A real CLI process test covers the base-to-variant-to-diff journey, reopening
  after moving the project, idempotent save, and payload-free activity events.
- The typed Electron bridge exposes only fixed project-bound requests. Three
  bridge tests reject renderer paths, invalid pages, and substituted identities,
  and verify temporary request cleanup. UI typecheck/build and 55 tests pass.
- No real Nomos datasets have been created, changed, or reclassified by this
  stage. Existing logical training snapshots still need verified links to the
  new catalog; matching names or row counts will not suffice.
- Implementation details and CLI grammar are in `dataset-versions-spec.md`.
- All four Rust gates pass for this stage. The complete CLI, core, persistence,
  and existing scientific-governance regression suites passed without real
  training or external provider use.
- Existing legacy/managed Electron journeys also passed in four launches after
  adding the desktop bridge. This protects current screens; it is not evidence
  that the new dataset viewer exists or that its future interaction is complete.

### Dataset interface evidence — 10 September 2026

- Data now lists base/variant objects using the Models inventory layout. The old
  snapshot authority box, source-file cards, and duplicate purpose/import dialog
  are removed. Details use Rows, Changes, Versions, and Models tabs.
- Users can create a base from training imports, fork an exact version, add rows,
  remove a stable row ID, and replace a row from a one-record JSONL file. The
  viewer shows persisted versions and before/after evidence without rewriting
  history. Native JSON is explicitly inspectable in a bounded dialog.
- The offline Electron journey exercises all three edits, four variant versions,
  paged inspection/back navigation, held-out/source-change rejection, a lost
  mutation response after commit, project isolation, folder movement, and restart.
  Desktop and narrow row/diff/collection captures were inspected under `ui/qa/`.
- Six new controller tests cover duplicate requests, late failed reads, moved
  folders, lost save/list responses, invalid edit targets, and bounded per-project
  page caches. UI typecheck/build and all 61 unit tests pass.
- Folder recovery invalidates queries to the old location. Save retries retain
  exact version IDs; busy controls cannot submit duplicate work.
- All four Rust gates pass for this UI stage; the full deterministic suite is
  recorded in `target/dataset-ui-rust-tests.log` (local test output, not a tracked
  artifact). Renderer code calls the typed dataset bridge; row membership and
  mutation rules remain in the existing Rust dataset contracts.
- Nomos's two custody imports and native 6,992-row training snapshot have not yet
  been adopted into these dataset objects. Verified adapter-owned membership
  links, benchmark versions, baseline promotion/restoration controls, and the
  full automatic bounded agent journey remain required by the goal.
- A read-only CLI check of the real Nomos project confirms two model artifacts,
  two training imports, and no dataset catalog entries yet. The candidate still
  pins native snapshot `2602ef88-db56-4591-8a4c-2581c5613726`; its identity must
  not be replaced with a guessed catalog version.

### Recorded model training datasets — 10 September 2026

- Workspace core owns an immutable model-to-version link, with ordered native
  input identities and either imported final-stage or completed-training evidence.
  Dataset membership remains dataset-core-owned. Model and native snapshot IDs
  are unchanged. Migration 7 makes links append-only.
- The CLI can adopt the imported model's recorded inputs, reusing the same
  dataset/version after a partial write. Reads verify exact membership, source
  custody, manifest checksum/counts/order, and model/version fingerprints.
- Three local tests cover provenance, immutability, moved folders, edited
  dataset history, input-order tampering, and interrupted-link retry. The CLI
  process test checks exact replay IDs and payload-free activity UUIDs.
- All four Rust gates, UI typecheck/build, 61 unit tests, and the four-launch
  offline Electron suite pass. The renderer journey follows model -> exact
  dataset version -> trained model through the actual CLI and IPC contracts.
- Nomos baseline `b12bb044-0c7c-4ad6-99f6-91a2b4c44bb7` now links to Base
  dataset version `9448e8ca-bae2-8689-90ae-73a26d0b3c49` (6,800 rows).
  Adoption action `3f7453f1-0ba8-4194-81a9-8e6d32925485` records the operation.
  Full verification passed; original import metadata, model catalog, model/data
  bytes, baseline revision, and project manifest are unchanged. No training,
  provider calls, or holdout execution ran.
- The native candidate's 6,992-row dataset still needs adapter-verified adoption.
  Its manifest orders the two base inputs differently; the model link must
  preserve that order independently of dataset display order. Shared benchmark,
  promotion controls, and the complete automatic agent flow remain outstanding.
- Real-row paging initially timed out in the renderer. The read path now hashes
  only requested row payloads while retaining full source checksum/partition
  validation, and avoids redundant project opens and version reconstruction.
  A measured 25-row Nomos CLI read fell from 27.7s to 8.7s in the debug build.
  Sparse-page and off-page corruption tests preserve row identities and fail
  closed on changed source bytes. Dataset previews show questions, not internal
  decision-state IDs, when question text is available.
- The real-project Electron check passed the baseline -> 6,800-row version ->
  baseline journey, both model viewers, all project pages and preparation. The
  final verified reopen confirmed unchanged library, manifest, custody database
  and workspace facts. Captures include `ui/qa/managed-current-datasets.png` and
  `ui/qa/managed-current-linked-dataset.png`. This inspection did not start a run.

### Completed model datasets — 11 September 2026

- Completed Nomos fine-tunes now receive verified dataset links during normal
  output registration. `workspace dataset <folder> adopt-run <optimization>`
  recovers links for already-registered outputs without rerunning the experiment.
  The native adapter verifies its receipt and exact ordered inputs; the CLI
  verifies scientific completion; the workspace adapter owns imports, versions
  and links. Domain objects do not depend on the native adapter or SQLite.
- Identical training populations reuse a dataset version. Changed populations
  fork the starting model's recorded version and save additions/removals. Native
  input order is preserved independently of dataset row order. Interrupted
  adoption reuses existing imports, fork, revision and link identities.
- Six local model-dataset tests cover baseline/candidate links, changed and
  unchanged populations, interrupted persistence, moved folders, input-order
  tampering and lightweight-versus-full validation. Native receipt tests verify
  that adoption needs neither an executable Python nor available holdout files,
  rejects changed sources, and never recreates a missing training receipt.
- Routine project opens verify metadata references and the small training
  manifests without reconstructing every dataset's ancestry. Explicit full
  verification, row/diff reads and mutations retain complete membership checks.
  A measured Nomos metadata open took 0.22 seconds in the debug build. Full file
  and membership verification remains an explicit, more expensive operation.
- All four Rust gates passed, as did UI typecheck/build, 61 unit tests and the
  four-launch offline Electron suite. Full Rust output is recorded locally in
  `target/candidate-dataset-rust-tests.log`; Electron output is in
  `target/candidate-dataset-electron-tests.log`.
- Nomos candidate `4cfde9aa-7a28-43bf-aa37-d372086c37a4` now links to
  `Candidate 01 dataset`, version `a9edeb9e-63ce-81ad-8759-9495e82d6c30`:
  6,992 rows, with 192 additions to the unchanged 6,800-row base. Its native
  snapshot identity and manifest input order are preserved. Adoption action
  `360eb8a5-ca9c-4726-bf00-905188f67e3f` links the optimization, model and version.
  The original import records, model catalog, baseline link, project manifest
  and scientific database are unchanged. No training, provider or holdout work
  ran; only the verified delta import and dataset/link/activity records were added.
- The real read-only Electron journey passed both model -> exact dataset ->
  model paths, the candidate's Changes/Versions tabs, all project pages and
  preparation. Initial/final full verification and byte comparisons proved no
  inspection writes to the saved library, manifest or custody database. The
  candidate diff and Data collection captures were visually inspected under
  `ui/qa/managed-current-*`; the result is in
  `target/candidate-dataset-current-ui.log`.
- This completes the recorded candidate-data integration, not the overall goal.
  Shared benchmark versions/results, model-level baseline promotion/restoration,
  and the automatic bounded agent journey with complete offline end-to-end
  acceptance remain outstanding.

### Shared benchmark catalog — 11 September 2026

- Experiment core now owns a model-independent benchmark definition. It pins
  suite membership, roles, metric rules and adapter-normalized evaluation
  configuration. Baseline identities, training recipes and run IDs are excluded.
- Managed projects retain immutable numbered versions and original scientific
  provenance. The CLI previews, adopts, lists and deeply inspects recorded
  definitions without running evaluation or reading native holdout files.
- Pure, persistence and CLI process tests cover compatible model changes,
  incompatible evaluator changes, sealed-score exclusion, exact source binding,
  immutable history, stale writes, retries, tampering and folder movement.
- All four Rust gates passed, along with UI typecheck/build, 61 unit tests and
  the four-launch offline Electron suite. Local outputs are recorded in
  `target/project-benchmark-rust-tests.log`, `target/project-benchmark-ui-tests.log`
  and `target/project-benchmark-electron-tests.log`.
- A read-only preview of Nomos experiment
  `aad51ade-b04f-41c2-bbeb-4916fc1ae602` reproduced its shared definition in
  0.13 seconds, with byte-identical custody and scientific databases. No real
  benchmark version was adopted and no evaluation, provider or holdout work ran.
- Comparable model results across historical bindings, desktop integration and
  test-data onboarding remain required; this catalog alone does not complete
  the Evaluation page or authorize an optimization.

### Shared benchmark results — 11 September 2026

- `workspace benchmark <folder> results <version>` now starts with every managed
  model and attaches only exactly compatible development reports. Rejected
  candidates remain represented; an empty report list means Not evaluated.
- The workspace core owns the safe model/report projection. CLI composition
  resolves all verified project-owned scientific bindings, including intermediate
  historical bindings in different stores. SQLite owns the project-scoped lookup;
  the native adapter owns benchmark normalization. No metric policy was copied
  into the workspace adapter or renderer.
- Registered outputs require the exact source model and producing-run receipt.
  Baseline evidence maps through each binding's original baseline revision.
  Repeated reports retain distinct identities; reuse of a single report retains
  every run context. Old verdicts are not reinterpreted after a baseline change.
- CLI process tests cover three owned bindings, foreign co-located projects,
  incompatible benchmark definitions, registered and unregistered candidates,
  missing evaluations, retries, folder movement and byte-identical read-only
  databases. Projection tests reject receipt/source substitution, altered
  journals and report-ID collisions without partially updating results.
- The typed read-only desktop bridge rejects renderer paths/commands, foreign
  version responses, substituted model inventories, changed baseline roles,
  non-finite scores and protected-suite reports. It is ready for the replacement
  Evaluation screen, but no new results table has been rendered in this stage.
- All four Rust gates, UI typecheck/build, all 63 unit tests and four offline
  Electron launches passed. Local logs are `target/benchmark-results-rust-tests.log`,
  `target/benchmark-results-ui-tests.log` and
  `target/benchmark-results-electron-tests.log`.
- Read-only Nomos inspection confirms both `Nomos baseline` and `Candidate 01`
  remain registered, with active revision
  `a1aa8b5b-109c-4423-be7c-aa07c3e02caf` and byte-identical manifest and custody
  database. It still has no adopted project benchmark version. This stage did
  not change Nomos, start a run, call a provider or expose protected evidence.
- Remaining work includes the version selector/results page, benchmark onboarding,
  model promotion/restoration controls and the complete bounded automatic Optimize
  journey. The overall goal remains incomplete.

### Shared Evaluation interface — 11 September 2026

- Evaluation now shows all registered models against one selected benchmark
  version. Results, Protocol and Versions share the normal page/viewer layout.
  Rejected candidates remain visible, missing results say Not evaluated, and
  distinct repeated measurements remain inspectable rather than selecting a
  favorable score. Model links open the common viewer. Managed Models no longer
  duplicates comparison selection controls.
- Choosing a recorded benchmark previews verified authority and saves only that
  exact definition and parent. Existing definitions open their original version.
  The CLI rejects a definition changed since preview; interrupted save/reload
  retries preserve the same request. Controller tests fence late reads, isolate
  per-version errors, bound caches and prevent stale mutation retries after a
  fresh read or new selection.
- The offline Electron journey uses a test-only Rust fixture to create normal
  scientific journals and managed model custody, then uses production CLI/IPC
  for preview, adoption, model comparisons, two versions, report inspection,
  model links, browser history and restart. Native metadata and protected scores
  stay absent; scientific database bytes remain unchanged. Desktop, report and
  narrow layouts are visually inspected under `ui/qa/managed-benchmark-*`.
- All four Rust gates, UI typecheck/build, 69 unit tests and four offline
  Electron launches passed. Logs are `target/benchmark-page-rust-tests.log`,
  `target/benchmark-page-ui-tests.log` and
  `target/benchmark-page-electron-tests.log`.
- Real Nomos read-only verification passed both model/dataset journeys, all
  project pages, optimization status and the recorded-benchmark preview/cancel
  interaction. The saved library, manifest, custody database and verified
  workspace facts are unchanged. Nomos still has no adopted catalog version;
  no run, provider or protected evaluation was started. The result is recorded
  in `target/benchmark-page-current-ui.log`, with the preview capture under
  `ui/qa/managed-current-benchmark-preview.png`.
- Initial benchmark test-data onboarding, model promotion/restoration controls,
  and the complete automatic bounded optimization journey remain outstanding.
  This interface stage does not establish goal completion.

### Input-first optimization setup — 11 September 2026

- `OptimizationInputs` binds the active model/baseline revision, one exact
  training dataset version and one exact shared benchmark version. It neither
  changes the baseline's historical training link nor chooses an old repair
  recipe. The domain consumes existing artifact contracts and owns no trainer,
  evaluator, qualification or acceptance policy.
- The custody adapter stores append-only setup revisions in migration 9.
  Preview is read-only, including for older project databases. Save verifies
  custody and selected references, rechecks the baseline under the write
  transaction, and requires the expected previous setup. Exact request retries
  return their original revision without reactivating historical settings.
- `workspace optimization-setup` provides preview, audited save and history
  through actual CLI processes. Tests cover changed selections, same-model
  baseline revision changes, stale parents, foreign/substituted inputs, test and
  empty membership, changed source bytes, immutable storage, tampered history,
  schema upgrade, folder movement and payload-free activity UUIDs.
- All four Rust gates passed, along with UI typecheck/build, 69 unit tests and
  the existing four-launch Electron suite. Logs are
  `target/optimization-setup-rust-tests.log`,
  `target/optimization-setup-ui-tests.log` and
  `target/optimization-setup-electron-tests.log`. These prove backend behavior
  and desktop compatibility, not the still-missing input-first GUI journey.
- A real read-only Nomos setup-history query returned no saved setups and
  byte-identical manifest and custody database. No setup, migration, training,
  provider call or protected evaluation was performed on the real project.
- This is a saved configuration, not a runnable optimization or authorization.
  No new setup was written to Nomos. Desktop setup, fresh benchmark onboarding,
  execution scope/budgets and the full bounded agent/optimization integration
  still have to consume this selection end to end. See
  `optimization-launch-spec.md` for the contract and remaining execution boundary.

### Desktop optimization inputs — 11 September 2026

- Project Optimize now opens three compact input rows: the current baseline,
  a named dataset version, and a shared benchmark version. Each opens its
  ordinary artifact viewer. Missing inputs link to Data or Evaluation; a saved
  version does not silently follow later dataset or benchmark versions.
- The fixed desktop bridge invokes the existing setup preview/save/list CLI.
  It rejects paths, execution settings, secrets and foreign/substituted inputs.
  Save retains the exact retry request after a lost save or reload response;
  changed inputs and refresh abandon that retry. Reopening/moving a project
  fences late responses, and retries reload current history without reactivating
  an older setup. Full custody verification stays in the local Rust adapter.
- Existing optimization supervision remains in Runs. The new entry screen no
  longer starts a previously prepared recipe unrelated to the user's selection.
  It explicitly says automatic execution is not connected and that saving
  inputs does not start a run. No new execution authority is inferred.
- All four Rust gates passed, as did UI typecheck/build, 80 unit tests and four
  isolated Electron launches. The actual renderer/IPC/CLI journey tests missing
  and multiple datasets, all three artifact links, save, a lost reply after
  commit, exact retry, changed data/benchmark selection, immutable history,
  narrow layouts and restoration in another Electron process. Scientific
  database bytes remain unchanged. Screenshots were inspected at desktop and
  narrow widths under `ui/qa/managed-optimization-inputs*`.
- Read-only Nomos verification passed both model/dataset viewers, project pages,
  benchmark preview and the new Optimize entry. Its baseline-linked dataset is
  selected, while evaluation correctly requires a catalog version. The saved
  library, manifest, custody database and verified workspace facts are unchanged.
  No setup, provider call, training or protected evaluation ran on Nomos.
- Local evidence: `target/optimization-inputs-rust-tests.log`,
  `target/optimization-inputs-ui-tests.log`,
  `target/optimization-inputs-electron-tests.log`, and
  `target/optimization-inputs-current-ui.log`.
- Remaining work is still the complete automatic bounded optimization journey:
  launch scope/budgets, adapter-owned selected-data admission/materialization,
  agent and generation integration, automatic continuation and recovery. Fresh
  benchmark onboarding and model-level promotion/restoration are also pending.

### Automatic run continuation — 11 September 2026

- `encoder optimize drive` records authorization for an exact reserved run and
  executes ordinary stages through the existing handler until completion or a
  separate protected-evaluation approval. Manual `resume` still steps once.
  It does not select an old recipe in response to the new desktop inputs.
- Core owns the authorization event and strictly decreasing continuation rank.
  The new event uses schema version 2; historical event bytes stay unchanged.
  Exact retries retain one authorization and the original reserved child IDs.
  A second authorizer cannot replace an active run's authorization.
- One worker lease spans the drive. Between stages, persisted cancellation is
  reloaded. Errors stop the invocation; explicit recovery uses the existing
  child contracts without resetting budgets or repeating completed work. Status
  distinguishes a live worker from waiting, and activity records authorization.
- Two pure tests and six new actual-CLI scenarios cover finite paths, malformed
  or repeated authorization, final-evaluation approval, child-link interruptions,
  completed protected-report recovery, training failure, cancellation during
  preparation, duplicate workers and terminal replay. All 11 optimization CLI
  scenarios pass with deterministic fixtures and zero provider/native calls.
- Full-suite validation exposed a pre-existing WAL checkpoint race in a
  read-only database fixture. Its writer now checkpoints before the before/after
  comparison; the byte-equality requirement and read-only production code are
  unchanged. Failure output no longer dumps the complete database byte arrays.
- All four Rust gates passed after that fixture correction. UI typecheck/build,
  80 unit tests and four isolated Electron regression launches also passed.
  Evidence is recorded locally in `target/automatic-drive-rust-tests-final.log`,
  `target/automatic-drive-ui-tests.log` and
  `target/automatic-drive-electron-tests.log`. No real Nomos execution, provider
  request, protected evaluation or app restart was performed.
- This is an execution primitive, not goal completion. The input-first launch
  must still admit the selected dataset, pin finite budgets and use the bounded
  agent/generation contracts. Managed integration, benchmark onboarding,
  promotion/restoration controls and the complete Optimize journey remain open.

### Model baseline controls — 11 September 2026

- The model viewer now owns baseline changes. A candidate that passed the
  recorded final decision exposes `Make baseline`; a historical baseline
  exposes `Restore baseline`; the active baseline has no redundant action.
- Restoration appends a new baseline revision and advances the active pointer.
  It never rewrites model artifacts or earlier revisions, and stale renderer
  state is rejected against the exact active revision observed by the user.
- The desktop bridge accepts only revision UUIDs, generates the new revision
  identity internally, invokes one fixed project-scoped CLI command and verifies
  the returned project, target model and appended restoration record.
- Restoration is recorded by the CLI in the project Activity journal as
  `Baseline restored`, with action/event UUIDs and revision references.
- UI typecheck/build, all 83 unit tests and the full four-launch Electron
  acceptance suite pass, including the model-view restoration journey. No real
  project, provider, training process or protected evaluation was touched.

### Project optimization run root — 11 September 2026

- Input-first Optimize now has a project-owned run identity above slice-owned
  advisor, generation, training and evaluation work. It binds one exact launch
  and setup without translating them into a repair/interpolation recipe.
- `workspace optimization-run start` authorizes and idempotently reserves one
  queued run. Exact retries return the same run; an unreserved old authorization
  is rejected after baseline, inputs or provider settings change.
- The run definition and its first hash-chained event are immutable custody
  records. `list` and `show` verify their columns, JSON identities, launch
  binding and journal head without upgrading older project databases.
- This is the durable root for execution, not the executor. It performs no
  provider, training or evaluation work. The next stage must attach bounded
  child operations and transition this journal before the desktop can honestly
  make Optimize a one-click run action.
- `cargo fmt-check`, `cargo check-all`, `cargo lint` and `cargo test-all` pass.
  The actual CLI integration test covers reservation, exact retry, stale
  authorization, read-only history, immutable storage and redacted activity.

### Recoverable input verification — 11 September 2026

- A project run now records retryable preparation attempts instead of remaining
  silently queued. The states are `queued`, `preparing`, `ready`, and
  `preparation_failed`; every transition is part of the run's immutable
  hash-chained event journal.
- The verified receipt pins the exact baseline model, dataset version, shared
  benchmark, provider revision, scientific binding, runtime project and
  compiled adapter. It contains row counts and suite keys, but no dataset row,
  secret or final-holdout payload.
- `workspace optimization-run prepare` deeply re-reads selected source rows,
  reproduces the benchmark from its original protocol, checks that the current
  runtime can execute the same benchmark, and verifies native files. It makes
  no provider call and executes no training or evaluation.
- Failed verification records a stable code and can retry the same run after a
  runtime repair. Typed persistence rejects a forged runtime receipt even when
  its own fingerprint is internally valid.
- This closes input custody only. The next stage must materialize the selected
  dataset through the adapter and attach the first bounded advisor/training
  child before the desktop can start real improvement work.
- `cargo fmt-check`, `cargo check-all`, `cargo lint` and `cargo test-all` pass.
  The actual CLI integration tests cover retry after a missing runtime,
  rejection of a forged receipt, idempotent preparation and compatibility with
  the currently bound task adapter without executing sealed evaluation.

### Adapter-owned training data materialization — 11 September 2026

- A prepared project run now advances through `materializing`, `materialized`
  or `materialization_failed` in the same immutable hash-chained journal.
  Preparation and materialization retain separate retry counters and receipts.
- The project layer replays the exact selected dataset version but remains
  schema-neutral. The Nomos adapter validates accepted train-partition decision
  states, registry/label consistency, usable positive-negative examples,
  duplicate identities and every managed content fingerprint.
- Valid rows are canonically rendered below the owning run and dataset version.
  A complete dataset and row-free receipt publish together by directory rename;
  retries must reproduce the existing artifact exactly and never overwrite it.
- The adapter returns a new scientific project snapshot whose sole training
  input is that artifact. Its baseline remains the selected model and its
  development/final suites remain the shared benchmark. Existing snapshots are
  reused by stable source identity after an interrupted journal append.
- The actual CLI test covers a failed native attempt, exact retry, typed
  completion and idempotent replay; adapter tests cover row rejection and
  byte-identical publication. No model, provider or evaluation is executed.
- `cargo fmt-check`, `cargo check-all`, `cargo lint` and `cargo test-all` pass.
- The remaining execution boundary is the first finite candidate/advisor child
  attached to this exact materialized project.

### Shared baseline evidence references — 11 September 2026

- A materialized input-first project can now prepare its candidate protocol
  from the selected shared benchmark without evaluating the unchanged baseline
  again. Development and final baseline reports become explicit references,
  never unlabelled copies.
- Every reference binds the source scientific project, source protocol and
  source report UUID/fingerprint. The referenced report is bound to the new
  project and must retain the exact model, role, suite, metrics and support.
- The runner deeply verifies the benchmark against its source protocol and
  recovers an already-created referenced protocol idempotently. Tests prove
  protocol preparation performs zero evaluator calls and that ordinary
  candidate comparison still works against the referenced development facts.
- This removes redundant baseline work only. It does not authorize candidate
  training, provider calls or final evaluation. The next boundary remains the
  finite execution child attached to the project run.

### First finite experiment attachment — 11 September 2026

- A materialized project run now advances through retryable experiment
  attachment to `ready_to_run`. It records one row-free child receipt containing
  the exact materialized project, benchmark source, candidate, protocol and
  experiment-run identities.
- Candidate, protocol and experiment UUIDs are deterministically reserved from
  the parent run. Retry after child persistence but before the parent journal
  append reuses the same immutable children.
- The Nomos adapter, not project custody or presentation, compiles the explicit
  conservative first training candidate. The launch envelope contributes only
  a finite per-model time ceiling.
- Attachment reopens and verifies the native materialization, target project,
  benchmark source protocol and current adapter. It prepares referenced
  baseline evidence without trainer, evaluator or provider calls.
- Core, adapter and actual-CLI tests cover stable child identities, attachment
  recovery, typed custody and idempotent replay. The fixture's absent native
  runtime records an honest failed attempt before typed recovery.
- Candidate development execution and progress projection remain the next
  boundary; `ready_to_run` does not claim that training has started.
