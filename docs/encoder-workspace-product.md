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
   and neutral run/model names are present. Dataset deep links await stage 2;
   model-level baseline promotion/restoration controls remain outstanding.
2. Dataset versions and changes — backend, CLI, desktop collection and version
   viewer implemented. Model links and verified Nomos dataset adoption remain
   pending; new manually created versions are not historical training evidence.
3. Shared benchmark — pending. Version selection and one results table per exact
   benchmark version. Verify incompatibility and sealed isolation.
4. Automatic bounded optimization — pending. Persisted launch scope, agent and
   generation integration, routine continuation, output registration, recovery.
   Verify real CLI composition using deterministic offline fakes.
5. Complete journey — pending. Renderer interactions for setup, Optimize,
   results, artifact inspection, and promotion; read-only Nomos verification.

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
