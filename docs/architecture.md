# Platform architecture

## Dependency direction

```text
generation-cli ───────────────┐
generation-server ─────────────┼──> generation-core
generation-sqlite ────────────┤
generation-fake ───────────────┤
generation-openai-compatible ─┘

generation-core ──> no infrastructure or presentation crate
```

Later slices follow the same inward dependency rule:

```text
apps ───────────────────────────────────────────────────────────┐
SQLite and model adapters ──────────────────────────────────────┤
                                                                ↓
dataset-core → training-core → evaluation-core → analysis-core → optimization-core
```

Cross-cutting contracts remain small and inward-facing:

```text
apps / SQLite adapter ──> project-config
apps / SQLite adapter ──> recovery-core
slice cores / adapters ──> artifact-core
dataset-import ─────────> dataset-core + generation-core
```

`artifact-core` only canonicalizes fingerprint inputs and describes provenance
trees. `recovery-core` only describes process leases and interruption records.
Neither crate orchestrates slice business logic.

The arrows between core crates describe artifact consumption, not access to
another slice's implementation. Training consumes normalized snapshot examples;
it does not query generation jobs.

Each slice core owns its domain models, pure logic, application workflows, and
the ports required from infrastructure. Adapter crates implement those ports.
Executable applications assemble concrete adapters and invoke core workflows.

Dependencies must point inward. Core code must never import an adapter or
application executable.

## Workspace members

### `synthetic-data-core`

Owns Slice 1 business rules and project-owned interfaces. Its modules are
organized by a clear reason to change rather than by generic technical labels.

### `synthetic-data-sqlite`

Implements persistence ports with SQLite and owns migrations and SQL-specific
mapping. It must not decide what should be generated.

### `synthetic-data-openai-compatible`

Implements the generation-backend port through an OpenAI-compatible HTTP API.
It owns provider transport and response normalization, but not prompt policy,
validation, coverage, or persistence.

### `synthetic-data-test-support`

Re-exports deterministic test adapters and provides fixtures shared by
integration tests. It is not part of production decision-making.

### `synthetic-data-fake`

Implements the backend port with deterministic local rows. It is a production
adapter so the CLI and server can exercise every workflow without network
access, and it remains useful as a test dependency.

### `synthetic-data-cli`

Parses terminal input, invokes application workflows, renders results and
progress, and converts process signals into cancellation requests. Business
logic does not belong here.

### `synthetic-data-server`

Exposes the same application workflows through a small Axum API and serves the
primitive static developer UI. It assembles one in-process worker guarded by a
single permit; it does not contain business rules or provider-specific types.
This executable remains a Slice 1 surface while later slices are developed
through their cores and the CLI.

### `dataset-core`

Owns immutable snapshot models, deterministic stratified split assignment,
snapshot statistics/export, and accepted-row/source and snapshot-store ports.
It knows no generation backend or SQLite type.

### `dataset-import`

Streams JSONL and CSV records through configurable field mapping and composable
validation. It is a format adapter; accepted/rejected row ownership and import
provenance remain in `dataset-core`, and SQLite batching remains in the
persistence adapter.

### `project-config`

Owns strict, versioned TOML shapes, defaults, CLI override resolution, domain
validation, and deterministic configuration fingerprints. It stores only the
name of an API-key environment variable, never the key itself.

### `artifact-core` and `recovery-core`

`artifact-core` provides canonical SHA-256 fingerprints and the provenance port.
`recovery-core` provides workflow-kind, process-lease, interruption, and
resolution contracts. SQLite implements both without leaking SQL or process
inspection into slice domains.

### `training-core`, `training-linear`, and `training-transformer`

`training-core` owns run/checkpoint models, the trainer and predictor ports, and
the durable runner. Its example-source contract permits bounded reads without
coupling a backend to SQLite or a filesystem format. `training-linear` is the
replaceable deterministic CPU adapter and owns the local immutable checkpoint
store. `training-transformer` is the Candle/Tokenizers adapter for the exact
local BERT bundle described in ADR 001. Candle tensors, tokenizer encodings,
BERT configuration types, and optimizer internals do not cross into core.

The CLI pages immutable snapshot members into an ephemeral local spool. The
transformer backend keeps member IDs for deterministic ordering and loads only
the active text batch; it never holds the complete tokenized dataset.

### `evaluation-core`

