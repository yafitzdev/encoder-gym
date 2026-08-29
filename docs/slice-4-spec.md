# Slice 4 product specification — Evaluation

## Objective

Evaluate an immutable model checkpoint against a chosen snapshot split and
persist enough prediction evidence for later error analysis.

The application must let a user:

1. Choose a checkpoint, snapshot, and split.
2. Run inference through a project-owned `Predictor` port.
3. Persist one prediction per evaluated snapshot member.
4. Inspect accuracy, macro precision/recall/F1, per-label metrics, and a
   confusion matrix.
5. Inspect metrics grouped by arbitrary dataset dimensions.
6. Compare evaluation runs that use compatible label sets.

Evaluation never retrains a model and never changes snapshot membership.
Metrics are recomputable from persisted predictions.

The controlled-workflow phase groups compatible evaluation cohorts into
immutable benchmark suites with deterministic acceptance contracts. Every run
records its cohort role, purpose, disclosure level, and exposure. Development
evidence may drive later analysis; sealed-acceptance evidence is separately
authorized, aggregate-only by default, and ineligible for analysis,
optimization, advisor prompts, or workflow iteration.

Benchmark acceptance produces structured `pass`, `fail`, `inconclusive`, or
`invalid` results from persisted metrics and comparison evidence. An advisory
LLM interpretation cannot override that result.

The Slice 4.1 protocol, statistical comparison, and advisory selection workflow
is documented in [`evaluation-model-selection.md`](evaluation-model-selection.md).

## Test priorities

- exact confusion-matrix and metric calculations including zero denominators
- deterministic association of predictions with snapshot members
- split isolation
- per-label and per-dimension grouping
- predictor adapter normalization
- SQLite round trips

## Completion criterion

Slice 4 is complete when a checkpoint can be evaluated locally through the CLI
and all displayed metrics can be derived again from persisted prediction facts.
API and graphical UI work is deferred.
