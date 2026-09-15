<!-- README.md -->

<div align="center">

# Encoder Gym

### Local, reproducible encoder development—from data to decision.

**Generate, curate, train, evaluate, analyze, and improve text encoders while preserving the evidence behind every change.**

[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust)](https://www.rust-lang.org/)
[![Local first](https://img.shields.io/badge/execution-local--first-2563eb)](docs/platform-spec.md)
[![CLI + Desktop](https://img.shields.io/badge/interfaces-CLI%20%2B%20desktop-7c3aed)](ui/README.md)
[![Status: active development](https://img.shields.io/badge/status-active%20development-f59e0b)](docs/current-status.md)

[Start Here](#start-here) • [Why Encoder Gym?](#why-encoder-gym) • [The Encoder Loop](#the-encoder-loop) • [Quick Start](#quick-start) • [Governance](#governance--provenance) • [Architecture](#architecture) • [Limitations](#limitations) • [Documentation](#links) • [GitHub](https://github.com/yafitzdev/encoder-gym)

</div>

<br />

---

<a id="start-here"></a>

### Where to start 🚀

> [!IMPORTANT]
> The checked-in pilot exercises the complete local workflow with deterministic
> synthetic data and hashing-linear training. It needs no API key, network,
> model download, Python environment, or GPU.

```powershell
$env:SYNTH_DATABASE_URL = "sqlite://data/pilot-support.db?mode=rwc"

cargo run -p synthetic-data-cli -- project bootstrap-preview examples/pilot-support/project-bootstrap.toml
cargo run -p synthetic-data-cli -- project bootstrap examples/pilot-support/project-bootstrap.toml
```

Preview writes nothing. Bootstrap creates the immutable inputs and prints the
exact next command for starting the workflow. Continue with that command, then
use `workflow status` to see the next explicit decision.

See the [Pilot Quick Start](docs/pilot-quickstart.md) for the full offline run.

---

### About

Encoder Gym is a local, single-user platform for building and improving text
encoders. It connects synthetic-data generation, dataset management, training,
evaluation, error analysis, and optimization through explicit persisted
contracts.

The main product path is the Rust CLI, backed by SQLite and immutable files. A
local Electron desktop provides managed project folders, model and dataset
custody, experiment inspection, comparisons, settings, and activity history.
External providers are optional and are used only through explicitly configured,
bounded operations.

Encoder Gym is not a hosted AutoML service and it does not hide an open-ended
optimization loop behind a button. Training inputs, evaluation roles, budgets,
reviews, and promotion decisions remain visible and reproducible.

The standard workflow targets text classification. A provider-neutral experiment
contract also supports compiled task adapters for other encoder tasks; the first
real adapter is the isolated Nomos retrieval ranker.

---

### Why Encoder Gym?

**The complete encoder loop 🧪**
> Generate or import examples, curate immutable datasets, train checkpoints,
> evaluate them, inspect failures, and turn reviewed findings into the next
> bounded data plan.

**Immutable evidence instead of mutable experiment folders 🧾**
> Snapshots, checkpoints, evaluations, reports, proposals, reviews, and
> promotion decisions retain the identities and fingerprints of what produced
> them.

**Evaluation that stays separate from adaptation 🔒**
> Development evidence may guide iteration. Sealed acceptance evidence is
> handled through a separate explicit finalization step and is never supplied
> to analysis, generation, or optimization.

**A real offline path 🏠**
> Deterministic generation, hashing-linear training, local SQLite persistence,
> and mock provider tests make the ordinary development workflow reproducible
> without external services.

**Human authority at consequential boundaries 🧭**
> Agents may research, diagnose, or propose within finite scopes. Deterministic
> contracts and explicit reviews remain the authority for qualification,
> iteration, sealed evaluation, and promotion.

**Replaceable backends with project-owned contracts 🔌**
> Generation providers, evaluators, advisors, trainers, and production task
> adapters remain behind narrow interfaces. Provider-specific types do not
> define the platform's domain model.

**Recovery is part of the workflow ♻️**
> Long-running work reserves durable identities before execution. Status,
> cancellation, recovery, idempotent replay, and provenance are ordinary product
> operations rather than cleanup scripts.

---

### What You Can Do

| Stage | What Encoder Gym does | Durable output |
|-------|-----------------------|----------------|
| **Generate or import data** | Builds explicit coverage plans, generates bounded synthetic examples, or imports JSONL/CSV rows with source provenance. | Dataset rows and generation receipts |
| **Curate and version** | Audits row quality, records reviews, and freezes exact train/validation/test membership. | Immutable dataset snapshot |
| **Train** | Runs deterministic hashing-linear training or a supported local BERT-family CPU backend. | Training run and checksum-verified checkpoints |
| **Evaluate** | Applies a persisted benchmark contract to an exact checkpoint and immutable cohort. | Predictions, metrics, and acceptance evidence |
| **Analyze** | Aggregates persisted errors by label and arbitrary categorical dimensions without rerunning the model. | Error report and inspectable findings |
| **Improve** | Converts eligible development findings into finite, reviewable data recommendations and generation plans. | Optimization proposal, review, and successor plan |

Optional bounded capabilities add authenticity research, dataset architecture,
row qualification, generation supervision, benchmark architecture, declarative
project preparation, and production-task adapters around the same core artifact
contracts.

---

### The Encoder Loop

```text
Task + benchmark contract
          │
          ▼
Generate or import ──→ qualify ──→ immutable snapshot
                                           │
                                           ▼
                                     train checkpoint
                                           │
                                           ▼
                                development evaluation
                                           │
                                           ▼
                        error analysis ──→ reviewed proposal
                              ▲                    │
                              └── next bounded iteration

After development stops:

separate sealed finalization ──→ promote candidate or retain baseline
```

Every arrow crosses a persisted contract. A later stage links the earlier
artifact instead of reaching into its internal state or replacing it in place.

---

### What Exists Today

> [!NOTE]
> Encoder Gym is at version `0.1.0` and is under active development. The core
> local workflows are implemented and tested; interface coverage is intentionally
> uneven while the desktop catches up with the CLI.

| Area | Current state |
|------|---------------|
| Core platform | Synthetic data, datasets, training, evaluation, analysis, and optimization have Rust domain, persistence, CLI, and deterministic tests. |
| Controlled workflow | A finite, persisted CLI workflow composes the slices with explicit approvals, budgets, contamination checks, sealed finalization, and promotion. |
| Training | Deterministic hashing-linear and supported local BERT-family CPU backends. |
| External providers | Optional OpenAI-compatible generation, row-quality evaluation, and advisory adapters with explicit configuration and finite limits. |
| Desktop | Managed local projects, model/data custody, comparisons, settings, activity history, and recorded experiment inspection. It does not expose every CLI capability. |
| Production tasks | Provider-neutral experiment contracts with a compiled Nomos retrieval-ranking adapter as the first real integration. |
| Deployment | Local, single-user execution. No hosted service, authentication, distributed execution, or multi-worker runtime. |

The detailed implementation and real experiment handoff are tracked in
[Current Project Status](docs/current-status.md).

---

<a id="quick-start"></a>

<details>

<summary><strong>📦 Quick Start</strong></summary>

<br />

#### Prerequisites

- Git
- Rust `1.85` or newer through `rustup`
- The `rustfmt` and `clippy` components requested by `rust-toolchain.toml`
- Node.js `22.19` or newer and npm only when running the desktop app

The repository pins the stable Rust toolchain profile and required components.

#### Inspect the CLI

```text
cargo run -p synthetic-data-cli -- --help
```

The executable is currently named `synth`. Human-readable output is the
default; add `--output json` before a subcommand for one machine-readable JSON
value on stdout.

#### Run the complete offline pilot

```powershell
$env:SYNTH_DATABASE_URL = "sqlite://data/pilot-support.db?mode=rwc"

cargo run -p synthetic-data-cli -- project bootstrap-preview examples/pilot-support/project-bootstrap.toml
cargo run -p synthetic-data-cli -- --output json project bootstrap examples/pilot-support/project-bootstrap.toml
```

Run the exact workflow command printed by bootstrap:

```text
cargo run -p synthetic-data-cli -- workflow start <DEFINITION_ID>
cargo run -p synthetic-data-cli -- workflow status <RUN_ID>
```

If the workflow pauses for review, inspect its status and approve only that
bounded iteration:

```text
cargo run -p synthetic-data-cli -- workflow approve <RUN_ID>
```

Development completion, sealed evaluation, and promotion remain separate:

```text
cargo run -p synthetic-data-cli -- workflow finalize <RUN_ID>
cargo run -p synthetic-data-cli -- workflow promote <RUN_ID>
cargo run -p synthetic-data-cli -- provenance workflow-run <RUN_ID>
cargo run -p synthetic-data-cli -- doctor
```

See [Pilot Quick Start](docs/pilot-quickstart.md) for expected outputs,
idempotent replay, and the opt-in real-provider smoke.

#### Launch Encoder Gym desktop

```powershell
cd ui
npm ci
npm start
```

The desktop starts with an empty project library. Creating or opening a project
does not start training, evaluation, or a provider call. See the
[Desktop Guide](ui/README.md).

</details>

---

<a id="core-concepts"></a>

<details>

<summary><strong>📦 Core Concepts and Artifacts</strong></summary>

<br />

| Concept | Meaning |
|---------|---------|
| **Project configuration** | A resolved local task definition with explicit labels, dimensions, backends, budgets, and workflow settings. |
| **Dataset** | An append-only identity containing accepted, rejected, generated, and imported row facts. |
| **Snapshot** | Immutable, reproducibly split dataset membership used by later stages. |
| **Benchmark bundle** | Exact development and optional sealed suites plus their qualification and contamination authority. |
| **Training run** | Persisted lifecycle, backend configuration, progress, and input binding for one training attempt. |
| **Checkpoint** | Immutable model artifact tied to its training run and source snapshot. |
| **Evaluation** | Metrics and predictions produced by one checkpoint against one exact suite. |
| **Analysis report** | Persisted findings derived from evaluation facts, never from presentation state. |
| **Optimization proposal** | A finite set of explicit data recommendations awaiting review. |
| **Promotion decision** | Immutable acceptance or rejection of a candidate; not a mutable `best-model` pointer. |

Artifacts retain enough identity and configuration to inspect how later results
were derived. See [Provenance](docs/provenance.md).

</details>

---

<a id="governance--provenance"></a>

<details>

<summary><strong>📦 Governance and Provenance</strong> → <a href="docs/workflow-governance-spec.md">Full Workflow Contract</a></summary>

<br />

Encoder Gym treats experiment governance as executable product behavior:

| Boundary | Enforced behavior |
|----------|-------------------|
| Dataset history | Completed snapshots and derived artifacts are immutable. |
| Benchmark leakage | Trainer-visible data must pass a strict persisted contamination check against the bound benchmark population. |
| Evidence roles | Development evidence may support adaptation; sealed evidence is aggregate-only and non-adaptive. |
| Agent authority | Bounded agents may inspect or propose; deterministic validation and explicit reviews decide what becomes active. |
| External calls | Provider operations require configured authority and persisted finite request, attempt, token, and optional cost limits. |
| Long-running work | Child identities, attempts, cancellation, and recovery are persisted before or alongside execution. |
| Acceptance | Deterministic benchmark contracts decide pass, fail, inconclusive, or invalid. |
| Promotion | Finalization and promotion are explicit operations with immutable records. |

Provenance follows source snapshots, plans, model artifacts, evaluations,
reports, reviews, and decisions. Use the `provenance` command family to inspect
the complete dependency tree for a supported artifact.

See [Provenance](docs/provenance.md), [Benchmark Stewardship](docs/benchmark-stewardship-spec.md),
and [Controlled Workflow Governance](docs/workflow-governance-spec.md).

</details>

---

<a id="desktop"></a>

<details>

<summary><strong>📦 Encoder Gym Desktop</strong> → <a href="ui/README.md">Desktop Guide</a></summary>

<br />

The Electron desktop is a local multi-project workspace. It can create and open
Gym-owned folders, copy supported checkpoints into managed custody, import and
version datasets, inspect models and recorded runs, compare compatible benchmark
results, manage provider connections, verify project files, and export a
hash-chained activity journal.

The renderer receives typed, bounded data through the desktop bridge. It cannot
read arbitrary paths, provider credentials, sealed scores, sealed rows, or raw
native diagnostics. Adding a folder or saving configuration never authorizes
training or provider spending.

The desktop is not yet a graphical surface for every independent platform
slice. Generic presentation and managed custody are broader than execution:
production task execution still requires a compatible compiled adapter, and the
existing experiment CLI is currently wired to Nomos.

```powershell
cd ui
npm ci
npm start
```

</details>

---

<a id="architecture"></a>

<details>

<summary><strong>📦 Architecture</strong> → <a href="docs/architecture.md">Full Architecture Guide</a></summary>

<br />

```text
Electron desktop        synth CLI        local generation server
        └──────────────────┼──────────────────┘
                           ▼
              application composition and runners
                           ▼
        project preparation + finite workflow contracts
                           ▼
 generation → dataset → training → evaluation → analysis → optimization
                           ▼
        SQLite persistence + immutable files + backend adapters
```

Core crates own domain objects and replaceability contracts. SQLite, HTTP,
provider SDKs, process execution, CLI parsing, and presentation remain in
adapters or applications. A later stage consumes immutable artifacts from the
previous stage instead of importing its infrastructure types.

The original local HTTP server remains a generation control surface; the newer
dataset-through-optimization capabilities are CLI-first.

</details>

---

<a id="cli-reference"></a>

<details>

<summary><strong>📦 CLI Reference</strong> → <a href="docs/cli.md">Full CLI Guide</a></summary>

<br />

| Command family | Purpose |
|----------------|---------|
| `project`, `config` | Preview, resolve, and atomically prepare a finite encoder project. |
| `dataset`, `snapshot`, `quality` | Create, import, audit, review, version, and inspect data. |
| `plan`, `allocation`, `generate`, `job` | Define coverage and execute bounded generation. |
| `encoder`, `training` | Register supported bundles and run or continue training. |
| `cohort`, `benchmark`, `evaluation` | Define evidence roles and evaluate exact checkpoints. |
| `analysis`, `optimize`, `advisor` | Diagnose persisted errors and prepare reviewable improvements. |
| `workflow` | Execute the finite governed cross-slice workflow. |
| `experiment`, `encoder optimize`, `production-campaign` | Run bounded adapter-backed production encoder work. |
| `workspace` | Manage desktop-owned projects, custody, providers, benchmarks, and optimization inputs. |
| `recovery`, `provenance`, `doctor` | Recover interrupted work and verify stored facts. |

Inspect the live command tree rather than relying on copied examples:

```text
cargo run -p synthetic-data-cli -- --help
cargo run -p synthetic-data-cli -- workflow --help
cargo run -p synthetic-data-cli -- workspace --help
```

Human output is the default. Place `--output json` before the subcommand for one
machine-readable JSON value on stdout; diagnostics remain on stderr.

</details>

---

<a id="backends--configuration"></a>

<details>

<summary><strong>📦 Backends and Configuration</strong></summary>

<br />

| Role | Implementations currently available |
|------|-------------------------------------|
| Synthetic generation | Deterministic fake; OpenAI-compatible endpoint |
| Dataset-quality evaluation | Deterministic fake; OpenAI-compatible endpoint |
| Advisory interpretation | Deterministic fake; OpenAI-compatible endpoint |
| Text-classification training | Hashing-linear; supported local BERT-family CPU bundle |
| Production encoder tasks | Compiled task adapters; Nomos retrieval ranking is the first integration |

Copy `.env.example` to `.env` for local development. Supported process settings
include:

- `SYNTH_DATABASE_URL` — SQLite connection URL
- `SYNTH_BIND_ADDRESS` — local generation-server bind address
- `SYNTH_OPENAI_API_KEY` — optional generation/evaluation credential
- `SYNTH_ADVISOR_API_KEY` — optional advisor credential
- `BRAVE_SEARCH_API_KEY` — optional bounded research credential
- `RUST_LOG` — diagnostic log filter

Non-secret provider settings are persisted explicitly. CLI credentials remain
process-local; desktop credentials are stored through its operating-system-backed
encrypted credential store and are never returned to the renderer.

See [Configuration](docs/configuration.md), [Operations](docs/operations.md), and
[Transformer Training](docs/transformer-training.md).

</details>

---

<a id="recovery--reproducibility"></a>

<details>

<summary><strong>📦 Recovery and Reproducibility</strong></summary>

<br />

Generation jobs, training runs, workflows, experiments, and managed optimization
runs persist state rather than relying on a live terminal or UI session.
Interrupted work is recovered through its reserved identity; retries reuse
completed artifacts when their fingerprints still match.

```text
cargo run -p synthetic-data-cli -- recovery list
cargo run -p synthetic-data-cli -- recovery resume-generation <JOB_ID> --config project.toml
cargo run -p synthetic-data-cli -- provenance workflow-run <RUN_ID>
cargo run -p synthetic-data-cli -- doctor
```

`doctor` derives its verdict from stored facts and artifact bytes. It does not
trust a previous green UI state. See [Recovery](docs/recovery.md),
[Provenance](docs/provenance.md), and [Database Migrations](docs/migrations.md).

</details>

---

<a id="limitations"></a>

<details>

<summary><strong>📦 Limitations</strong></summary>

<br />

| Boundary | Current behavior |
|----------|------------------|
| Product scope | Local, single-user operation; no authentication, hosted control plane, multi-user collaboration, or distributed workers. |
| Standard task | The complete generic slice workflow currently targets text classification. Other encoder tasks require compiled adapters. |
| Desktop coverage | The desktop manages projects and evidence but does not expose every CLI workflow or every production adapter. |
| Trainer support | Hashing-linear and supported BERT-family CPU bundles; arbitrary model code and pickle-based weights are not accepted. |
| Contamination detection | Exact source, exact text, normalized text, and declared group checks—not semantic or embedding deduplication. |
| External provenance | The platform cannot prove that benchmark content was absent from a pretrained base model or training performed elsewhere. |
| Automation | Workflows are finite, persisted, and bounded. There is no open-ended autonomous optimization loop. |
| Compatibility | The repository is at `0.1.0`; public package and long-term compatibility commitments have not been declared. |

The complete scope and non-goals are in the
[Platform Specification](docs/platform-spec.md).

</details>

---

<a id="faq--troubleshooting"></a>

<details>

<summary><strong>📦 FAQ / Troubleshooting</strong></summary>

<br />

**Do I need an API key?**
> No. The offline pilot, deterministic fake backends, hashing-linear trainer,
> and ordinary test suite require no credentials. Keys are needed only for an
> explicitly selected external provider.

**Do I need a GPU?**
> No for the included pilot. The supported transformer path is a local CPU
> backend, though real encoder training can take materially longer than the
> deterministic baseline.

**Does opening a desktop project start training?**
> No. Project creation, imports, configuration, inspection, and verification do
> not authorize model execution or provider calls.

**Can sealed evaluation data influence optimization?**
> No. Sealed rows, predictions, and scores are excluded from adaptive analysis
> and agent inputs. Sealed finalization is a separate explicit operation.

**Can Encoder Gym use an OpenAI-compatible endpoint?**
> Yes, for explicitly configured generation, quality evaluation, or advisory
> roles. Each external operation remains subject to its persisted authority and
> finite limits.

**Can it train any encoder architecture?**
> Not automatically. The generic trainer supports the documented local
> BERT-family bundle contract. Different task semantics or model layouts need a
> compatible backend or compiled production-task adapter.

**Where is local state stored?**
> Core CLI workflows use the configured SQLite URL plus immutable artifact
> files. Desktop projects own their model, dataset, registry, activity, and run
> directories inside the selected project folder.

For detailed command failures, start with `synth doctor`, then see
[Operations](docs/operations.md) and [Recovery](docs/recovery.md).

</details>

---

<a id="development"></a>

<details>

<summary><strong>📦 Development and Verification</strong> → <a href="docs/development.md">Development Guide</a></summary>

<br />

Run the Rust gates from the repository root:

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

Run the desktop checks separately:

```powershell
cd ui
npm ci
npm run check
npm run smoke
```

Ordinary tests use deterministic fakes, local fixtures, and loopback mock
servers. Live-provider tests and real-project checks are explicit opt-ins.

Read [Architecture](docs/architecture.md) before changing dependency boundaries
and [Development](docs/development.md) before implementing a coherent component.

</details>

---

### License

A project license has not been published yet. Until one is added, this
repository is not offered under an open-source license.

---

### Links

- [GitHub](https://github.com/yafitzdev/encoder-gym)
- [Platform Specification](docs/platform-spec.md)
- [Pilot Quick Start](docs/pilot-quickstart.md)
- [CLI Guide](docs/cli.md)
- [Desktop Guide](ui/README.md)
- [Architecture](docs/architecture.md)
- [Provenance](docs/provenance.md)
- [Current Project Status](docs/current-status.md)
- [Development Guide](docs/development.md)

Yan Fitzner — [GitHub](https://github.com/yafitzdev) • [Hugging Face](https://huggingface.co/yafitzdev)
