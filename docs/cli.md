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

Generation execution inspection commands are:

- `synth config construction-preview <PROJECT.toml> --label LABEL [--dimension
  NAME=VALUE] [--start-index N] [--count N]` to compile recipes, show
  deterministic row seeds, and reveal whether a provider call is required;
- `synth job execution <JOB_ID>` for the immutable, non-secret runtime
  specification and its fingerprint;
- `synth job attempts <JOB_ID>` for ordered provider requests or deterministic
  construction batches, outcomes, usage, errors, and fingerprints; and
- `synth job prompt <JOB_ID> --cell-index N [--requested-count N]` to reconstruct
  the next prepared batch and, when needed, its request from pinned facts
  without contacting the provider.

These commands inspect persisted facts; they do not regenerate, retry, or
silently use current backend configuration.

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

Authenticity-research commands are:

- `synth research brief-validate <FILE>` to resolve dataset and semantic facts
  and validate all source/provider/budget policy without an external call;
- `research start <FILE>` to run the bounded Pi loop in the foreground;
- `status|watch`, `evidence`, and `profile` to inspect persisted progress and
  artifacts;
- `cancel <RUN_ID>` and `recover <RUN_ID>` for durable control without silently
  replaying uncertain external calls;
- `review <PROFILE_ID> --approve|--reject|--request-revision --reason TEXT` for
  append-only operator decisions; and
- `bind <DATASET_OR_PLAN_ID> <PROFILE_ID>` plus `context <DATASET_OR_PLAN_ID>`
  for the explicit generation handoff.

The offline start form adds `--script` and `--corpus`; real research omits both
and uses `--search-api-key-env`. Research never starts generation. See
`authenticity-research.md`.

Dataset Architect commands are:

- `synth architect brief-validate <FILE>` to resolve dataset, accepted coverage,
  semantic context, an optional approved authenticity profile, and optional
  governed development diagnostics before any model call;
- `architect start <FILE> [--script <FILE>]` to run the finite Pi loop;
- `status|watch|proposal`, `cancel`, and `recover` to inspect or control durable
  execution without replaying uncertain model calls;
- `review <PROPOSAL_ID> --approve|--reject|--request-revision --reason TEXT` for
  append-only operator decisions;
- `apply <PROPOSAL_ID>` to atomically create an ordinary unequal generation plan
  and immutable per-cell strategy context; and
- `context <PLAN_ID>` to inspect that exact generation handoff.

Application fails if accepted coverage changed after the proposal. An optional
`analysis_report_id` must resolve to exactly one active development or diagnostic
cohort; the CLI records a `dataset_architecture` slices exposure. Sealed evidence
is structurally ineligible. Generation pins the context and makes no architect
or Pi call. See `dataset-architect-spec.md` and the checked-in offline example in
`examples/architect/`.

Dataset-qualification commands are:

- `synth quality policy-preview --preset fast|balanced|strict` to inspect the
  complete finite policy before writing state;
- `quality audit-create <DATASET_ID>` and `audit-start <RUN_ID>` to pin every
  accepted source row and run the deterministic fake evaluator by default;
- `audit-status|audit-watch|audit-cancel|audit-recover` for durable control;
- `assessments <RUN_ID>` and `summary <RUN_ID>` for immutable row evidence
  and persisted quality coverage;
- `curate <RUN_ID>`, `row-review <ASSESSMENT_ID>` (compatibility form),
  `row-review --report-id <REPORT_ID> --source-row-id <ROW_ID>`, and
  `manifest-review <PROPOSAL_ID>` for deterministic selection plus explicit
  append-only operator decisions; and
- `snapshot create <DATASET_ID> --quality-manifest <MANIFEST_ID> ...` to build
  an ordinary immutable snapshot containing only approved members.

The deterministic fake remains the default. External evaluation is an explicit
two-step opt-in using the existing saved OpenAI-compatible generation backend:

