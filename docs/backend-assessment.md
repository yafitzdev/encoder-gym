# Backend feature assessment

Assessed on 2026-09-08 from commit `d7810d9`, on
`codex/backend-stabilization`. This branch is checked out at
`.worktrees/backend-stabilization` beneath the original repository. The GUI
session retains the original checkout and its changes on `main`.

This is an assessment checkpoint, not a claim that the backend has been fully
audited or that every implemented path is production-proven. No product code,
live database, model artifact, or GUI file was changed for this assessment.

## Overall assessment

The backend implements all six original slices. It is a substantial local
experiment platform, with 44 Rust workspace crates, SQL migrations, a CLI,
an older Slice 1 HTTP server, and a separate TypeScript Pi sidecar. The next
phase should stabilize existing behavior and contracts before adding more
feature families.

The strongest foundation is the classification workflow: explicit generation,
immutable snapshots, local training, persisted predictions, deterministic
analysis, and reviewed optimization. The newer production optimization layer
has useful independent contracts, but its top-level orchestration needs more
ordinary end-to-end coverage.

## Implemented capabilities

| Area | What exists | Evidence and practical boundary |
| --- | --- | --- |
| Synthetic generation | Dataset schemas, arbitrary categorical dimensions, Cartesian coverage, unequal targets, deterministic allocation, hybrid field recipes, fake and OpenAI-compatible backends, validation, exact/normalized deduplication, durable attempts and bounded retry | Core and SQLite tests; `hybrid_construction_cli`, `allocation_cli`, `cli_e2e`, and workflow resilience tests. Semantic deduplication is deliberately absent. Live-provider behavior is opt-in. |
| Dataset management | JSONL/CSV import, accepted/rejected provenance, immutable snapshots, deterministic label-stratified and optional group-aware splitting, statistics, pagination, exports | Dataset core, import, persistence, quality, and CLI tests. Group indivisibility constrains achievable split ratios. Legacy snapshots can remain unqualified. |
| Training and checkpoints | Hashing-linear classifier, real local BERT training, frozen/fine-tuned encoder modes, bounded batches, checkpoint checksums, prediction and explicit continuation | Training runner tests, deterministic linear tests, tiny BERT tests and `transformer_cli_e2e`. The generic transformer adapter supports exact BERT/F32/CPU bundles; it is not a general Hugging Face or GPU trainer. |
| Evaluation | Batched checkpoint inference, persisted predictions, accuracy and class/slice metrics, confidence/calibration summaries, paired statistical comparison and benchmark gates | Evaluation core and SQLite tests plus CLI journeys. Comparisons depend on exact compatible inputs and protocols; metrics remain reproducible from evidence. |
| Error analysis | Persisted error grouping, minimum support, bounded representative evidence, uncertainty, overlapping-slice coverage, comparison diagnosis and append-only reviews | Analysis tests and `cli_e2e`. This consumes stored predictions; it does not infer root causes or rerun a model. |
| Classification optimization | Immutable protocols, explicit scoring policies, constrained finite allocation, scenario comparisons, bounded training suggestions, approval, stale rejection/rebase, idempotent plan application and campaign outcome assessment | Optimization core, SQLite and CLI tests. Applying a proposal creates a plan; the separate governed workflow owns execution. |
| Data quality and optional advice | Semantic profiles, authenticity research, dataset/benchmark architects, blind quality evaluation, reviewed curation, scoped generation supervision and canary-reviewed guidance | Fake/local process tests across the relevant core, runner, SQLite and CLI components; 11 TypeScript sidecar tests. Live research/provider quality was not revalidated here. |
| Governed workflows | Strict project preparation/bootstrap, training-to-benchmark contamination checks, development/sealed roles, approval envelopes, finite budgets, durable child identities, recovery, stopping and promotion records | Preparation, bootstrap, governance, resilience and integrated CLI tests. Training/validation input authority and sealed-evidence isolation are explicit contracts. |
| Production encoder experiments | Task-neutral experiment/repair/campaign contracts, native delta qualification, immutable logical training snapshots, renewable benchmarks and `encoder optimize` stages | Fake experiment runner tests, campaign-core tests and SQLite integrity tests. The shipped production CLI is statically composed with Nomos; another task requires another compiled adapter. |
| Operations | JSON/human output, inspection, provenance, database/artifact Doctor, process leases and interruption recovery | Broad integration tests exist, but the read-only and output contracts below need tightening. |

