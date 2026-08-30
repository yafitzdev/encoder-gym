# Pilot quick start

This reference run exercises the complete local product from ordinary benchmark
files. It uses deterministic fake generation and hashing-linear training, so it
needs no network, credentials, model download, Python, or GPU.

The manifest is `examples/pilot-support/project-bootstrap.toml`. It declares:

- four support labels and two arbitrary categorical dimensions;
- a JSONL development cohort with 16 rows;
- a CSV sealed cohort with 10 different rows;
- exactly 20 accepted training rows, four reserved for one bounded iteration;
- aggregate-only, adaptation-ineligible sealed evidence; and
- finite generation, training, evaluation, retry, and iteration settings.

Use a dedicated local database when trying the pilot:

```powershell
$env:SYNTH_DATABASE_URL = "sqlite://data/pilot-support.db?mode=rwc"
cargo run -p synthetic-data-cli -- project bootstrap-preview examples/pilot-support/project-bootstrap.toml
cargo run -p synthetic-data-cli -- --output json project bootstrap examples/pilot-support/project-bootstrap.toml
```

Preview must report `eligible: true`, 16 initial generation cells, clean
contamination, 16 accepted development rows, and 10 accepted sealed rows. It
does not write the database.

Bootstrap prints a bootstrap ID, preparation ID, both source snapshot IDs, the
workflow definition ID, and an exact next command. Repeating it unchanged must
return `created: false` and the same identities.

Run the printed command:

```text
cargo run -p synthetic-data-cli -- workflow start <DEFINITION_ID>
cargo run -p synthetic-data-cli -- workflow status <RUN_ID>
```

If review mode pauses on a proposal, inspect the status and authorize the
bounded iteration explicitly:

```text
cargo run -p synthetic-data-cli -- workflow approve <RUN_ID>
```

After development stops, sealed evaluation and promotion remain separate
explicit actions:

```text
cargo run -p synthetic-data-cli -- workflow finalize <RUN_ID>
cargo run -p synthetic-data-cli -- workflow promote <RUN_ID>
cargo run -p synthetic-data-cli -- provenance project-bootstrap <BOOTSTRAP_ID>
cargo run -p synthetic-data-cli -- provenance workflow-run <RUN_ID>
cargo run -p synthetic-data-cli -- doctor
```

`doctor` verifies bootstrap identities, completed imports, immutable all-test
snapshot membership and provenance, preparation/workflow facts, budgets,
evidence isolation, and downstream artifacts.

Changing either source file changes the bootstrap fingerprint and creates new
immutable history; it never edits the earlier benchmark or preparation.

For a real local encoder, first register a supported BERT bundle as documented
in `docs/transformer-training.md`, copy the pilot manifest, select `bert-cpu`,
and set its immutable `base_model_id`. For real generation, copy the manifest,
select `openai-compatible`, configure a finite row/request budget, and provide
the API key only through `SYNTH_OPENAI_API_KEY`. Those are deliberate external
smokes and are not part of ordinary tests.

The checked-in, ignored live bootstrap smoke uses four one-row requests at
most, a 1,024-token ceiling per request, then requires ordinary training and
development-evaluation artifacts. Provider-specific request controls can be
passed through the strict `[project.generation.extra]` map; the DeepSeek smoke
uses `thinking = { type = "disabled" }` so the bounded output budget is spent
on JSON rows rather than reasoning:

```powershell
$env:SYNTH_OPENAI_API_KEY = "..."
$env:SYNTH_E2E_OPENAI_BASE_URL = "https://api.deepseek.com"
$env:SYNTH_E2E_OPENAI_MODEL = "deepseek-v4-flash"
cargo run -p synthetic-data-cli -- backend check --base-url $env:SYNTH_E2E_OPENAI_BASE_URL --model $env:SYNTH_E2E_OPENAI_MODEL
cargo test -p synthetic-data-cli --test project_bootstrap_cli bootstrapped_pilot_runs_bounded_openai_compatible_generation -- --ignored --nocapture
```

Provider response bodies are not persisted on failures. Durable job errors
retain only the HTTP status and safe structured identifiers such as provider
error code, type, and parameter.