```powershell
synth backend configure --base-url https://provider.example/v1 --model provider-model
synth quality audit-create <DATASET_ID> --preset fast --egress external-candidate-text --evaluator openai-compatible
$env:SYNTH_OPENAI_API_KEY = "..."
synth quality audit-start <RUN_ID> --api-key-env SYNTH_OPENAI_API_KEY
# The same option is available on audit-recover.
```

The V1 OpenAI-compatible evaluator supports only the `fast` preset. The
`balanced` and `strict` presets require one or more genuinely independent
reviewer model or backend configurations, which the current CLI cannot yet
pin. The deterministic fake evaluator continues to support all three presets
for offline workflows.

Creation fixes the quality prompt policy, zero temperature, deterministic
seed, protocol, response limits, per-request output-token ceiling, endpoint,
and model into the evaluator identity fingerprint stored on the run. The
adapter clamps the aggregate request allowance to a default 16,384 output
tokens before emitting `max_tokens`. Start and recovery read the credential
only from the named environment variable and require reconstructed identities
to equal the persisted identities before any request. The credential and its
environment-variable name are not persisted. A non-loopback endpoint must use
HTTPS even when it accepts unauthenticated requests.

The saved generation backend is currently the reconstruction source rather than
an immutable evaluator-configuration snapshot. Changing its endpoint or model
therefore makes an existing external audit safely non-runnable instead of
silently changing providers. Presets still impose finite row, request, attempt,
input-token, output-token, and total-token ceilings. `--max-cost-microusd` is
rejected with `--evaluator openai-compatible` until provider pricing can be
persisted and reproduced as part of the audit configuration.

Audit creation resolves semantic-catalog and optional approved authenticity
guidance once, stores that exact normalized payload, and never consults a newer
current binding while resuming. Unevaluated, invalid, borderline, and
quarantined rows are excluded unless an explicit row review includes them.
The report/row review target works even when no assessment exists, and the CLI
fully verifies the persisted report and source-row membership before appending
the reviewer and reason. A proposal with newer row reviews is stale and cannot
be approved until `quality curate <RUN_ID>` produces its successor.

Routine large-population output is bounded: audit creation/start/status,
curation, and manifest approval return identifiers, fingerprints, counts, and
compact progress/report summaries instead of embedding every plan item,
proposal entry, or manifest member. Use `quality proposal <PROPOSAL_ID>` and
`quality manifest <MANIFEST_ID>` when full verified row membership is intended.
`quality summary <RUN_ID>` includes structurally accepted population rows,
assessed rows, qualified rows, and `remaining_qualified_rows`; that last value
is the persisted audit population minus qualified rows, not a generation target.
Legacy snapshots remain valid and visibly lack a curation application. See
`dataset-quality-spec.md`.

Declarative project-preparation commands are:

- `synth project preview <MANIFEST>` to resolve exact initial cell targets,
  request estimates, cohort disclosures, leakage checks, governance mode, and
  the finite stage graph without writing project artifacts;
- `synth project prepare <MANIFEST>` to atomically persist the normal project
  configuration, cohorts, reports, suites, immutable benchmark bundle, and
  workflow definition;
- `synth project show <PREPARATION_ID>` to inspect the immutable preparation and
  its resolved workflow definition; and
- `synth project list` to page preparation summaries.

The manifest may be strict TOML or JSON. Its cohort sources are existing
immutable snapshot IDs. A repeated manifest fingerprint returns the original
preparation. Preparation never starts a workflow; run the printed `synth
workflow start <DEFINITION_ID>` command as a separate authorization. Start from
`examples/project-preparation.toml` and see `project-preparation.md`.

Exact initial-budget allocation commands are:

- `synth plan describe <DATASET_ID>` to inspect Cartesian cardinality without
  materializing every cell;
- `synth allocation preview <DATASET_ID> --total-rows 20000` for a read-only
  complete balanced preview;
- `synth allocation create <DATASET_ID> --total-rows 20000` to atomically persist
  the immutable allocation and its ordinary Slice 1 plan;
- `--reserved-rows` to keep part of the requested total outside the initial plan;
- `--policy weighted --weights <JSON_OR_TOML>` for label and dimension-value
  weights;
