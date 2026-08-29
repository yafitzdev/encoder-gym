# Encoder Development Platform

A local, CLI-first Rust platform for generating, managing, training, evaluating,
analyzing, and improving text-classification datasets and encoders. Slices 1–6
are implemented through explicit immutable artifact and backend contracts.

See [`docs/platform-spec.md`](docs/platform-spec.md) for the full platform and
the linked per-slice completion criteria.

## Prerequisites

- Git
- The stable Rust toolchain installed through rustup
- The `rustfmt` and `clippy` components (requested by
  `rust-toolchain.toml`)

The fake backend is deterministic and requires no credentials. It is the best
way to verify a local installation.

## Use the CLI

Inspect all commands with:

```text
cargo run -p synthetic-data-cli -- --help
```

Human output is the default. Add `--output json` before the subcommand for one
machine-readable JSON value on stdout. File-producing commands use `--file`.

Start a reproducible project from the checked-in example:

```text
cargo run -p synthetic-data-cli -- config validate examples/project.toml
cargo run -p synthetic-data-cli -- --output json config show examples/project.toml
cargo run -p synthetic-data-cli -- config init examples/project.toml
cargo run -p synthetic-data-cli -- doctor --config examples/project.toml
```

See [`docs/configuration.md`](docs/configuration.md) and
[`docs/operations.md`](docs/operations.md). The command overview is in
[`docs/cli.md`](docs/cli.md), and immutable artifact lineage is described in
[`docs/provenance.md`](docs/provenance.md). The complete advisory optimization
workflow is in [`docs/optimization.md`](docs/optimization.md).

A minimal fake-backend workflow is:

```text
cargo run -p synthetic-data-cli -- dataset create --name support --task "Classify support messages" --label billing --label fraud --dimension difficulty=easy,hard --dimension style=clean,messy
cargo run -p synthetic-data-cli -- dataset list
cargo run -p synthetic-data-cli -- allocation preview <DATASET_ID> --total-rows 20000 --reserved-rows 2000
cargo run -p synthetic-data-cli -- allocation create <DATASET_ID> --total-rows 20000 --reserved-rows 2000
cargo run -p synthetic-data-cli -- generate <PLAN_ID> --backend fake
cargo run -p synthetic-data-cli -- coverage <PLAN_ID>
cargo run -p synthetic-data-cli -- rows --dataset-id <DATASET_ID>
cargo run -p synthetic-data-cli -- export <DATASET_ID> --format jsonl --file exports/support.jsonl
```

## Governed encoder workflow

The cross-slice workflow is also CLI-only. After creating immutable development
and sealed cohorts, clean contamination reports, benchmark suites, and a
resolved project configuration, define one finite workflow and run it:

```text
cargo run -p synthetic-data-cli -- workflow define --definition workflow.toml
cargo run -p synthetic-data-cli -- workflow start <DEFINITION_ID>
cargo run -p synthetic-data-cli -- workflow status <RUN_ID>
cargo run -p synthetic-data-cli -- workflow approve <RUN_ID> --recommendation-id <ID>
cargo run -p synthetic-data-cli -- workflow resume <RUN_ID>
cargo run -p synthetic-data-cli -- workflow finalize <RUN_ID>
cargo run -p synthetic-data-cli -- workflow promote <RUN_ID>
cargo run -p synthetic-data-cli -- provenance model-promotion <PROMOTION_ID>
```

`start` runs initial allocation, generation, immutable snapshot V1, fresh
training, development evaluation, deterministic acceptance, error analysis,
the optional advisor, and proposal creation. Review mode pauses before the
dataset diff; bounded preauthorization continues only when the exact action is
inside its persisted envelope. `finalize` is the only workflow command that
uses sealed evidence, and `promote` records immutable promotion or rejection—it
does not overwrite a mutable "best model" pointer. See
[`docs/workflow-governance-spec.md`](docs/workflow-governance-spec.md) and
[`docs/cli.md`](docs/cli.md).

`plan create --targets <FILE>` accepts explicit per-cell targets when an equal
distribution is not appropriate. This is the underlying plan model used by
both the CLI and browser application.

`allocation preview|create` converts one exact accepted-row total into complete
absolute cell targets using balanced, weighted, minimum-then-weighted, or
explicit policy. Reserved rows remain outside the initial plan for later
evidence-driven iterations. `create` atomically persists the reproducible
allocation and the ordinary unequal generation plan it produced.

Plan targets are absolute dataset coverage. If a cell already contains 20
accepted rows and a later plan targets 25, generation requests only the five-row
deficit.

## Dataset-to-optimization CLI workflow

Once accepted rows exist, create an immutable stratified snapshot:

```text
cargo run -p synthetic-data-cli -- snapshot create <DATASET_ID> --name baseline
cargo run -p synthetic-data-cli -- snapshot stats <SNAPSHOT_ID>
cargo run -p synthetic-data-cli -- snapshot export <SNAPSHOT_ID> --format jsonl --file exports/baseline.jsonl
```

Train the deterministic local hashing-linear baseline and inspect its durable
checkpoints:

```text
cargo run -p synthetic-data-cli -- training run <SNAPSHOT_ID> --epochs 20
cargo run -p synthetic-data-cli -- training checkpoints <TRAINING_RUN_ID>
cargo run -p synthetic-data-cli -- training predict <CHECKPOINT_ID> --text "Why was I charged twice?"
```

For real local encoder training, register an exact supported BERT bundle and
select the replaceable CPU adapter:

