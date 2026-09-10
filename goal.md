# Goal: a usable encoder optimization workspace

Implement the product agreed with the user on 10 September 2026. A user supplies
a baseline encoder, a starting training dataset, and a shared evaluation
benchmark, then presses Optimize. A bounded agent manages the improvement work
and returns model artifacts, dataset changes, comparable results, and readable
progress. Changing labels or adding a button does not complete this goal.

The implementation contract and progress record is
`docs/encoder-workspace-product.md`. This replaces the previous goal's page
design and its outdated assertion that Nomos has no candidates. Preserve the
implemented custody, provenance, credentials, deterministic decisions, and
durable execution contracts.

## Required outcome

- A project-level Optimize action is reachable from every project page. Initial
  setup selects model, training dataset version, and evaluation benchmark
  version. Finite execution limits have sensible defaults and remain inspectable.
- Models is a simple inventory. Completed candidates remain visible even when
  rejected. All models open the same viewer with status, evaluations, training
  dataset, producing run, and artifact details. There is exactly one active
  baseline; promotion/restoration records history without changing model bytes.
- Data lists the base dataset and named derived datasets. Each dataset owns
  immutable snapshots, parent references, and row-level additions/removals.
  Users can create a variant, inspect its rows and changes, and follow history.
  Source files and hashes are details, never the organizing structure.
- Evaluation presents one project benchmark with immutable versions, a shared
  metric protocol, and results for models evaluated on that same version.
  Development and final holdout remain isolated internally. Changing a benchmark
  creates a new version and never makes old scores appear comparable.
- Runs presents one entry per optimization, readable ongoing work, results,
  model/dataset links, and technical details on demand. Routine internal stages
  advance automatically after Optimize; no repeated bookkeeping buttons.
- Bounded agent decisions and data generation use the project's separate
  provider authorities. Preserve deterministic acceptance, finite spending,
  cancellation, restart recovery, idempotency, and sealed-evidence isolation.
- Every meaningful project action has an action UUID and durable JSON events.
  Credentials and sealed content never enter activity logs.
- Use the same page layout and object-viewer patterns. Remove explanatory
  filler, inert count badges, repair/interpolation headings, and duplicate views.
- Existing managed projects and Nomos evidence remain usable. Do not silently
  adopt unrelated imported historical checkpoints as managed model artifacts.

## Execution and evidence

Implement coherent stages through core contracts, persistence, CLI, and typed
desktop integration. Commit verified stages. Required Rust gates and relevant
UI/offline renderer checks must pass. Verify against real Nomos state using
read-only inspection; do not launch paid/training/holdout work during development.

Completion requires the real end-to-end workflow and artifact journeys above,
verified by deterministic integration tests and actual rendered UI interaction.
Keep outstanding items explicit until they are implemented and verified.
