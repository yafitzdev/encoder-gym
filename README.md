<!-- README.md -->

<div align="center">

# Encoder Gym

### Local, reproducible encoder development—from data to decision.

**Generate, curate, train, evaluate, analyze, and improve text encoders while preserving the evidence behind every change.**

[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust)](https://www.rust-lang.org/)
[![Version: 0.1.0](https://img.shields.io/badge/version-0.1.0-2563eb)](CHANGELOG.md)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-yellow)](LICENSE)

[Start Here](#start-here) • [Why `encoder-gym`?](#why-encoder-gym) • [The Encoder Loop](#the-encoder-loop) • [CLI](#cli) • [Quick Start](#quick-start) • [Studio](#studio) • [Governance](#governance--provenance) • [Architecture](#architecture) • [Limitations](#limitations) • [Documentation](#links) • [GitHub](https://github.com/yafitzdev/encoder-gym)

</div>

<br />

---

<a id="start-here"></a>

### Where to start 🚀

```powershell
$env:SYNTH_DATABASE_URL = "sqlite://encoder-gym-demo.db?mode=rwc"

cargo run -p synthetic-data-cli -- project bootstrap-preview demo/project-bootstrap.toml
cargo run -p synthetic-data-cli -- project bootstrap demo/project-bootstrap.toml
```

Preview writes nothing. Bootstrap creates the immutable inputs and prints the
exact next command for starting the workflow. Continue with that command, then
use `workflow status` to see the next explicit decision.

See the [Pilot Quick Start](docs/QUICKSTART.md) for the full offline run.

---

### About

An **encoder** takes an input, just like an LLM, but instead of generating a
response, it turns that input into a useful representation. Fine-tune it for a
task and it can classify, rank, or compare inputs quickly and cheaply.

A fine-tuned support encoder might return:

```text
Input:  "I was charged twice for the same order."

Output: {
  "billing": 0.93,
  "technical_support": 0.05,
  "account_access": 0.02
}
```

`encoder-gym` handles the full loop: data generation and curation,
training, evaluation, error analysis, and comparison. Its agent inspects the
evaluation and test set, reasons about which data should be added or removed,
trains the next candidate, and checks whether it is actually better.

The idea is simple: set up `encoder-gym`, start a run, go to sleep, and wake up to
a better encoder than the one you had before—with the evidence to prove it.

Yan Fitzner — ([LinkedIn](https://www.linkedin.com/in/yan-fitzner/), [GitHub](https://github.com/yafitzdev), [HuggingFace](https://huggingface.co/yafitzdev)).

---

### Why `encoder-gym`?

**Agent-native, with tools 🤖**
> The agent can inspect evaluation evidence, reason about dataset weaknesses,
> use bounded tools to change the data, train another candidate, and measure the
> result.

**Recovery built in ♻️**
> Runs, artifacts, and decisions are persisted as work happens. Interrupted
> training and optimization can be inspected, resumed, or safely replayed.

**Custom-made GUI 🖥️**
> A purpose-built desktop interface makes encoder projects, datasets, runs, and
> results easy to manage. The CLI provides precise, scriptable control.

**Fully local 🏠**
> Data, models, SQLite state, evaluation, and supported training can stay on
> your machine. External model providers are optional.

---

### What You Can Do

| Stage | What `encoder-gym` does | Durable output |
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

```mermaid
block-beta
    columns 6

    space inspect["Inspect failures"] reason["Reason about data"] curate["Add, remove,<br/>or rebalance"] train["Train next"] space
    space:6
    setup["1 · SETUP"] space evaluate["2 · STATUS<br/>Evaluate"]:2 space report["3 · REPORT"]

    setup --> evaluate
    evaluate --> report
    evaluate --> inspect
    inspect --> reason
    reason --> curate
    curate --> train
    train --> evaluate
```

Setup defines the job. Status is the working loop: evaluate, understand the
failures, improve the data, and train again. Report delivers the best candidate
along with its measurements and a trace of how it was produced.

---

<a id="cli"></a>

### CLI

The `synth` CLI is the main way to run `encoder-gym`. These are the commands that
drive a complete project from setup to a final result:

| Command | What it does |
|---------|--------------|
| `synth project bootstrap-preview <MANIFEST>` | Validate the complete project and preview what will be created without writing anything. |
| `synth project bootstrap <MANIFEST>` | Import the local benchmark data and prepare a startable project. |
| `synth workflow start <DEFINITION_ID>` | Start the encoder-development workflow created during setup. |
| `synth workflow status <RUN_ID>` | Show the current stage, results so far, and the next required action. |
| `synth workflow watch <RUN_ID>` | Wait until the run pauses, finishes development, or completes. |
| `synth workflow approve <RUN_ID>` | Approve a bounded data proposal when the workflow reaches a review boundary. |
| `synth workflow finalize <RUN_ID>` | Run the configured sealed acceptance evaluation once. |
| `synth workflow promote <RUN_ID>` | Record whether the final candidate replaces the baseline. |
| `synth provenance workflow-run <RUN_ID>` | Trace the run back through its data, training, evaluation, and decisions. |
| `synth doctor` | Verify the database, configuration, artifacts, and optional backends. |

Specialized command families expose each stage directly: `dataset`, `snapshot`,
and `quality` manage data; `plan`, `generate`, and `job` run generation;
`encoder`, `training`, and `evaluation` operate models; and `analysis`,
`optimize`, and `recovery` handle improvement and interrupted work.

When running from this repository, replace `synth` with
`cargo run -p synthetic-data-cli --`. Add `--output json` before a subcommand
for machine-readable output. See the [full CLI reference](docs/CLI.md) for every
command and option.

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
$env:SYNTH_DATABASE_URL = "sqlite://encoder-gym-demo.db?mode=rwc"

cargo run -p synthetic-data-cli -- project bootstrap-preview demo/project-bootstrap.toml
cargo run -p synthetic-data-cli -- --output json project bootstrap demo/project-bootstrap.toml
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

See [Pilot Quick Start](docs/QUICKSTART.md) for expected outputs,
idempotent replay, and the opt-in real-provider smoke.

#### Launch Encoder Gym Studio

```powershell
cd ui
npm ci
npm start
```

Studio starts with an empty project library. Creating or opening a project
does not start training, evaluation, or a provider call. See the
[Studio Guide](ui/README.md).

</details>

---

<a id="studio"></a>

### Encoder Gym Studio 🖥️

Encoder Gym Studio puts the full workflow in one focused interface: configure a
run, follow the agent while it works, and decide whether the resulting encoder
is worth keeping.

#### 1. Set up the run

![Encoder Gym Studio setup screen](docs/assets/studio/setup.png)

Choose the baseline encoder, starting dataset, evaluation benchmark, and the
models that will inspect failures and generate new training data.

#### 2. Follow the agent loop

![Encoder Gym Studio status screen](docs/assets/studio/status.png)

See which stage is active, what the agent is doing, and how far the current
training or evaluation task has progressed.

#### 3. Review the result

![Encoder Gym Studio report screen](docs/assets/studio/report.png)

Compare the candidate against the baseline, inspect every metric, and see the
final keep-or-reject decision.

```powershell
cd ui
npm ci
npm start
```

See the [Studio Guide](ui/README.md) for project setup and the complete interface.

---

<a id="governance--provenance"></a>

<details>

<summary><strong>📦 Governance and Provenance</strong> → <a href="docs/features/governance/workflow-governance-spec.md">Full Workflow Contract</a></summary>

<br />

`encoder-gym` treats experiment governance as executable product behavior:

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

See [Provenance](docs/PROVENANCE.md), [Benchmark Stewardship](docs/features/evaluation/benchmark-stewardship-spec.md),
and [Controlled Workflow Governance](docs/features/governance/workflow-governance-spec.md).

</details>

---

<a id="architecture"></a>

<details>

<summary><strong>📦 Architecture</strong> → <a href="docs/ARCHITECTURE.md">Full Architecture Guide</a></summary>

<br />

```text
Encoder Gym Studio      synth CLI        local generation server
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
[Limitations](docs/LIMITATIONS.md).

</details>

---

### License

Apache License 2.0. See [LICENSE](LICENSE).

---

### Links

- [GitHub](https://github.com/yafitzdev/encoder-gym)
- [Changelog](CHANGELOG.md)
- [License](LICENSE)
- [Documentation](docs/README.md)
- [Platform Specification](docs/PLATFORM.md)
- [Pilot Quick Start](docs/QUICKSTART.md)
- [CLI Guide](docs/CLI.md)
- [Studio Guide](ui/README.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Provenance](docs/PROVENANCE.md)
- [Production Readiness](docs/PRODUCTION_READINESS.md)
- [Development Guide](docs/DEVELOPMENT.md)