```text
cargo run -p synthetic-data-cli -- encoder register --name support-bert C:\models\support-bert
cargo run -p synthetic-data-cli -- training run <SNAPSHOT_ID> --backend bert-cpu --encoder-id <ENCODER_ID>
cargo run -p synthetic-data-cli -- training continue <CHECKPOINT_ID> --epochs 2
```

The workflow remains CLI-only. It uses bounded raw-text/tokenization batches,
durable progress, immutable checksum-verified checkpoints, and the existing
evaluation/predictor boundary. Read
[`docs/transformer-training.md`](docs/transformer-training.md) for the exact
bundle contract, configuration, cancellation, continuation, and opt-in public
model smoke test. The hashing-linear backend remains the fast deterministic
default for development and ordinary platform tests.

Evaluate a checkpoint, build an error report, and create a finite data proposal:

```text
cargo run -p synthetic-data-cli -- evaluation run <CHECKPOINT_ID> --split test
cargo run -p synthetic-data-cli -- evaluation predictions <EVALUATION_RUN_ID>
cargo run -p synthetic-data-cli -- analysis create <EVALUATION_RUN_ID> --minimum-support 1
cargo run -p synthetic-data-cli -- analysis findings <ANALYSIS_REPORT_ID> --kind cell
cargo run -p synthetic-data-cli -- analysis evidence <ANALYSIS_REPORT_ID> '<FINDING_KEY>' --format jsonl --file evidence.jsonl
cargo run -p synthetic-data-cli -- optimize preview <ANALYSIS_REPORT_ID> --protocol examples/optimization.toml
cargo run -p synthetic-data-cli -- optimize propose <ANALYSIS_REPORT_ID> --protocol examples/optimization.toml
cargo run -p synthetic-data-cli -- optimize show <PROPOSAL_ID>
cargo run -p synthetic-data-cli -- optimize recommendations <PROPOSAL_ID> --eligible
cargo run -p synthetic-data-cli -- optimize review <PROPOSAL_ID> --state approved-for-plan-creation
```

Applying a proposal only creates a normal, reviewable unequal generation plan.
It does not start generation, training, or an autonomous loop:

```text
cargo run -p synthetic-data-cli -- optimize apply <PROPOSAL_ID> --approval-id <REVIEW_ID>
cargo run -p synthetic-data-cli -- generate <NEW_PLAN_ID> --backend fake
```

Use `analysis export` and `optimize export` to write immutable JSON artifacts.
Use `optimize export-summary` and `optimize export-recommendations` for
automation-friendly JSONL/CSV rows. Optimization never starts generation or
training; `optimize apply` only creates an ordinary generation plan.
Analysis is bounded, confidence-aware, overlap-aware, and can consume a
persisted paired comparison through `--comparison-id`; it never reruns a model.
Finding reviews are separate append-only records. Read
[`docs/error-analysis.md`](docs/error-analysis.md) for interpretation and CLI
examples.

Historical proposals created with `--budget` remain usable only through the
explicit `optimize legacy-apply` compatibility command.
The new slices intentionally have no HTTP or graphical UI surface.

## Import external datasets

The JSONL example uses nested fields; a CSV rejection fixture is also included:

```text
cargo run -p synthetic-data-cli -- dataset import <DATASET_ID> \
  --input examples/datasets/support.jsonl --format jsonl --dry-run \
  --text-field message --label-field category \
  --dimension difficulty=metadata.difficulty \
  --dimension style=metadata.style
```

Imports stream in bounded batches, retain source path and row number, and
participate in snapshots and unified export beside generated rows.

## Recovery, diagnostics, and provenance

```text
cargo run -p synthetic-data-cli -- recovery list
cargo run -p synthetic-data-cli -- recovery resume-generation <JOB_ID> --config project.toml
cargo run -p synthetic-data-cli -- provenance optimization-proposal <PROPOSAL_ID>
cargo run -p synthetic-data-cli -- doctor --config project.toml
```

Read [`docs/recovery.md`](docs/recovery.md) for interruption semantics and
[`docs/migrations.md`](docs/migrations.md) for schema operations.

## OpenAI-compatible generation

Copy `.env.example` to `.env` and provide `SYNTH_OPENAI_API_KEY` when the
endpoint requires authentication. Configure the non-secret base URL and model
in SQLite before generation:

```text
cargo run -p synthetic-data-cli -- backend configure --base-url https://api.openai.com/v1 --model <MODEL>
cargo run -p synthetic-data-cli -- generate <PLAN_ID> --backend openai-compatible
```

The browser backend form accepts the key for the lifetime of the server process
only. API keys are never written to SQLite. Provider-specific request and
response types remain inside the adapter crate; the rest of the application
depends only on the project-owned `GenerationBackend` port.

## Local process configuration

Supported environment variables are documented in `.env.example`:

- `SYNTH_DATABASE_URL` — SQLite connection URL
- `SYNTH_BIND_ADDRESS` — local HTTP bind address
- `SYNTH_OPENAI_API_KEY` — optional bearer token
- `RUST_LOG` — server log filter

Never commit credentials. `.env` and its local variants are ignored.

The repository retains the earlier Slice 1 server/control surface for backward
compatibility. This enhancement phase adds no HTTP or graphical UI work;
Slices 2–6 remain CLI-only.

## Verification

Run all required checks from the repository root:

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

The ordinary test suite uses deterministic fakes and local mock HTTP servers;
it does not contact an external provider or consume API credits.

Read [`docs/architecture.md`](docs/architecture.md) before changing component
boundaries and [`docs/development.md`](docs/development.md) before adding a
coherent feature.
