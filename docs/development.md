# Development workflow

## Incremental sequence

For Slice 1, implementation and validation followed this sequence:

1. Domain models
2. Dimension engine
3. Generation planner
4. Fake generation backend
5. Parsing and validation pipeline
6. Deduplication
7. Persistence
8. OpenAI-compatible backend
9. Job runner
10. Coverage calculation
11. Complete CLI workflows
12. Minimal HTTP API
13. Minimal graphical UI
14. Export

After Slice 1, build Dataset Management, Encoder Training, Evaluation, Error
Analysis, and Optimization in that order. Slices 2–6 stop at complete CLI
workflows until the user explicitly starts a separate API/UI phase.

After the independent slices, Controlled Workflow and Evaluation Governance may
compose their existing application contracts. Build it in this order: product-
boundary specification, pure allocation/governance policies, persistence,
CLI-first orchestration, recovery/doctor, offline end-to-end verification, and
documentation. Do not extend the HTTP API or graphical UI during this phase.

Declarative project preparation follows the same boundary order: specification,
pure strict-manifest compilation and preview, atomic adapter persistence,
scriptable CLI, offline process test, doctor/provenance verification, and
documentation. It may compose constructors and ports, but it must not invoke
CLI commands internally or start the prepared workflow.

Pilot bootstrap follows preparation's boundary: strict local-source manifest,
streaming format-adapter validation, ordinary import/snapshot construction,
one atomic SQLite transaction with the existing preparation bundle, CLI process
verification, and a checked-in offline reference project. Source-content
fingerprints—not mutable paths or presentation state—own replay idempotency.

Authenticity research follows the same inward-first sequence: specification and
trust boundary, pure research artifacts and budgets, deterministic fake loop,
the narrow Pi process adapter, durable SQLite execution, replaceable search and
fetch adapters, explicit approval/binding, prompt handoff, then CLI acceptance.
The Pi package has a pinned lockfile and its formatting, type-check, unit-test,
and build commands run alongside the Rust gates. Ordinary tests never search
the web or call a paid model.

Dataset qualification follows the same boundary-first sequence: specification,
pure source-set planning and policy verdicts, deterministic fake evaluator,
durable runner and SQLite evidence, append-only curation review, qualified
snapshot handoff, workflow gating, provenance/doctor, offline CLI acceptance,
and only then an optional bounded audit coordinator. V1 includes only directly
assessed or explicitly overridden rows; sampling-based inference is a later
policy rather than a hidden shortcut.

Generation-quality supervision follows the same boundary-first sequence:
authoritative specification, immutable contract and observation shapes, pure
strategy allocation and scoped drift policy, deterministic fakes, protected
revision/canary contracts, bounded Pi tools, durable segment orchestration,
SQLite integrity/recovery/provenance, CLI acceptance, and documentation. The
supervisor composes ordinary generation and quality contracts. It never mutates
an active job, lets Pi decide quality, or exposes sealed evidence.
Completed supervision then uses an immutable qualification handoff and a
zero-I/O evidence replay into the ordinary quality curation path. Verification
must cover rejection before completion, exact directly qualified membership,
excluded weak/canary evidence, idempotent finalization, explicit approval,
snapshot membership, deep Doctor checks, and end-to-end snapshot provenance.

Governed workflow integration follows with a pure outcome-control compiler,
optional workflow authority and legal paths, exact supervisor child links,
pause/resume/cancellation/recovery, existing finalization and curation reuse,
then an offline workflow process test. Definitions that omit supervision must
remain behaviorally and fingerprint compatible.

Benchmark qualification follows the stewardship sequence: explicit statistical
policy, pure aggregate evidence, immutable persistence with deep recomputation,
CLI inspection, separate append-only approval, workflow/preparation gating,
provenance/Doctor, and offline process acceptance. The policy, approval,
preparation binding, migration guards, and offline acceptance path are now
implemented. Sealed population summaries must never disclose row content or
adaptive diagnostics.

Benchmark architecture follows qualification with the same inward-first trust
boundary: specification, immutable safe brief and aggregate inputs, pure
blueprint/freshness validation, deterministic fake runner, bounded Pi research
tools, SQLite lifecycle/evidence persistence, append-only review and acquisition
handoff, provenance/Doctor, then an offline CLI journey. The handoff must not
create benchmark authority or bypass import, contamination, qualification, or
approval.

