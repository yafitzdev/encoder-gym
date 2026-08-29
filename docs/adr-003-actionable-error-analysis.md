# ADR 003 — Bounded, normalized actionable error analysis

## Status

Accepted.

## Decision

Analysis owns a normalized immutable protocol and a read-only evaluation
evidence port. The application runner validates completed evaluation and
optional comparison identities, pages predictions in deterministic order,
aggregates group state with bounded representative pools, and performs a second
bounded pass for exact overlap coverage.

SQLite stores the report, ranked findings, and prediction evidence links in one
transaction. Full prediction payloads remain in the evaluation tables.
Append-only review records are separate from immutable report identity.
Optimization consumes `DiagnosticContract`, an analysis-owned projection of
validated cell evidence.

## Consequences

Memory scales with observed finding groups and configured evidence limits, not
prediction count. Analysis can be replaced or tested without SQLite, and it
cannot silently rerun inference. The second pass trades additional local reads
for exact non-additive coverage. Median confidence uses a documented fixed
histogram approximation. Existing report JSON remains decodable through serde
defaults; new reports also have normalized queryable rows.
