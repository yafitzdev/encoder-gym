# Actionable error analysis

Error analysis converts immutable evaluation predictions into prioritized facts
for a human decision. It never loads a checkpoint, reruns inference, edits a
dataset, or automatically starts optimization.

## Reading a finding

`support` is the number of evaluated examples belonging to the finding.
`error_count` is the incorrect subset, and `error_rate` is
`error_count / support`. `error_rate_lift` subtracts the evaluation-wide error
rate; a positive value means the group is worse than baseline. `error_share` is
the fraction of all evaluation errors represented by the group.

Confidence is the selected label's persisted probability. Margin is the top
probability minus the second-highest probability. Entropy is natural-log
categorical entropy, with zero-probability terms contributing zero. Expected
probability is the probability assigned to the ground-truth label. The median
error confidence is a deterministic 101-bucket (0.01 resolution) approximation
so aggregation stays bounded. Persisted probability vectors must be complete,
finite, unique by label, sum to one within `1e-6`, and agree with the selected
label/confidence; analysis fails instead of silently renormalizing them.

Minimum support filters findings, not source predictions. Findings with no
errors are not ranked, but correct examples from the same identity may be kept
as bounded contrast evidence. Numerical means for an empty error set are
defined as zero. Published floating-point summaries are rounded to 12 decimal
places. Ranking uses the selected metric descending, then error count
descending, then canonical finding key ascending.

## Overlap and evidence

One error can belong to a label, confusion pair, dimension, full cell,
intersection, and confidence finding. Their raw counts are therefore not
additive. A second bounded pass assigns each error to the earliest ranked
matching retained finding. `marginal_error_count` is newly covered errors;
`cumulative_error_coverage` is the unique covered share through that rank.

Each finding keeps at most the configured number of deterministic evidence
links: highest-confidence error, lowest-margin error, an example near median
confidence, stable seeded errors, and optional correct contrasts. Links retain
prediction, snapshot-member, and source-row IDs. The CLI resolves full text,
probabilities, dimensions, and provenance from persisted predictions when
showing or exporting evidence.

## Workflow

```text
synth analysis create <RUN_ID> --minimum-support 20 \
  --ranking error-rate-lift \
  --confidence-threshold 0.5,0.8 \
  --dimension-intersection difficulty,style \
  --maximum-examples 10

synth analysis findings <REPORT_ID> --kind confusion-pair --limit 50
synth analysis finding <REPORT_ID> '<CANONICAL_FINDING_KEY>'
synth analysis evidence <REPORT_ID> '<KEY>' --format csv --file evidence.csv
synth analysis high-confidence-errors <REPORT_ID>
synth analysis weak-cells <REPORT_ID>
synth analysis comparison-group <REPORT_ID> persistent
synth analysis review <REPORT_ID> '<KEY>' --state candidate-for-label-schema-review
synth analysis reviews --report-id <REPORT_ID>
```

Finding keys are canonical JSON and cannot collide when dimension names or
values contain punctuation, quotes, separators, or Unicode. Quote the key in
your shell.

For comparison-aware diagnosis, first persist a Slice 4.1 paired comparison,
then supply it explicitly:

```text
synth evaluation compare <LEFT_RUN_ID> <RIGHT_RUN_ID>
synth analysis create <RUN_ID> --comparison-id <COMPARISON_ID>
```

The comparison must contain the analyzed run and reproduce the same cohort and
evaluation-protocol fingerprints. Analysis pages paired predictions and reports
fixed-by-right, regressed-by-right, persistent-error, and high-confidence
regression counts plus bounded examples and slice deltas. The stable diagnostic
contract additionally exposes the paired counts per canonical full cell so
optimization never infers them from presentation rows. Analysis does not rerun
either checkpoint.

Review history is append-only and outside the report fingerprint. Marking a
finding acknowledged, accepted, a candidate for more data/schema review, or
resolved does not rewrite historical evidence. Optimization remains an
explicit separate command and consumes only the stable analysis-owned cell
diagnostic contract.
