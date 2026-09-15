# Encoder Gym

Encoder Gym is a local, CLI-first Rust platform for generating, managing,
training, evaluating, analyzing, and improving text-classification datasets and
encoders. Slices 1–6 are implemented through explicit immutable artifact and
backend contracts.

See [`docs/platform-spec.md`](docs/platform-spec.md) for the full platform and
[`docs/current-status.md`](docs/current-status.md) for the current verified
handoff state. The platform specification links each slice's completion
criteria.

## Prerequisites

- Git
- The stable Rust toolchain installed through rustup
- The `rustfmt` and `clippy` components (requested by
  `rust-toolchain.toml`)

The fake backend is deterministic and requires no credentials. It is the best
way to verify a local installation.

## Encoder Gym desktop

The separately requested desktop phase provides persistent local project
folders and read-only baseline/candidate comparisons. From the repository root,
run `cd ui`, `npm ci`, then `npm start` (Node.js 22.19+). Start by adding a
folder, or explicitly open the historical example. No training or evaluation
starts when a folder is added.

See [`ui/README.md`](ui/README.md) for page responsibilities, supported journal
formats, recovery, and tests. Folder organization and presentation are generic;
the existing experiment execution CLI remains adapter-specific. Other platform
slice records are not yet exposed through this desktop reader.

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

Optional reusable label and dimension definitions are managed through the
versioned semantic catalog. Attachments are explicit, generation jobs pin exact
profile versions, and no second database or service is required. See
[`docs/semantic-catalog.md`](docs/semantic-catalog.md).

Authenticity research is an optional, explicitly started Pi-agent workflow. It
studies permitted sources, produces an evidence-linked profile for human
approval, and hands only abstract guidance to generation. The complete offline
and real-provider procedures are in
[`docs/authenticity-research.md`](docs/authenticity-research.md).

Dataset architecture is a separate optional Pi-agent workflow that recommends
explicit per-cell budgets and scoped generation strategies from pinned dataset,
semantic, authenticity, coverage, and eligible development-diagnostic facts.
Nothing is applied until an operator approves it; stale coverage blocks the
handoff, and generation consumes only the resulting immutable plan/context.
See [`docs/dataset-architect-spec.md`](docs/dataset-architect-spec.md).

Benchmark architecture is a separate row-free Pi workflow for deciding what
evaluation evidence the encoder must face before any benchmark is assembled.
It combines deployment risks with bounded research, validates support,
threshold, disclosure, and renewal requirements deterministically, then
requires human approval before producing a non-authorizing acquisition
handoff. The complete contract and offline example are in
[`docs/benchmark-architect-spec.md`](docs/benchmark-architect-spec.md) and
[`examples/benchmark-architect/`](examples/benchmark-architect/).

Before training, the optional dataset-quality gate can audit every accepted
generated or imported row. Its evaluator is blind to the assigned targets and
only supplies bounded evidence; deterministic policy and append-only operator
reviews decide which rows enter an immutable curation manifest:

```text
cargo run -p synthetic-data-cli -- quality policy-preview --preset balanced
cargo run -p synthetic-data-cli -- quality audit-create <DATASET_ID> --preset balanced --evaluator fake
cargo run -p synthetic-data-cli -- quality audit-start <RUN_ID>
cargo run -p synthetic-data-cli -- quality summary <RUN_ID>
cargo run -p synthetic-data-cli -- quality curate <RUN_ID>
cargo run -p synthetic-data-cli -- quality row-review --report-id <REPORT_ID> --source-row-id <ROW_ID> --include --reviewer operator --reason "manual evidence checked"
cargo run -p synthetic-data-cli -- quality manifest-review <PROPOSAL_ID> --approve --reviewer operator --reason "reviewed for training"
cargo run -p synthetic-data-cli -- snapshot create <DATASET_ID> --name qualified --quality-manifest <MANIFEST_ID>
```

The qualified snapshot retains the complete audit, guidance, review, and source
provenance chain. The deterministic fake requires no provider, while the
replaceable OpenAI-compatible evaluator is isolated behind the same internal
contract. See [`docs/dataset-quality-spec.md`](docs/dataset-quality-spec.md).
The old `row-review <ASSESSMENT_ID>` form remains supported. The report/row form
is required for invalid or unaudited rows that have no assessment. Audit,
curation, and approval commands return bounded ID/fingerprint/count summaries;
use `quality proposal <ID>` or `quality manifest <ID>` for the fully verified
row-level artifact. `quality summary` reports both accepted/assessed coverage
and the remaining population that is not yet qualified.

To opt into external candidate-text egress, first persist the same
OpenAI-compatible base URL and model used by generation, then create and start
the audit explicitly:

```powershell
cargo run -p synthetic-data-cli -- backend configure --base-url https://provider.example/v1 --model provider-model
cargo run -p synthetic-data-cli -- quality audit-create <DATASET_ID> --preset fast --egress external-candidate-text --evaluator openai-compatible
$env:SYNTH_OPENAI_API_KEY = "..."
cargo run -p synthetic-data-cli -- quality audit-start <RUN_ID> --api-key-env SYNTH_OPENAI_API_KEY
```

The key remains process-local. Non-loopback evaluator endpoints require HTTPS,
and start/recovery refuse to run if the current saved endpoint or model no
longer reconstructs the exact evaluator identities pinned at audit creation.
External audits retain finite row, request, attempt, and token ceilings. An
explicit monetary ceiling is currently rejected because provider pricing is
not yet an immutable persisted audit input.