Production encoder optimization follows the same pattern: task-neutral repair
and experiment contracts, adapter-owned native artifacts, append-only SQLite
authority, a thin CLI composition root, manual or finitely authorized continuation, explicit
sealed authorization, deterministic fake terminal paths, an opt-in real adapter
test, Doctor/provenance replay, and a bounded real experiment. A new experiment
must omit `[existing_experiment]`; that section is only for honestly adopting a
previously completed exact run.

The `synthetic-data-cli/test-fixtures` feature builds `synth-optimize-fixture`,
a test-only composition of the ordinary CLI parser, optimization handlers,
SQLite stores, and a deterministic implementation of `EncoderTaskBackend`.
The production `synth` executable has no runtime fake override. The standard
`cargo test-all` alias enables these process tests. To run them alone:

```text
cargo test -p synthetic-data-cli --features test-fixtures --test encoder_optimize_cli
```

The fixture uses the same persisted repair approval contracts as the SQLite
tests. SQLite triggers inject failures between durable child creation and parent
journal linking; retries must preserve child identities and backend call counts.
No Nomos checkout, Python, model download, GPU, provider key, or paid service is
required for this acceptance path.

The same process suite covers `encoder optimize drive`: automatic completion,
separate final-evaluation approval, one authorization event, immutable child
identities, failures between child persistence and parent linking, and recovery
without repeated backend work. A test-only stdin handshake holds preparation
while independent production CLI processes inspect the live worker, reject
duplicate `drive`/`resume`, and cancel before the next stage. It never opens the
real Nomos project or grants a provider budget.

The same feature also builds `synth-benchmark-fixture`, a test-only constructor
for shared-benchmark desktop acceptance. It requires a new fixture directory
and uses ordinary domain constructors, scientific journals and managed model
custody with a tiny deterministic BERT bundle. Electron then calls production
`synth workspace benchmark` commands for preview/adoption/results. No runtime
override, real training, provider call or protected-test execution is involved.
`npm run smoke` in `ui` builds the fixture and tests comparison, version changes,
original verdicts, model links, history and restart against those persisted facts.

## Component loop

For each component:

1. Define the smallest useful public behavior.
2. Write a failing unit or integration test.
3. Implement only enough behavior to pass it.
4. Run formatting, compilation, linting, and tests.
5. Review dependency direction and error boundaries.
6. Exercise the behavior through the CLI when it becomes user-facing.
7. Commit the coherent change.

## Standard commands

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

The Pi sidecar also runs `npm run check` from `adapters/research-agent-pi`.
Its tracked Git attributes enforce LF for text files on every platform, including
Windows with `core.autocrlf=true`. An existing checkout with CRLF files can run
`npm run format` once; fresh checkouts already have the formatter's line endings.

Run the CLI during development with:

```text
cargo run -p synthetic-data-cli -- --help
```

## Configuration and secrets

Reproducible project settings live in strict TOML; start with
`examples/project.toml` and read `docs/configuration.md`. Local process settings
begin with `.env.example`. `.env` and all `.env.*` files except the example are
ignored. Backend credentials come from the environment or the server's
process-local memory and are never written to TOML, SQLite, fingerprints, CLI
output, or logs.

Persist only non-secret backend settings.

## Database changes

SQLite schema changes use ordered migrations. A migration and its corresponding
persistence tests belong in the same coherent change. Tests use isolated
temporary databases and do not reuse the developer database under `data/`.

Never edit a migration that may already have been applied outside the current
unreleased working tree. Add a new ordered migration, preserve foreign-key
targets when rebuilding SQLite tables, and run `PRAGMA foreign_key_check` in an
upgrade test or with `synth doctor`. See `docs/migrations.md`.

## External API tests

Tests default to deterministic fake or local mock backends. Live-provider smoke
tests, if added, must be opt-in and clearly separated so ordinary verification
never consumes API credits.

Transformer tests generate a tiny deterministic BERT bundle locally through
the `training-transformer/test-fixtures` feature. Standard tests require no
network, model cache, Python runtime, GPU, or credentials. The ignored public
bundle smoke command is documented in `docs/transformer-training.md` and must
remain explicitly opt-in.

