# ADR 002: Durable evaluation evidence

Evaluation owns a normalized protocol and source identity in `evaluation-core`.
SQLite implements paged example access and atomic prediction/progress commits.
Immutable paired-comparison and model-selection reports reference completed
evaluation runs through explicit persistence and provenance edges.

Runs are bounded by batch size rather than cohort size. Interrupted and
cancelled runs retain diagnostic facts but cannot feed downstream decisions.
Comparison validity is strict; incompatible runs must be evaluated again under
a common protocol.