- `--policy minimum-then-weighted --minimum-per-cell N` for explicit floors;
- `--policy explicit --targets <JSON_OR_TOML>` for complete absolute targets;
- `--constraints <JSON_OR_TOML>` for exact per-cell constraints or concise
  partial selector rules with minima, maxima, and exclusions;
- `--explain` or `synth allocation explain <ID>` for label and dimension-value
  distributions; and
- `synth allocation list|show` for immutable persisted results.

Preview accounts for persisted accepted coverage but writes nothing. Infeasible
constraints return structured issues and cannot create a generation plan.
Totals are absolute accepted-row targets. See `allocation.md` for policy,
rounding, selector-overlap, safety-limit, and provenance behavior.

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
- `synth workflow define --definition workflow.toml --group-dimension account_id`
  to apply that group identity to the mandatory zero-tolerance check over the
  complete development and optional sealed suite union; and
- `synth contamination show|list|override` to inspect immutable reports or add
  a named, reasoned exception without altering the report.

Contamination reports persist fingerprints of overlapping evidence rather than
raw overlapping text. A blocked report is ineligible for a benchmark suite
unless it has an explicit persisted override.

For `workflow define`, `--group-dimension` must match the single dimension
already pinned by suite-local contamination reports. If none is pinned, the flag
introduces one; if the flag is omitted, the command infers the single pinned
dimension. Conflicting dimensions and missing or empty group values fail the
definition. Unlike standalone suite creation, the global workflow check accepts
neither nonzero thresholds nor an override.

Benchmark and acceptance commands are:

- `synth benchmark create --definition benchmark.toml` to resolve a strict
  definition into an immutable suite with cohort, role, protocol,
  contamination, model-format, disclosure, and threshold fingerprints;
- `synth benchmark validate|show|list` to verify that bound role decisions and
  contamination evidence remain eligible;
- `synth benchmark assess <SUITE_ID> --run <COHORT_ID>=<RUN_ID>` with optional
  paired `--comparison <COHORT_ID>=<COMPARISON_ID>` values; and
- `synth benchmark assessment-show|assessment-list` to inspect immutable
  `pass`, `fail`, `inconclusive`, or `invalid` outcomes and exact reasons;
- `synth benchmark training-check-show <CHECK_ID>` to inspect one immutable
  training-to-benchmark check, including exact population, cohort, bundle,
  combined-report, protocol, status, and fingerprint pins;
- `synth benchmark training-check-validate <CHECK_ID>` to deeply reload the
  pinned snapshot, bundle, reports, normalized joins, and current active role
  decisions and report `valid`, `training_allowed`, status, and reasons; and
- `synth benchmark training-check-list` with optional `--snapshot-id <ID>`,
  `--benchmark-bundle-id <ID>`, and `--status clean|blocked` filters to page
  historical checks under the current `train_and_validation_v1` input protocol.

Metric contracts require at least one effective decision bound and support
overall metrics, per-label precision/recall/F1, canonical typed slice keys,
minimum support, maximum regressions, paired confidence bounds, and optional
McNemar significance. Paired evidence is accepted only when its fingerprint,
candidate run, protocol, cohort, metrics, and internal counts agree. Sealed
suites accept overall metrics only, require aggregate, adaptation-ineligible
disclosure, and require a separate `--authorize-sealed` acknowledgement.
Repeating an identical assessment returns the existing artifact.

Training checks are created automatically by governed workflow training; there
is no manual create command. The current input protocol treats every train and
validation member as model-influencing and excludes test-only members. For each
candidate snapshot, the workflow compares that exact population with every
bundle-pinned development and optional sealed cohort under the bundle's group
dimension and the default zero-overlap policy. A blocked check is still
persisted and shown by these commands, but the stage fails non-retryably before
backend startup and creates no training run or checkpoint.