The historical Nomos result in `docs/current-status.md` is a recorded negative
experiment that correctly retained the baseline. It is not evidence that
optimization reliably improves arbitrary encoders. No real Nomos execution,
sealed evaluation, external provider call, GPU work, or model download was performed
in this assessment.

## Stabilization findings

### 1. Give execution failures an unambiguous CLI result

A disposable two-label project and a loopback server returning HTTP 503
reproduced a failed `synth generate` job with zero accepted rows, one failed
request, a persisted error message, and process exit code **0**. The result
correctly says `state: "failed"`, but a shell or caller checking only the exit
code receives success. This conflicts with the nonzero-failure promise in
`docs/operations.md`.

`commands/generation.rs::run_job` prints the terminal job without translating
its failed state into a command failure. Training and evaluation runners also
represent some backend failures as successfully returned failed-run objects;
their CLI boundaries need the same contract audit. Only generation was
dynamically reproduced here.

First repair: define execution-command exit behavior separately from successful
status/inspection of a historically failed run. Preserve the structured result
and artifact identity, and add process tests proving failed execution cannot be
mistaken for successful completion. Check callers before changing this contract.

### 2. Make read-only CLI operations actually read-only

`apps/generation-cli/src/main.rs` opens `SqliteStore` and calls
`detect_interrupted_workflows` before dispatching ordinary commands, including
configuration validation and previews. `SqliteStore::connect` creates missing
databases, enables WAL, and applies migrations.

The compiled CLI reproduced this directly: validating the unchanged example
TOML created a 2,437,120-byte database. The same validation failed with an
unable-to-open-database error when given a missing database directory.
A command that only validates a TOML file therefore still depends on a
writable database. Inspection can also perform startup recovery, which writes
failure/recovery facts for dead owners. That recovery behavior is documented,
but it conflicts with a strict read-only preview/inspection contract and makes
these commands unsuitable as passive readers without an explicit policy.

The preparation preview test verifies that no preparation record was created;
it does not prove that the database, migrations, leases, or unrelated run states
were untouched. The production optimize dispatcher also connects its writable
store before selecting preview/status/report commands.

First repair: dispatch pure configuration operations before database startup,
then provide an explicit read-only loading policy for previews and passive
inspection. Keep execution-time recovery on the paths that need it. Add process
tests against missing/read-only databases and interrupted-run fixtures.

### 3. Keep diagnostic logging out of JSON stdout

The CLI promises exactly one JSON value on stdout. Its tracing subscriber in
`apps/generation-cli/src/main.rs` does not select stderr, although it accepts
`RUST_LOG`. Enabling dependency diagnostics can therefore mix log records into
the command result. Progress messages already use stderr, so they do not cover
this separate tracing path. A compiled `dataset list` probe returned valid JSON
at ordinary log levels, but with `RUST_LOG=sqlx::query=debug` it exited 0 with
2,266 bytes of mixed diagnostics/result on stdout and an empty stderr. Parsing
stdout as a single JSON value failed.

First repair: direct the CLI tracing writer to stderr and test a JSON command
with SQL debug logging enabled. This preserves the current JSON payload shapes
for scripts and future GUI consumers.

### 4. Repair the fresh-checkout Windows verification gate

`npm run check` in `adapters/research-agent-pi` fails its first step with 14
formatting errors. `git ls-files --eol` shows committed LF files checked out as
CRLF under the current `core.autocrlf=true`; there is no tracked attributes
rule enforcing the formatter's expected line endings. No formatter writes were
applied during assessment.

