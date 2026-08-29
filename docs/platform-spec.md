# Encoder-development platform specification

## Product objective

Build a local, single-user platform that turns explicit synthetic-data plans
into reproducible text-classification experiments:

```text
Data Generation
      ↓
Dataset Management
      ↓
Encoder Training
      ↓
Evaluation
      ↓
Error Analysis
      ↓
Optimization
```

Every arrow crosses an explicit, persisted contract. A later slice consumes
immutable artifacts from the earlier slice and never reaches into its internal
implementation.

## Shared invariants

- IDs are globally unique and timestamps use UTC.
- Dataset snapshots and model artifacts are immutable after completion.
- Derived records retain source IDs and configuration sufficient to reproduce
  the operation.
- User-visible counts and metrics are calculated from persisted facts.
- Backend contracts are owned by the platform, not copied from providers.
- A deterministic local implementation and fake are available for ordinary
  development and tests.
- Long-running operations use durable run states and never block an HTTP
  request.
- CLI workflows are complete before equivalent API and UI workflows are added.

## Slice definitions

- [Slice 1 — Synthetic Data Generation](slice-1-spec.md)
- [Slice 2 — Dataset Management](slice-2-spec.md)
- [Slice 3 — Encoder Training and Checkpoints](slice-3-spec.md)
- [Slice 4 — Evaluation](slice-4-spec.md)
- [Slice 5 — Error Analysis](slice-5-spec.md)
- [Slice 6 — Optimization](slice-6-spec.md)

## Current non-goals

The local platform does not include authentication, multi-user collaboration,
cloud deployment, distributed execution, multiple workers, autonomous agents,
bandits, reinforcement learning, automatic external spending, or semantic
deduplication. Optimization is deterministic recommendation logic, not an
unbounded autonomous loop.

## Platform completion criterion

The platform is complete when a user can generate labeled examples, freeze an
immutable and reproducibly split snapshot, train a replaceable local classifier
backend with checkpoints, evaluate a chosen checkpoint, inspect persisted
errors by label and arbitrary dimensions, obtain an explicit optimization
proposal, and turn that proposal into a new generation plan without rewriting
an earlier slice.
