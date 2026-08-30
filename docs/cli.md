# CLI reference guide

The terminal is the only new presentation surface for Slices 2–6. Use
`synth <COMMAND> --help` for the generated flag reference. Global
`--output human|json` behavior, pagination, exit status, and file-output rules
are documented in `operations.md`.

Transformer-specific command groups are:

- `synth encoder register|list|show|verify`
- `synth training run|list|status|cancel`
- `synth training checkpoints|checkpoint|predict|continue`
- `synth evaluation run`
- `synth provenance checkpoint`
- `synth doctor`

`training run --backend bert-cpu` requires either `--encoder-id` or
`training.base_model_id` in strict TOML. Explicit flags override TOML. Durable
progress is written to stderr; JSON results remain a single value on stdout.
There is no interactive TUI, graphical UI, or new HTTP endpoint for this work.

OpenAI-compatible backend commands are:

- `synth backend configure --base-url URL --model MODEL` to persist non-secret
  provider settings;
- `synth backend check` to authenticate without generating and verify that the
  configured model appears in the provider model list; and
- `synth backend show` to inspect the persisted non-secret configuration.

`backend check` accepts `--base-url` and `--model` overrides and reads the key
only from the named `--api-key-env` environment variable.

Semantic catalog commands are:

- `synth semantic profile-create <FILE>` and `profile-revise <ID> <FILE>` for
  immutable reusable or dataset-scoped profile versions;
- `profile-list|profile-show` for exact version inspection;
- `suggest <DATASET_ID>` for read-only compatible reusable candidates;
- `bind <DATASET_ID> <PROFILE_ID>` and `unbind ... --layer ...` for explicit,
  append-only attachment decisions;
- `bindings|resolve <DATASET_ID>` for active layers and effective semantics; and
- `job-context <JOB_ID>` for the exact context pinned before generation.

There is no automatic name-based binding. With no explicit binding, ordinary
task/schema inference remains the fallback. See `semantic-catalog.md`.

Declarative project-preparation commands are:

- `synth project preview <MANIFEST>` to resolve exact initial cell targets,
  request estimates, cohort disclosures, leakage checks, governance mode, and
  the finite stage graph without writing project artifacts;
- `synth project prepare <MANIFEST>` to atomically persist the normal project
  configuration, cohorts, reports, suites, and workflow definition;
- `synth project show <PREPARATION_ID>` to inspect the immutable preparation and
  its resolved workflow definition; and
- `synth project list` to page preparation summaries.

The manifest may be strict TOML or JSON. Its cohort sources are existing
immutable snapshot IDs. A repeated manifest fingerprint returns the original
preparation. Preparation never starts a workflow; run the printed `synth
workflow start <DEFINITION_ID>` command as a separate authorization. Start from
`examples/project-preparation.toml` and see `project-preparation.md`.

Exact initial-budget allocation commands are:

- `synth allocation preview <DATASET_ID> --total-rows 20000` for a read-only
  complete balanced preview;
- `synth allocation create <DATASET_ID> --total-rows 20000` to atomically persist
  the immutable allocation and its ordinary Slice 1 plan;
- `--reserved-rows` to keep part of the requested total outside the initial plan;
- `--policy weighted --weights <JSON_OR_TOML>` for label and dimension-value
  weights;
- `--policy minimum-then-weighted --minimum-per-cell N` for explicit floors;
- `--policy explicit --targets <JSON_OR_TOML>` for complete absolute targets;
- `--constraints <JSON_OR_TOML>` for per-cell minima, maxima, and exclusions; and
- `synth allocation list|show` for immutable persisted results.

Preview accounts for persisted accepted coverage but writes nothing. Infeasible
constraints return structured issues and cannot create a generation plan.

Evaluation-governance commands are:

- `synth cohort create <SNAPSHOT_ID> --name NAME --split test --role
  sealed-acceptance --reason REASON` to bind an immutable snapshot split to its
  initial role;
- `synth cohort list|show|history`, `cohort assign`, and `cohort retire` to
  inspect or append explicit role decisions;
- `synth exposure record <COHORT_ID> --purpose acceptance --disclosure
  aggregate` to append a disclosure fact;
- `synth exposure list|risk` to inspect the exposure ledger and its derived
  adaptive-overfitting risk; and
- `--retirement-reason` to atomically retire a sealed cohort when row-level
  manual inspection discloses it.

Sealed cohorts reject training, diagnosis, advisor, optimization, and other
adaptation-eligible exposure. They can be used for acceptance or manual
inspection only. Role decisions and exposure facts are append-only.

Leakage-defense commands are:

- `synth snapshot create ... --group-dimension account_id` to keep all rows
  sharing a declared categorical group value in one deterministic split;
- `synth contamination check --cohort <ID> --cohort <ID>` for strict source,
  exact-text, and normalized-text cross-cohort checks;
- `--group-dimension account_id` to include group overlap and `--policy FILE`
  to load explicit nonzero thresholds; and
- `synth contamination show|list|override` to inspect immutable reports or add
  a named, reasoned exception without altering the report.

Contamination reports persist fingerprints of overlapping evidence rather than
raw overlapping text. A blocked report is ineligible for a benchmark suite
unless it has an explicit persisted override.

Benchmark and acceptance commands are:

- `synth benchmark create --definition benchmark.toml` to resolve a strict
  definition into an immutable suite with cohort, role, protocol,
  contamination, model-format, disclosure, and threshold fingerprints;