Owns persisted evaluation runs and predictions, exact classification metrics,
dimension slicing, comparisons, and the predictor-driven runner.

### `analysis-core`

Owns the normalized analysis protocol, collision-safe finding identities,
bounded streaming aggregation, uncertainty summaries, overlap-aware ranking,
paired-comparison diagnosis, immutable reports, and the read ports it needs
from persisted evaluation evidence. Its first pass uses memory proportional to
observed groups and configured evidence limits; a second bounded pass assigns
each error to its earliest ranked finding for exact marginal and cumulative
coverage. It has no model or SQLite dependency.

The project-owned `DiagnosticContract` is the only analysis shape Slice 6 needs
for cell recommendations. It carries canonical cell identities, complete
bounded metrics, the latest append-only review disposition, evaluation and
comparison identity, and reproducible per-cell and whole-contract
fingerprints. Optimization does not consume SQL rows, CLI types, or
aggregation internals.

### `optimization-core`

Owns the immutable decision protocol/evidence envelope, auditable scoring,
finite constrained allocation, normalized data/review recommendations, typed
bounded training candidates, scenario comparison, append-only review rules,
safe plan conversion, campaign compatibility, and deterministic outcome
assessment. It is advisory: persistence and explicit CLI application are
separate from generation and training execution.

Slice 6.1 decisions begin with a canonical, validated optimization protocol.
Every new proposal embeds the resolved protocol and its fingerprint so later
scoring, allocation, approval, and application behavior cannot drift with
mutable defaults. Historical proposal payloads remain explicit legacy
artifacts without a protocol. See [ADR 004](adr-004-immutable-optimization-protocol.md).

Before scoring, `OptimizationEvidence` binds that diagnostic contract to the
immutable dataset definition, source snapshot, canonical accepted-cell
coverage, optional comparison, optimization protocol, and, when requested,
the exact training-configuration space. This optimization-owned envelope is
fingerprinted and contains no persistence or presentation types.

`optimization-core` consumes narrow artifact shapes from generation, dataset,
training, evaluation, and analysis cores. It does not import their runners,
SQLite adapters, CLI parsing, backend implementations, or provider types. A
training configuration space is bound to a completed run, final checkpoint,
snapshot, and supported registered backend. Campaigns only validate and link
already-created artifact identities; they cannot schedule or poll workflows.

SQLite stores the complete immutable JSON artifacts plus normalized protocol,
coverage, decision, recommendation, training-candidate, review, application,
scenario, campaign-link, and outcome relationships needed for filtering and
integrity audits. Decision-grade application atomically inserts the ordinary
generation plan and its application marker.

## Durability and provenance

Generation, training, and evaluation runners register a lease containing the
owning PID and operating-system process start time. Startup reconciliation only
marks a `running` workflow interrupted when that exact process identity is no
longer alive. Generation is eligible for in-place resume because planning reads
persisted accepted coverage. Training and evaluation preserve interrupted
history and are not silently replayed under the same artifact identity.

Snapshots, registered base-model bundles, resolved configurations, evaluation
inputs, analysis protocols/reports, optimization evidence/proposals, reviews,
campaign links, and outcomes have deterministic SHA-256 fingerprints. Floating
point inputs are normalized through their persisted JSON representation before
hashing so identities reproduce after reload. Checkpoint
bytes are checksum-verified before every production load. Provenance traces are
built from persisted foreign keys and source-row provenance, not presentation
state.

Analysis findings and representative prediction links are normalized in
SQLite. Human finding reviews are append-only children; changing review state
never changes or refingerprints the immutable report.

Transformer checkpoints are atomically written, immutable single-file
containers with a JSON manifest plus safetensors for model and AdamW state.
Continuation creates a new run linked to its parent checkpoint; interrupted
runs are never silently resumed under the same identity.

## Interface strategy

Rust traits define replaceability boundaries for generation and persistence.
They are application ports, not provider abstractions copied from external
SDKs. Async ports must remain usable behind `Arc<dyn Trait>`; use an explicitly
boxed future or a narrowly contained compatibility helper if native async trait
syntax is not dyn-compatible.

Dynamic-library plugin loading is not required. Generation, training, and
prediction implementations are adapter crates selected during application
composition.

## UI strategy

The existing graphical UI covers Slice 1 only. Slices 2–6 remain CLI-only until
the user explicitly starts a separate UI phase. Any future UI communicates only
with an API and never imports or reimplements core rules.