Run separately, sidecar lint, type checking, build, and all 11 tests pass. This
is a reproducible checkout/formatting mismatch rather than a demonstrated
runtime defect. Fix it with repository-owned line-ending policy scoped to the
affected package, and verify from another clean checkout.

### 5. Add end-to-end coverage for the production optimization parent

The lower-level experiment runner tests cover development rejection, explicit
sealed success, and reserved-identity recovery. Campaign-core and
`encoder-experiment-sqlite/tests/repair_evidence.rs` also exercise optimization
definition persistence and journal replay. These are valuable existing tests.

However, `apps/generation-cli/tests` has no ordinary process journey for
`encoder optimize start/resume/authorize-sealed/cancel/report`. Its dispatcher
constructs a concrete `NomosBackend` even for status/report operations. The
local unit tests in `encoder_optimize.rs` cover strict manifest parsing and
stable candidate IDs, rather than the complete parent lifecycle.

The recorded productized real run adopted an existing completed experiment.
A new non-adopted run therefore deserves a deterministic full-parent test,
including crashes between child creation and parent journal append, repeat
commands, cancellation and both terminal decisions. Prefer a narrow
application seam around existing task/store ports; do not introduce a second
workflow implementation or make ordinary tests depend on a Nomos checkout.

### 6. Reduce change risk in the largest feature modules

The manifest review found no production core dependency on SQLx, Axum,
provider HTTP clients, Candle, CLI parsing, or the Nomos adapter. Generation's
application runner does depend on Tokio for retry timing; its domain objects
remain separate. The existing inward dependency direction is worth preserving.

Large modules are a maintenance risk, not automatically a correctness bug:
generation SQLite quality and provenance each exceed 5,000 nonblank lines;
the Nomos adapter root exceeds 3,900. Extract focused units only while fixing
the corresponding feature, retaining public ports and immutable fingerprints.
Avoid a broad refactor across all slices.

`docs/current-status.md` still describes an untracked/deferred GUI, and
`docs/encoder-optimize.md` points to a `goal.md` that now concerns the GUI.
Those handoff statements are stale at the assessed commit. Coordinate any
shared documentation updates with the GUI work instead of editing its goal.

## Verification

- `cargo fmt-check`: passed.
- `cargo check-all`: passed with all workspace targets and features.
- `cargo lint`: passed with warnings denied.
- `cargo test-all`: passed across 106 test binaries; 507 passed, zero failed,
  four explicitly opt-in tests ignored (two live-provider smokes, a public BERT
  bundle smoke, and real isolated Nomos integrity verification).
- Pi dependencies installed from the local lockfile/cache without network.
- Pi `npm run check`: failed at formatting, with the 14 CRLF/LF differences
  described above.
- Pi lint, typecheck, build and tests run separately: passed; 11 tests passed.
- Disposable CLI process probes reproduced writable validation, invalid JSON
  under debug logging, and a failed generation job exiting 0. The generation
  probe made exactly one loopback request, using only a synthetic credential.
- Rust 1.98.0, Node 25.6.0, Windows. The declared Rust 1.85 minimum was not
  validated with that historical toolchain.

The Rust test log and disposable CLI reproduction artifacts are under the
worktree's ignored `target/` directory. Only this assessment is intended for
the assessment commit. Each subsequent fix should include the relevant
regression test, all required repository gates, and its own coherent commit.

## Recommended implementation order

1. Repair execution exit behavior, logging and pure/read-only CLI startup
   contracts, one coherent change at a time.
2. Make the Windows sidecar verification gate reproducible.
3. Exercise the complete production optimization parent with deterministic
   persistence and task adapters, then fix observed lifecycle failures.
4. Continue feature-focused durability, migration, artifact-integrity and
   failure-path audits, extracting large modules only where the work needs it.

This workstream owns backend cores, persistence, adapters, tests and CLI
contracts. GUI surfaces remain in the parallel session. Compatible bug fixes
should retain CLI payloads; any necessary contract change should be documented
before the GUI starts consuming it.