The same gate precedes lookup of a reusable completed training run, so reuse is
not a bypass. Every governed run also stores an immutable input binding to the
check plus a digest of the exact ordered labels and train/validation examples
seen by the backend. The runner reproduces that digest before startup, and only
a completed run with the exact binding and resolved training configuration may
be reused. It also seals the verified example sequence and rejects a source
that changes on any subsequent backend read. Training-run persistence rejects
skipped lifecycle states and completion without the exact final checkpoint.
Later development/iteration evaluation, sealed finalization, and promotion
revalidate the linked clean check. Use `synth provenance
training-benchmark-check <CHECK_ID>` to trace it to the exact snapshot,
benchmark bundle, and combined contamination report. New `workflow promote`
records also pin the check ID and fingerprint, and model-promotion provenance
includes that check as a parent. Promotion additionally proves that the final
checkpoint, completed training run, input binding, clean check, development
assessment, and sealed assessment form one exact chain.

This firewall covers only data controlled and persisted by the platform. It
checks source-row identity, exact text, normalized text, and an optional declared
group identity. It does not detect semantic paraphrases, prove independence
from base-model pretraining data, or account for equivalent data supplied to a
trainer outside the platform.

`workflow define` reloads the suites and their immutable snapshot evidence,
requires distinct development and optional sealed evidence, and recomputes one
clean zero-tolerance report over exactly their combined cohorts. It reuses an
existing report and `BenchmarkBundle` with matching fingerprints or persists
the derived authority with the definition in one transaction. A
`benchmark_bundle` supplied in the definition file is only an assertion: it
must exactly match the derived binding.

The bundle pins exact cohort-role decision identities as well as suite and
report fingerprints. `workflow start`, `resume`, and each running stage require
those decisions to remain current and active. A role transition or retirement
therefore closes the old authority; create new eligible suites and a new
bundle-backed definition instead of trying to revive it.

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
  `examples/workflow.toml`) to derive or reuse the global benchmark-bundle
  authority and persist resolved slice references, initial allocation,
  governance mode, finite budgets, and stop policy as one immutable
  fingerprinted definition;
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
  inspect the immutable promote/reject record, including the selected clean
  training-benchmark check pin.

The workflow definition resolves analysis and optimization protocols, optional
advisor configuration, `training_iteration_policy = "fresh"`, exact project,
suite, global-contamination, and benchmark-bundle fingerprints, and all budgets
before the run starts. Advisor secrets
are environment-variable names only. `review_each_iteration` always pauses;
`preauthorized_bounded` records each envelope decision and rejects actions that
exceed its backend, model, configuration, row, request, token, or iteration
limits. Development acceptance is not final acceptance: only `finalize` may
touch the sealed suite, and sealed results never feed analysis or optimization.

At each initial or iteration training stage, the workflow creates or reuses the
one executable check for the exact snapshot, bundle, check protocol, and
trainer-input protocol. This happens before either a trainer starts or a
completed run is reused. The linked check is then required throughout downstream
evaluation, finalization, and promotion.

The workflow checks disclosure before the relevant work: diagnosis and
follow-up diagnosis require `row_content`, comparison requires `predictions`,
optimization requires `slices`, and advisor use requires `aggregate` or
`row_content` according to its egress policy. These development purposes are
adaptive, so every participating development cohort must also be
adaptation-eligible. `finalize` instead uses the exact bundle-pinned sealed suite
with aggregate, non-adaptive acceptance exposure. Paired comparisons are joined
to candidate evaluations by cohort; when a regression contract is configured,
their IDs are included in the acceptance assessment and stop decision.

Definitions and runs migrated from before `0046_benchmark_bundles` remain
available to `workflow definition-show|definition-list`, `workflow status|list`,
and provenance inspection, but their null bundle binding is not executable.
They cannot start or advance a workflow; run `workflow define` or project
preparation again to create new governed authority.

Migrations `0047_training_benchmark_checks`,
`0048_promotion_training_benchmark_check`, and
`0049_training_input_authority` do not rewrite historical training
runs, checkpoints, or promotions. Those artifacts remain listable, inspectable,
and traceable, but they receive no retroactive clearance. Legacy training runs
retain an absent input binding and are ineligible for governed reuse. A legacy
promotion has paired null training-check fields and no corresponding provenance
parent; that visible absence cannot satisfy a new governed promotion.

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
