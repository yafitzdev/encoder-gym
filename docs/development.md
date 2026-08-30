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
  resume, cancellation, a deterministic HTTP backend failure, bounded retry
  exhaustion, append-only attempt lineage, and the no-accepted-rows invariant.
- `workflow_governance_e2e.rs` proves unsafe sealed-evidence disclosure and an
  overbroad preauthorization envelope are rejected before project or workflow
  artifacts are persisted.
- `project_preparation_cli.rs` covers strict preview, atomic preparation,
  preparation idempotency, and the handoff to an initialized workflow.

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
