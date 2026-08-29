# Slice 2 product specification — Dataset Management

## Objective

Turn accepted generated rows into immutable, reproducible dataset snapshots
that later slices can consume without depending on generation jobs or UI state.

The application must let a user:

1. Create a snapshot from accepted rows belonging to one dataset definition.
2. Record a human-readable name and optional description.
3. Select rows deterministically and retain every source row ID.
4. Assign every selected row to `train`, `validation`, or `test`.
5. Configure split ratios and a deterministic seed.
6. Preserve label and arbitrary categorical dimensions.
7. Inspect snapshot totals, split counts, label counts, and cell coverage.
8. List snapshot members and their source provenance.
9. Export a snapshot as JSONL or CSV with split assignments.

## Snapshot rules

- Completed snapshots are immutable.
- A source row appears at most once in a snapshot.
- Only accepted rows may become snapshot members.
- Split ratios are non-negative, sum to one within numeric tolerance, and
  produce deterministic assignments for the same seed and source rows.
- Splitting is stratified by label. Small strata use deterministic largest-
  remainder allocation so every row is assigned exactly once.
- Snapshot statistics come from persisted members, never cached frontend state.

## Architecture

`dataset-core` owns snapshot domain objects, deterministic splitting,
statistics, export, and persistence ports. SQLite implements those ports. The
slice consumes source rows only through a narrow accepted-row source port; it
does not depend on the generation job runner or backend adapters.

## Test priorities

- stable split assignment regardless of input ordering
- exact row conservation and no duplicate membership
- stratification and small-stratum rounding
- rejection of invalid ratios and non-accepted rows
- statistics derived from members
- JSONL/CSV export with split and provenance
- SQLite round trips

## Completion criterion

Slice 2 is complete when a persisted immutable snapshot can be created from
Slice 1 accepted rows, inspected, deterministically split, and exported through
the CLI. API and graphical UI work is deferred.