During generation, the optional quality supervisor can stop weak-but-valid or
drifting segments before they fill the plan. Deterministic policy remains the
authority; bounded Pi may only propose a protected guidance revision, and a
reviewed revision activates only after an independently assessed canary. Both
the generator and blind evaluator are replaceable (`fake` or
OpenAI-compatible) and separately fingerprinted. See
[`docs/generation-quality-supervisor.md`](docs/generation-quality-supervisor.md)
and [`examples/supervisor/`](examples/supervisor/).

The supervisor can also own the generation stages of the finite encoder
workflow. Copy
[`examples/project-preparation-supervised.toml`](examples/project-preparation-supervised.toml),
replace its benchmark snapshot placeholder, and use `project preview` then
`project prepare`. Its outcome controls compile into immutable execution
authority; the workflow links the exact supervisor run, pauses with the exact
next CLI command, and enters the ordinary explicit curation, qualified
snapshot, training-firewall, training, and evaluation path. The legacy
`quality_gate` and `generation_supervision` modes are mutually exclusive.

The fastest complete offline journey starts from the checked-in local benchmark
files and requires no copied UUIDs:

```text
cargo run -p synthetic-data-cli -- project bootstrap-preview examples/pilot-support/project-bootstrap.toml
cargo run -p synthetic-data-cli -- project bootstrap examples/pilot-support/project-bootstrap.toml
cargo run -p synthetic-data-cli -- workflow start <DEFINITION_ID>
cargo run -p synthetic-data-cli -- workflow approve <RUN_ID>
cargo run -p synthetic-data-cli -- workflow finalize <RUN_ID>
cargo run -p synthetic-data-cli -- workflow promote <RUN_ID>
cargo run -p synthetic-data-cli -- doctor
```

Bootstrap imports the declared development JSONL and sealed CSV as ordinary
immutable Dataset Management artifacts, runs all preparation/governance checks,
and prints the exact workflow-start command. Preview writes nothing; unchanged
source bytes replay the original bootstrap. Read
[`docs/pilot-quickstart.md`](docs/pilot-quickstart.md).

A minimal fake-backend workflow is:

```text
cargo run -p synthetic-data-cli -- dataset create --name support --task "Classify support messages" --label billing --label fraud --dimension difficulty=easy,hard --dimension style=clean,messy
cargo run -p synthetic-data-cli -- dataset list
cargo run -p synthetic-data-cli -- plan describe <DATASET_ID>
cargo run -p synthetic-data-cli -- allocation preview <DATASET_ID> --total-rows 20000 --reserved-rows 2000 --explain
cargo run -p synthetic-data-cli -- allocation create <DATASET_ID> --total-rows 20000 --reserved-rows 2000
cargo run -p synthetic-data-cli -- generate <PLAN_ID> --backend fake
cargo run -p synthetic-data-cli -- coverage <PLAN_ID>
cargo run -p synthetic-data-cli -- rows --dataset-id <DATASET_ID>
cargo run -p synthetic-data-cli -- export <DATASET_ID> --format jsonl --file exports/support.jsonl
```

## Prepare a complete project from one manifest

For an advanced setup that already owns immutable benchmark snapshots, copy
[`examples/project-preparation.toml`](examples/project-preparation.toml), replace
its snapshot placeholder, and preview the complete finite project without
writing any artifacts:

```text
cargo run -p synthetic-data-cli -- project preview project-preparation.toml
cargo run -p synthetic-data-cli -- project prepare project-preparation.toml
cargo run -p synthetic-data-cli -- project list
cargo run -p synthetic-data-cli -- project show <PREPARATION_ID>
cargo run -p synthetic-data-cli -- workflow start <DEFINITION_ID>
```

`project prepare` atomically creates the training dataset configuration,
cohorts, role decisions, leakage reports, benchmark suites, and finite workflow
definition. Repeating the same manifest returns the original preparation. It
does not start generation or training; `workflow start` remains a separate
authorization. See [`docs/project-preparation.md`](docs/project-preparation.md).

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
cargo run -p synthetic-data-cli -- provenance project-preparation <PREPARATION_ID>
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

Each new definition pins an immutable benchmark bundle. Workflow execution
fails closed if a pinned cohort role is no longer the current active decision.
The complete development loop requires adaptation-eligible `row_content`
disclosure; sealed evidence remains aggregate-only and non-adaptive. Legacy
pre-bundle workflows remain inspectable but cannot be resumed.

For a reviewed production-repair snapshot, `synth encoder optimize` provides
the thinner one-cycle operator path used by the real Nomos experiment. It
reserves all child identities up front, advances one durable stage per
`resume`, stops before sealed evidence, and finishes with either
`promote_candidate` or `retain_baseline`. See
[`docs/encoder-optimize.md`](docs/encoder-optimize.md) for the strict manifest,
command family, recovery rules, and verified negative outcome.

`plan create --targets <FILE>` accepts explicit per-cell targets when an equal
distribution is not appropriate. This is the underlying plan model used by
both the CLI and browser application.

`allocation preview|create` converts one exact accepted-row total into complete
absolute cell targets using balanced, weighted, minimum-then-weighted, or
explicit policy. Reserved rows remain outside the initial plan for later
evidence-driven iterations. `create` atomically persists the reproducible
allocation and the ordinary unequal generation plan it produced.
Concise selector constraints, policy semantics, distribution explanations, and
allocation provenance are documented in
[`docs/allocation.md`](docs/allocation.md).

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