- `synth benchmark validate|show|list` to verify that bound role decisions and
  contamination evidence remain eligible;
- `synth benchmark assess <SUITE_ID> --run <COHORT_ID>=<RUN_ID>` with optional
  paired `--comparison <COHORT_ID>=<COMPARISON_ID>` values; and
- `synth benchmark assessment-show|assessment-list` to inspect immutable
  `pass`, `fail`, `inconclusive`, or `invalid` outcomes and exact reasons.

Metric contracts support overall metrics, per-label precision/recall/F1,
arbitrary persisted slice keys, minimum support, maximum regressions, paired
confidence bounds, and optional McNemar significance. Sealed suites require
aggregate, adaptation-ineligible disclosure and a separate
`--authorize-sealed` acknowledgement. Repeating an identical assessment returns
the existing artifact.

Project bootstrap commands are:

- `synth project bootstrap-preview <MANIFEST>` to validate local JSONL/CSV
  development and sealed sources, source-content fingerprints, mappings,
  contamination, exact allocation, and workflow eligibility without mutation;
- `synth project bootstrap <MANIFEST>` to atomically persist ordinary completed
  imports, immutable all-test cohort snapshots, and the existing preparation
  bundle, then print the separate workflow-start command;
- `synth project bootstrap-show <ID>` to inspect the immutable bootstrap,
  preparation, source dataset/import/snapshot identities, and next command; and
- `synth project bootstrap-list` for bounded, stable bootstrap history.

Unchanged manifests and source bytes return the original bootstrap. Source
changes produce a new fingerprint and immutable artifact set. Existing
`project preview|prepare` remains the lower-level path for operator-managed
snapshot IDs.

Finite-workflow commands are:

- `synth workflow define --definition workflow.toml` (start from
  `examples/workflow.toml`) to persist resolved slice
  references, initial allocation, governance mode, finite budgets, and stop
  policy as one immutable fingerprinted definition;
- `synth workflow start <DEFINITION_ID>` to create and drive the authorized
  initial pipeline until development completion, an approval pause, a bounded
  stop, or a failure;
- `synth workflow status <RUN_ID>` to show the persisted run, usage, complete
  attempt chain, linked generation jobs, and per-cell coverage;
- `synth workflow watch <RUN_ID>` to wait until a concurrently running workflow
  pauses or stops, while `history|list` remain scriptable snapshots;
- `synth workflow approve <RUN_ID> --recommendation-id <ID>` to persist an
  exact compatible review decision and continue a review-each-iteration run;
- `synth workflow resume <RUN_ID>` to resume an interrupted running stage or
  start the next bounded retry after a retryable failure;
- `synth workflow cancel <RUN_ID>` to persist cancellation and forward it to an
  active generation job for observation between batches;
- `synth workflow finalize <RUN_ID>` to explicitly run only the configured
  sealed aggregate-only suite after development has stopped; and
- `synth workflow promote <RUN_ID>` plus `promotion-show <ID>` to persist and
  inspect the immutable promote/reject record.

The workflow definition resolves analysis and optimization protocols, optional
advisor configuration, `training_iteration_policy = "fresh"`, exact suite and
project fingerprints, and all budgets before the run starts. Advisor secrets
are environment-variable names only. `review_each_iteration` always pauses;
`preauthorized_bounded` records each envelope decision and rejects actions that
exceed its backend, model, configuration, row, request, token, or iteration
limits. Development acceptance is not final acceptance: only `finalize` may
touch the sealed suite, and sealed results never feed analysis or optimization.

Stage attempts form a fingerprint-linked append-only chain. Run updates use the
expected latest attempt as an optimistic concurrency guard. Illegal stage
transitions, stale attempts, retries above the configured ceiling, and usage
above finite row/request/advisor/iteration budgets are rejected by the core
state machine.

Error-analysis commands are:

- `synth analysis create <RUN_ID>` with support, ranking, confidence,
  intersection, evidence-limit, seed, and optional `--comparison-id` flags;
- `synth analysis list|show` for immutable reports;
- `synth analysis findings|finding` for normalized, paged findings;
- `synth analysis evidence` for resolved prediction evidence and JSONL/CSV
  export through `--format ... --file ...`;
- `synth analysis high-confidence-errors|weak-cells|comparison-group` for
  focused diagnostic views;
- `synth analysis review|reviews` for append-only human decisions; and
- `synth analysis export` for the immutable report summary.

Finding keys are canonical JSON strings. Quote them when invoking a shell.
See `error-analysis.md` for metric semantics and complete examples.

Optimization and campaign commands are:

- `synth optimize preview|propose|scenarios` for bounded decision artifacts;
- `synth optimize scenario-show|scenario-materialize` for policy alternatives;
- `synth optimize list|show|recommendations|explain` for stable inspection;
- `synth optimize review|apply|rebase` for the human approval boundary;
- `synth optimize training-space|training-candidates` for typed finite
  experiment candidates that never start training;
- `synth optimize export|export-summary|export-recommendations` for JSON,
  JSONL, and CSV artifacts; and
- `synth campaign create|list|show|link|assess` for a validated append-only
  experiment ledger.

Use a strict TOML/JSON protocol with decision-grade `preview` and `propose`.
`apply` requires an approval ID and only creates a plan. See
`optimization.md` for semantics and the complete operator workflow.

See `transformer-training.md` for the complete local BERT workflow and
`examples/transformer-project.toml` for non-secret settings.
