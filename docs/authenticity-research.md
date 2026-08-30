# Authenticity research

Authenticity research is a separate, opt-in operation. A bounded Pi agent
searches permitted sources and drafts evidence-backed guidance about how real
inputs look. It never inserts web content into a dataset and never starts
generation, training, evaluation, or optimization.

## Local prerequisites

The main product remains Rust-only. Research additionally needs a supported
Node.js runtime because Pi runs in an isolated TypeScript sidecar:

```text
cd adapters/research-agent-pi
npm ci
npm run check
cd ../..
```

The package lock pins the exact Pi dependency graph. Rust owns all search,
fetch, budget, persistence, evidence, review, and binding decisions. Pi receives
only the six research tools documented in
[`research-agent-spec.md`](research-agent-spec.md).

## Complete offline walkthrough

Create the checked-in example dataset in a fresh database:

```text
cargo run -p synthetic-data-cli -- config init examples/project.toml
```

Validate the brief without executing Pi or making an external call:

```text
cargo run -p synthetic-data-cli -- research brief-validate examples/research/support-authenticity-brief.json
```

Run the genuine Pi loop with its deterministic faux model and local corpus:

```text
cargo run -p synthetic-data-cli -- research start examples/research/support-authenticity-brief.json --script examples/research/support-scripted-turns.json --corpus examples/research/support-corpus.json
```

The start command prints whether external calls will occur and the exact limits
before execution. Save the returned run and profile IDs, then inspect durable
facts and make an explicit decision:

```text
cargo run -p synthetic-data-cli -- research status <RUN_ID>
cargo run -p synthetic-data-cli -- research evidence <RUN_ID>
cargo run -p synthetic-data-cli -- research profile <RUN_ID>
cargo run -p synthetic-data-cli -- research review <PROFILE_ID> --approve --reason "Evidence reviewed"
cargo run -p synthetic-data-cli -- research bind <DATASET_OR_PLAN_ID> <PROFILE_ID>
cargo run -p synthetic-data-cli -- research context <DATASET_OR_PLAN_ID>
```

Rejection and revision requests use `--reject` and `--request-revision`.
Reviews and bindings are append-only. Only the latest exact approval can be
bound.

Create or use a plan and generate normally. Research is not re-entered:

```text
cargo run -p synthetic-data-cli -- generate <PLAN_ID> --backend fake
cargo run -p synthetic-data-cli -- job execution <JOB_ID>
cargo run -p synthetic-data-cli -- job prompt <JOB_ID> --cell-index 0 --requested-count 1
cargo run -p synthetic-data-cli -- doctor
```

The execution pins the authenticity-context, prompt-template, and source
novelty-guard fingerprints. Prompt preview shows the exact abstract profile
guidance but not retained excerpts or page content.

## Control and recovery

`research watch <RUN_ID>` polls persisted progress. `research cancel <RUN_ID>`
sets durable cancellation intent; the host checks it before further tool work.
If the host process was terminated, run `research recover <RUN_ID>`. Recovery
marks open calls interrupted and the run failed without replaying a possibly
paid search, fetch, or model call. Collected evidence remains inspectable.

## Real-provider run

Copy the example brief and change `provider.provider` and `provider.model` to a
pair recognized by the pinned Pi model registry. Set `provider.api_key_env` to
the uppercase name of the environment variable holding the model credential;
never put the credential itself in the brief.

Real search uses the Brave Search web endpoint. Put its credential in a
separate environment variable, then start without `--script` or `--corpus`:

```powershell
$env:MY_MODEL_API_KEY = "..."
$env:BRAVE_SEARCH_API_KEY = "..."
cargo run -p synthetic-data-cli -- research start my-research-brief.json --search-api-key-env BRAVE_SEARCH_API_KEY
```

The brief's domain/source allowlist and all finite budgets are enforced by the
Rust host. A live smoke run should use deliberately small search, page, token,
duration, and cost ceilings. Ordinary repository tests use no network or paid
provider.
