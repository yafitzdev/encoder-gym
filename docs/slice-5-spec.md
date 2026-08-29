# Slice 5 product specification — Error Analysis

## Objective

Turn persisted evaluation predictions into inspectable, reproducible error
findings without rerunning inference or training.

The application must let a user:

1. Analyze one completed evaluation run.
2. List every misclassified example with text, expected label, predicted label,
   confidence, dimensions, and provenance.
3. Group errors by expected label, predicted label, confusion pair, and
   arbitrary dimension value.
4. Rank weak slices using explicit support and error-rate calculations.
5. Persist an immutable analysis report that references its evaluation run.
6. Export the report as JSON.

The implementation uses a normalized immutable protocol, deterministic bounded
aggregation, confidence/margin/entropy summaries, collision-safe categorical
identities, overlap-aware ranked coverage, bounded evidence links, optional
paired-comparison diagnosis, and append-only review history. Semantic
clustering, LLM-authored explanations, and embedding analysis are not required.

The controlled-workflow phase may optionally send bounded, policy-approved
development findings to a separate provider-neutral advisor. That advisor is
not part of Error Analysis and cannot change report contents or fingerprints.
Analysis creation and evidence export must reject sealed-acceptance evaluations.

## Test priorities

- correct grouping and ranking
- minimum-support behavior
- stable tie ordering
- report reproducibility from persisted predictions
- preservation of example provenance

## Completion criterion

Slice 5 is complete when a user can create and inspect a persisted report
through the CLI and trace every finding back to evaluation predictions and
original snapshot members. API and graphical UI work is deferred.
