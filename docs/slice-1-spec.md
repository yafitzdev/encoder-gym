# Slice 1 product specification

## Objective

Build a useful standalone local application for explicit synthetic-data
generation, optimized initially for text classification.

The application must let a user:

1. Create a dataset definition.
2. Define arbitrary categorical dimensions.
3. Inspect the Cartesian generation cells.
4. Assign a target count to every cell.
5. Configure an LLM generation backend.
6. Start and stop generation.
7. Watch durable progress.
8. Reject invalid generated examples.
9. Inspect accepted and rejected rows.
10. Inspect coverage for every cell.
11. Export the accepted dataset.

## Dataset shape

The initial output is text-classification data:

```json
{
  "text": "Why was I charged twice?",
  "label": "billing",
  "dimensions": {
    "difficulty": "easy",
    "writing_style": "clean",
    "ambiguity": "obvious"
  }
}
```

Dimension names and values are user-defined categorical data. Example names in
this specification must never be hard-coded.

The complete generation-space cardinality must be inspectable without
materializing every cell. The local application rejects schemas above its
explicit cell safety limit before Cartesian expansion.

Optional semantic profiles may enrich labels, dimensions, and dimension values
with descriptions, examples, counterexamples, and inclusion/exclusion rules.
Profiles are immutable and versioned; reusable and dataset-specific layers are
attached explicitly and resolved deterministically. Dataset definitions remain
valid and independently useful without profiles. Generation jobs pin the exact
resolved profile versions used by prompt construction.

## Explicit coverage

Generation is cell-based rather than random. Labels and dimension values form a
deterministic Cartesian product. Every planned cell has its own target count.

The first interface may offer an equal-target shortcut, but the underlying plan
must represent unequal targets without changing its model.

The controlled-workflow phase adds a separate pure initial-budget allocator. It
may convert one exact accepted-row total and a balanced, weighted, constrained,
or explicit policy into absolute Slice 1 cell targets. It must preview and then
create an ordinary unequal generation plan; it does not change the Slice 1 plan
or backend contracts.

Allocation constraints may be expressed as exact cells or compiled from
validated partial label/dimension selectors. Persisted allocation facts retain
the resulting exact per-cell bounds. Operator-facing explanations summarize
realized targets and effective weights across labels and every dimension value.

For every cell, persisted facts must support reporting:

- target
- attempted
- accepted
- rejected
- remaining

## Required components

### Domain and dimension engine

Represent only the concepts needed by this slice. Expand dimension definitions
into deterministic combinations without LLM or persistence dependencies.

### Generation planner

Calculate the work needed per explicit cell from targets and existing accepted
coverage. Do not assume all targets are equal.

### Prompt construction

Convert the dataset definition, task, cell, requested row count, generation
parameters, and optional examples into a normalized generation request. Prompt
templates must be inspectable and must not live in provider clients.

### Generation backend port

The application owns a provider-neutral `GenerationBackend` contract. A backend
receives a normalized request and returns normalized rows, usage information,
metadata, and errors. It does not plan work, store rows, calculate coverage, or
manage UI state.

Slice 1 requires a deterministic fake backend and one working
OpenAI-compatible backend. A future NVIDIA adapter may implement the same port,
but NVIDIA-specific types must not escape its adapter.

"Plugin" means a replaceable adapter registered by the application. Runtime
dynamic-library loading is not required.

### Parsing, validation, and deduplication

Generated output passes through parsing and small composable validators before
acceptance. Initial checks cover structural validity, non-empty text, valid
labels, correct dimension values, optional text-length boundaries, and exact or
normalized-text duplication. Semantic deduplication is out of scope.

### Persistence

SQLite stores dataset definitions, dimensions, plans, jobs, generated rows,
validation outcomes, coverage facts, generation metadata, immutable execution
specifications, and one append-only record per provider request. Accepted and
rejected rows retain provenance including backend, model, job identifier,
request-attempt identifier, and creation time. Raw API credentials are never
persisted.

The execution specification pins initial per-cell needs, backend/model and
non-secret endpoint identity, generation parameters, batching/retry limits,
prompt-template identity, and semantic-context fingerprint before the first
provider call. Job creation, execution specification, and semantic assignment
are one transaction.

Sophisticated dataset versioning is not part of this slice.

### Job runner

One local worker generates manageable batches with reasonable retries. A failed
request does not destroy the whole job. Jobs support `queued`, `running`,
`completed`, `failed`, and `cancelled` states and durable progress counters.

Each provider request is persisted as `started` before network I/O and becomes
`succeeded`, `failed`, or `interrupted`. A successful attempt, its generated
rows, accepted source rows, and reconciled job counters commit atomically.
Retries count against a persisted per-cell row-attempt ceiling. Transport,
timeout, rate-limit, and server failures may retry with bounded backoff;
configuration, permanent HTTP rejection, and invalid-response failures do not.
Normalized-text claims in SQLite prevent two local processes from accepting a
new duplicate while preserving any historical duplicates during migration.

The CLI may execute a job in the foreground while persisting its state. The
later HTTP process runs the same application job runner without blocking an HTTP
request. No daemon, distributed queue, or multiple-worker coordination is
required.

### Interfaces

The CLI is the first complete interface. A later minimal HTTP API and graphical
control surface call the same application use cases. The UI remains a primitive
developer tool made from forms, buttons, tables, and progress indicators.

### Export

Export accepted rows while retaining their useful dimensions and provenance.
JSONL and CSV are the initial formats.

## Test priorities

Prioritize deterministic tests for:

- dimension combination generation
- unequal generation planning
- coverage calculations
- backend-output parsing
- validation composition
- duplicate detection
- backend response normalization
- failure, retry, and cancellation behavior
- crash interruption, atomic attempt/row commits, and cumulative attempt limits
- execution/prompt fingerprint reproduction and migration compatibility

Most tests use the fake backend and require no network access.

## Explicit non-goals

Do not implement:

- encoder training or model checkpoints
- evaluation, benchmarks, or error analysis
- optimization loops, optimizer agents, bandits, or reinforcement learning
- dataset version trees
- authentication or multi-user collaboration
- cloud deployment or distributed execution
- multiple workers
- advanced visualizations
- embedding or semantic deduplication

## Completion criterion

Slice 1 is complete when every objective above works locally through the final
control surface and the generation provider can be replaced through the
project-owned backend port without modifying planning, prompting, validation,
persistence, coverage, export, CLI, API, or UI code.
