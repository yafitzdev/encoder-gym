# Evaluation and model selection

Evaluation is a CLI-only evidence workflow. Each run freezes its split, batch
size, metric definitions, statistical settings, ordered labels, checkpoint
identity, snapshot identity, and cohort fingerprint. Inference is paged and
each completed batch commits predictions and progress atomically.

```text
synth evaluation run <CHECKPOINT_ID> --split test --batch-size 32 --top-k 1,2
synth evaluation status <RUN_ID>
synth evaluation metrics <RUN_ID>
synth evaluation predictions <RUN_ID> --incorrect --expected-label billing --limit 50
synth evaluation export <RUN_ID> --format jsonl --file predictions.jsonl
synth evaluation cancel <RUN_ID>
```

Partial predictions remain inspectable, but comparison, analysis, export, and
selection accept only completed runs.

## Paired evidence

Paired comparisons require the identical snapshot, split, member cohort,
ordered labels, and metric protocol. This makes fixed and regressed examples,
paired bootstrap intervals, and exact McNemar results meaningful. A positive
point estimate alone is not statistical significance: inspect whether the
confidence interval crosses zero and whether the McNemar result meets the
protocol threshold.

Log loss penalizes low probability on the correct class. Multiclass Brier
score measures squared probability error. Expected calibration error compares
confidence with observed correctness; lower values are better for all three.
Minimum slice support suppresses weak slices from ranked summaries without
deleting their prediction facts.

## Advisory selection

```text
synth evaluation compare <LEFT_RUN_ID> <RIGHT_RUN_ID>
synth evaluation leaderboard --snapshot-id <ID> --split test --metric macro-f1
synth evaluation select --snapshot-id <ID> --split test --metric macro-f1 --minimum-improvement 0.01
synth provenance trace model-selection <SELECTION_ID>
```

Leaderboards never mix cohorts or protocol families. Ties are deterministic.
Selection reports are immutable and advisory: they never deploy, copy, or
mutate a checkpoint.