## CLI process tests

`apps/generation-cli/tests/cli_e2e.rs` launches the compiled binary for each
step against a temporary database. It is the authoritative no-network smoke
test for import, generation, snapshotting, training, evaluation, analysis,
paired comparison, normalized findings, evidence export, append-only review,
proposal/application, provenance, export, pagination, and diagnostics.
It also covers the decision-grade optimization protocol, explicit approval,
idempotent plan application, scenario persistence, campaign lineage, exact
workflow allocation, fake advising, V1/V2 iteration, paired stopping, explicit
sealed assessment, immutable promotion, recovery, workflow provenance, and the
final doctor audit.

The smaller acceptance scenarios make workflow failure semantics independently
diagnosable:

- `workflow_resilience_e2e.rs` covers interrupted-process recovery, repeatable
  resume of the exact reserved child identity, active-child cancellation,
  a deterministic HTTP backend failure, bounded retry exhaustion, append-only
  attempt/execution lineage, and the no-accepted-rows invariant.
- `workflow_governance_e2e.rs` proves unsafe sealed-evidence disclosure and an
  overbroad preauthorization envelope are rejected before project or workflow
  artifacts are persisted.
- `project_preparation_cli.rs` covers strict preview, atomic preparation,
  preparation idempotency, and the handoff to an initialized workflow.
- `managed_optimization_setup_cli.rs` covers the active-model/dataset-version/
  benchmark-version selection through actual CLI processes. It checks read-only
  preview, version-8 migration, exact retry history, stale baseline/parent,
  foreign or altered input, test-data exclusion, folder movement and activity
  redaction. It also covers the one-click authority's version-9 read-only
  preview/list, exact retry, separate provider limits, stale-provider rejection,
  immutable history and credential/row-free activity. Saving setup does not
  start training; launch authorization still performs no provider, training or
  evaluation work.
- `managed-optimization-launch.test.mjs` verifies the fixed Electron boundary:
  exact project-owned preview/history, current provider identity, strict finite
  defaults, retry identity, temporary-file cleanup, and rejection of paths,
  credentials, injected execution fields and altered launch scope.

The desktop setup acceptance in `ui/src/managed-smoke-checks.ts` uses actual
CLI-created dataset/benchmark custody and production selection IPC. It tests
artifact links, saved inputs, changed versions, a lost save response after
commit, exact retry, and restoration in a second Electron process. Existing run
supervision is tested through Runs; Optimize no longer opens the old recipe
preparation screen. The read-only current-project check verifies Nomos without
saving inputs or starting work. Run `npm run smoke` in `ui`; on Windows finish
backend-building commands before running suites that hold `synth.exe` open.

All ordinary acceptance tests use fake or loopback-only backends. An ignored
OpenAI-compatible smoke boundary exists for deliberate provider verification:

```powershell
$env:SYNTH_E2E_OPENAI_BASE_URL = "https://provider.example/v1"
$env:SYNTH_E2E_OPENAI_MODEL = "provider-model-name"
$env:SYNTH_OPENAI_API_KEY = "..."
cargo test -p synthetic-data-cli --test workflow_resilience_e2e live_openai_compatible_smoke_is_explicitly_opt_in -- --ignored --nocapture
```

That command may incur provider cost. Never run it as part of `cargo test-all`
or CI, and never persist its credential.

`apps/generation-cli/tests/transformer_cli_e2e.rs` independently exercises
registration, verification, bounded CPU training, checkpoint prediction,
explicit continuation, evaluation, provenance, and doctor with the tiny local
fixture.

Production encoder tests are layered because native Nomos payloads may not enter
ordinary fixtures. `encoder-experiment-runner/tests/offline.rs` covers both
development rejection without sealed use and the successful development plus
explicit sealed path with a deterministic fake adapter. Campaign-core tests
cover finite budgets, adoption, cancellation, and sealed-free retention;
SQLite tests cover migrations, idempotency, immutability, and tamper rejection.
The ignored `encoder-experiment-nomos/tests/isolated_local.rs` test verifies the
actual isolated checkout and large model artifacts when
`NOMOS_ENCODER_GYM_EXPERIMENT` is explicitly set.
