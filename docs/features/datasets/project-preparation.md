# Declarative project preparation

Project preparation turns one strict TOML or JSON manifest into the existing
slice artifacts needed to start a finite encoder-development workflow. It is a
compiler and transaction boundary, not a new workflow engine.

## Bootstrap directly from local files

For a new project, start with
`demo/project-bootstrap.toml`. Its development and optional
sealed cohorts declare local JSONL or CSV sources, text/label fields, and a
mapping for every arbitrary categorical dimension. Relative paths resolve from
the manifest directory.

```text
synth project bootstrap-preview demo/project-bootstrap.toml
synth project bootstrap demo/project-bootstrap.toml
synth project bootstrap-list
synth project bootstrap-show <BOOTSTRAP_ID>
synth workflow start <DEFINITION_ID>
```

Bootstrap preview streams and validates every source but writes nothing. It
reports SHA-256 content identities, processed/accepted/rejected counts, exact
initial allocation, contamination, disclosures, budgets, stage graph, and
eligibility. Benchmark ingestion is strict: one rejected or empty source blocks
creation instead of silently changing evaluation evidence.

Bootstrap creation performs one SQLite transaction containing ordinary source
datasets, completed imports, accepted source rows, immutable all-test snapshots,
the preparation bundle described below (including its benchmark bundle), and
one small bootstrap summary. A late failure rolls back every artifact.
Idempotency binds the canonical manifest and source-content hashes: unchanged
bytes return the original record, while changed bytes create new immutable
history. Paths are retained as import provenance but do not substitute for
content identity.

Bootstrap never starts a workflow, trains, evaluates, or calls a provider. The
printed `workflow start` command is a separate authorization. Sealed cohorts
still must be aggregate-only and adaptation-ineligible, and all existing
contamination and budget checks run through the same preparation compiler.

## Prepare from existing snapshots

The manifest owns:

- the synthetic training-dataset schema and replaceable generation backend;
- exact total and reserved row budgets plus balanced, weighted, constrained,
  or explicit cell allocation;
- immutable development and optional sealed snapshot/split sources;
- cohort roles, disclosure levels, adaptation eligibility, and leakage limits;
- development and sealed acceptance contracts; and
- an explicit benchmark-readiness policy plus operator approval rationale. The
  compiler computes readiness from the exact cohort populations and will not
  materialize approval when deterministic checks are blocked; and
- training/evaluation settings, iteration governance, stop policy, and finite
  row/request/advisor/stage budgets.

For existing benchmark artifacts, author a strict preparation manifest. Select
generation supervision when the finite workflow should generate through
deterministic quality windows, bounded prompt repair, and directly-qualified
curation. The manifest contains separate non-secret
generator and evaluator profiles; external credentials are named by an
environment variable and are never part of the manifest or preparation record.
First create or import the
benchmark dataset and an immutable snapshot, then provide its snapshot
UUID. Snapshot labels must match the project label order exactly, and the
selected split must contain rows.

```text
synth project preview project-preparation.toml
synth --output json project preview project-preparation.toml
synth project prepare project-preparation.toml
synth project show <PREPARATION_ID>
synth provenance project-preparation <PREPARATION_ID>
```

Preview is read-only at the domain level. It reports every generation cell and
target, initial/reserved totals, estimated initial requests, configured
backends, cohort counts, suite-local and all-cohort contamination status,
deterministic benchmark qualification support, policy issues, disclosures,
governance mode, and the finite stage graph. The all-cohort check
always uses zero tolerance even when the manifest gives suite-local nonzero
limits. UUIDs and timestamps generated only during artifact creation are
intentionally absent, so repeated previews are stable.

Prepare reruns the same validation and inserts one bundle in one SQLite
transaction: dataset, default configuration plan, non-secret backend settings,
resolved project configuration, cohorts and roles, contamination reports,
benchmark suites, immutable benchmark bundle, deterministic benchmark
qualification, explicit approval review, workflow definition, and preparation
summary. The bundle pins the development and optional sealed suite
IDs/fingerprints and the clean global report ID/fingerprint; both the definition
and preparation summary pin that bundle and the approved qualification/review
pair. A failure at any point rolls back the whole transaction. The stable
manifest fingerprint is unique; repeating the same manifest returns the
existing preparation even though a fresh compilation would otherwise generate
new artifact UUIDs.

Preparation rejects missing or changed snapshots, duplicate snapshot/split
assignments, label-order mismatches, empty selected splits, incompatible cohort
origins/roles, unsafe sealed disclosure, invalid budgets, infeasible initial
allocation, and any source-row, exact-text, normalized-text, or configured-group
overlap between any pair of cohorts in the combined development and optional
sealed set. The global report must cover exactly that set and cannot be
authorized by a contamination override. It never reads API keys and never starts
generation, training, evaluation, or sealed acceptance.

To execute the current complete workflow, every development cohort must pin
`row_content` disclosure and `adaptation_eligible = true`: row content is needed
by diagnosis, while the lower `predictions` and `slices` requirements are then
also covered. Preparation rejects a narrower or adaptation-ineligible
development policy before persisting any artifacts, and workflow runtime
rechecks the same core access policy before evidence work. Sealed cohorts remain
aggregate-only and adaptation-ineligible.

The successful command prints the separate authorization needed to execute:

```text
synth workflow start <DEFINITION_ID>
```

The generated definition uses the normal workflow contracts. Later code can
replace this manifest compiler without changing any slice implementation or
historical artifact.

With `[workflow.generation_supervision]`, `workflow start` or `workflow resume`
advances only to the next persisted supervisor boundary. If quality pauses,
the returned status gives the exact `supervisor issues`, diagnosis,
review/authorization, or canary command needed. A resume before that fact
changes is an idempotent no-op. Completion still pauses for an explicit
`quality manifest-review`; neither preparation nor the supervisor approves a
training snapshot. Do not configure `[workflow.quality_gate]` in the same
manifest.
