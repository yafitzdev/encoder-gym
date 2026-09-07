# Local CLI operations

## Output and exit behavior

Every command accepts global `--output human|json`. Human output is the default;
JSON mode writes exactly one JSON value to stdout for scripting. Progress,
cancellation notices, and startup recovery notices use stderr. Failures return
a non-zero exit code.

File-producing commands use `--file` so global `--output` is unambiguous:

```text
synth --output json export <DATASET_ID> --format jsonl --file data.jsonl
synth snapshot export <SNAPSHOT_ID> --format csv --file snapshot.csv
```

Dataset export streams bounded pages from the unified accepted source table, so
it contains both imported and generated rows with provenance.

## Import

JSONL paths may use dotted fields. CSV uses header names.

```text
synth dataset import <DATASET_ID> --input support.jsonl --format jsonl \
  --text-field message --label-field category \
  --dimension difficulty=metadata.difficulty \
  --dimension style=metadata.style --dry-run
```

Remove `--dry-run` to persist in bounded transactions. Use `dataset imports`,
`dataset import-show`, and `dataset import-rows` for progress and rejection
details.

## Filtering and pagination

List commands accept `--limit`, `--offset`, and `--summary`; filters are applied
in SQLite before pagination. Examples:

```text
synth dataset list --name support --limit 20 --summary
synth job list --dataset-id <ID> --state failed --summary
synth snapshot list --dataset-id <ID>
synth training list --snapshot-id <ID> --state completed
synth evaluation list --checkpoint-id <ID> --state completed
synth analysis list --evaluation-run-id <ID>
synth analysis findings <REPORT_ID> --kind confusion-pair --limit 20
synth optimize list --dataset-id <ID>
synth optimize recommendations <ID> --kind data-generation --eligible --limit 20
synth rows --dataset-id <ID> --status rejected --summary
```

## Diagnostics

```text
synth doctor --config project.toml
synth doctor --config project.toml --check-backend
```

Doctor checks SQLite integrity, foreign keys, migration success, strict config
validation/fingerprint, the training artifact path, registered encoder bundle
identity, every persisted checkpoint size/checksum, completed evaluation facts,
analysis protocol/report fingerprints, normalized finding order and coverage,
evidence ownership, review references, optimization evidence, scores,
allocations, normalized rows, approvals/applications, training candidates,
campaign links, outcome fingerprints, and optional generation backend
reachability. Connectivity output reports only the safe endpoint origin
and HTTP status; credentials are never printed.

The workflow integrity checks also validate benchmark bundles and their suite,
strict-contamination, cohort, and pinned-role evidence. Legacy null bundle links
are retained for inspection, not treated as executable authority.

## Governed workflow operation

Run one local workflow process at a time. `workflow start` and `resume` hold a
process-identity lease and reject concurrent drivers. Use `workflow status` for
persisted attempts, usage, generation jobs, and coverage, or `workflow watch`
from another terminal. Ctrl+C may interrupt the process; startup reconciliation
then exposes the run through `recovery list`, and `workflow resume` continues
from durable facts. `workflow cancel` is the durable cancellation path and
forwards cancellation to an active generation job between bounded batches.

Review mode deliberately stops at `awaiting_approval`; inspect the proposal and
approve exact recommendation IDs before more rows or training are authorized.
Preauthorized mode is still finite and records every envelope decision. A
development-complete workflow is only a candidate. Run `workflow finalize`
explicitly to disclose the configured sealed aggregate, then `workflow promote`
to record promotion, rejection, or inconclusive evidence. Never use sealed
row-level inspection to tune a candidate without first retiring/demoting that
cohort through the exposure workflow.

Execution rechecks that every bundle-pinned cohort role decision is still the
current active decision before each stage does evidence work. A transition or
retirement deliberately invalidates that definition for further execution;
derive new suites, a bundle, and a workflow definition from the new eligible
roles. Pre-`0046` workflows without a bundle remain inspectable but cannot be
started or resumed.

## Error analysis

```text
synth analysis create <RUN_ID> --minimum-support 20 \
  --ranking high-confidence-error \
  --dimension-intersection difficulty,style
synth analysis findings <REPORT_ID> --kind cell --limit 25
synth analysis finding <REPORT_ID> '<CANONICAL_FINDING_KEY>'
synth analysis evidence <REPORT_ID> '<KEY>' --format jsonl --file evidence.jsonl
synth analysis review <REPORT_ID> '<KEY>' --state candidate-for-more-data
```

Add `--comparison-id <COMPARISON_ID>` to consume an existing paired comparison.
Analysis reads persisted predictions twice in bounded pages and inserts the
finished report, findings, and evidence links in one transaction. It never
loads a checkpoint or runs inference. See `error-analysis.md`.

## Advisory optimization

```text
synth optimize preview <REPORT_ID> --protocol examples/optimization.toml
synth optimize scenarios <REPORT_ID> --protocol examples/optimization.toml
synth optimize propose <REPORT_ID> --protocol examples/optimization.toml
synth optimize review <PROPOSAL_ID> --state approved-for-plan-creation
synth optimize apply <PROPOSAL_ID> --approval-id <REVIEW_ID>
```

Preview is read-only. Propose persists one immutable decision. Apply is
transactional and idempotent, creates a generation plan, and never starts it.
Use `optimize rebase` after explicit stale-coverage rejection. Campaign links
only record compatible artifacts already created through normal commands. See
`optimization.md`.

## Local BERT training

```text
synth encoder register --name support-bert C:\models\support-bert
synth encoder verify <ENCODER_ID>
synth training run <SNAPSHOT_ID> --backend bert-cpu --encoder-id <ENCODER_ID>
synth training status <RUN_ID>
synth training checkpoints <RUN_ID>
synth training predict <CHECKPOINT_ID> --text "Why was I charged twice?"
synth training continue <CHECKPOINT_ID> --epochs 2
synth evaluation run <CHECKPOINT_ID> --split test
```

Progress is emitted on stderr, so JSON stdout stays composable. Ctrl+C requests
cancellation between bounded batches. See `docs/transformer-training.md` for
the exact supported bundle and configuration.

## Production encoder optimization

Use a dedicated SQLite database inside a clean, remote-free isolated task
checkout. Preview and start from one strict manifest, then follow the exact next
command printed by status:

```text
synth encoder optimize preview --manifest optimize.toml --workspace <COPY>
synth encoder optimize start --manifest optimize.toml --workspace <COPY>
synth encoder optimize status <RUN_ID> --workspace <COPY>
synth encoder optimize resume <RUN_ID> --workspace <COPY>
synth encoder optimize report <RUN_ID> --workspace <COPY>
synth encoder optimize provenance <RUN_ID> --workspace <COPY>
synth encoder optimize doctor <RUN_ID> --workspace <COPY>
```

Each `resume` performs at most one stage. Do not script repeated resume without
checking status, because an eligible candidate deliberately pauses for
`authorize-sealed`. Routine status uses shallow immutable-envelope checks;
Doctor is slower because it replays native data and project artifacts. See
`docs/encoder-optimize.md` and `docs/current-status.md`.

## Provenance

```text
synth provenance optimization-proposal <ID>
synth provenance optimization-proposal-review <ID>
synth provenance optimization-campaign <ID>
synth provenance optimization-outcome <ID>
synth provenance checkpoint <ID>
```

The JSON/human tree follows persisted dependency IDs through configuration,
dataset, plan/import/generation source, snapshot, training, checkpoint,
evaluation, analysis, and optimization artifacts. Fingerprints and checkpoint
checksums are included where applicable. Transformer checkpoint traces include
the registered base model and, for continuation runs, the parent checkpoint.
