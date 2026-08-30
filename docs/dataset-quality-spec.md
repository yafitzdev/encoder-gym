# Dataset Qualification and Curation specification

## Objective

Insert an independently useful quality boundary between structurally accepted
source rows and immutable training snapshots:

```text
generated or imported source rows
  -> structural validation
  -> immutable quality audit plan
  -> replaceable row-quality evaluator
  -> reproducible row assessments
  -> deterministic curation proposal
  -> append-only human review
  -> approved curation manifest
  -> qualified dataset snapshot
```

Structural acceptance means that a row is parseable, belongs to the expected
dataset, uses a known label and dimension values, and satisfies deterministic
construction rules. Qualification answers the separate question: whether the
row is trustworthy enough for the intended training use.

The capability belongs beside Dataset Management. It consumes immutable source
row facts through a narrow port and produces an immutable selection artifact.
It does not generate rows, rewrite labels, mutate source data, split snapshots,
train encoders, evaluate checkpoints, or decide benchmark acceptance.

## Initial scope

The first complete version audits text-classification training candidates. It
supports both generated and accepted imported rows through the normalized
`SourceRow` shape.

Every source row included by an approved V1 curation manifest must have either:

- a reproducible assessment whose deterministic policy verdict is `qualified`;
  or
- a current append-only human override that explicitly includes it with a
  reviewer and reason.

Unevaluated, invalid, borderline, and quarantined rows are excluded by default.
V1 performs a row-complete audit of the pinned source set. Stratified sampling
and inference to unevaluated rows are deferred until a separate policy can state
and validate its statistical assumptions. This conservative rule prevents a
sampling shortcut from silently weakening the quality gate.

The capability must never receive sealed-acceptance or external-benchmark row
contents. It qualifies training candidates before snapshot cohort roles are
assigned. A later benchmark-curation capability will own evaluation-data
quality without exposing sealed evidence to adaptive systems.

## Quality dimensions

The normalized evaluator response reports bounded integer scores and categorical
criterion outcomes rather than a provider-specific object or free-form verdict:

- assigned-label fidelity;
- a score for every competing label;
- adherence for every declared categorical dimension;
- optional authenticity adherence when an approved authenticity context is
  pinned;
- label-leakage risk;
- shortcut/artifact risk;
- evaluator confidence;
- bounded issue codes and a concise operator-facing rationale.

Scores use basis points from 0 through 10,000. Higher is better for fidelity,
adherence, authenticity, and confidence. Higher is worse for leakage and
shortcut risk. Integer scores avoid floating-point ambiguity in thresholds and
fingerprints.

The evaluator first receives the text, allowed labels and values, and their
approved semantics without being shown the row's assigned label or dimension
values. The host compares its blind ranking with the pinned source facts. This
reduces confirmation bias without pretending that the evaluator is ground
truth.

The core, not the evaluator, derives `qualified`, `borderline`, or
`quarantined` from the immutable policy. The evaluator may not choose whether a
row enters a snapshot.

An assessment is invalid unless it:

- belongs to the exact pinned source row and source fingerprint;
- scores exactly the known label vocabulary;
- scores exactly the row's declared dimensions;
- uses only bounded scores and known issue codes;
- contains no replacement text, replacement label, or replacement dimension;
- identifies the evaluator backend, model, protocol, and request attempt;
- reproduces its artifact fingerprint.

## Policy and operator controls

A strict `QualityPolicy` pins:

- minimum assigned-label score;
- minimum margin over the highest competing label;
- minimum dimension-adherence score;
- optional minimum authenticity score;
- maximum label-leakage and shortcut-risk scores;
- minimum evaluator confidence;
- the borderline margin around each threshold;
- handling of invalid evaluator output;
- finite batch, request-attempt, token, and optional cost limits.

Operator-facing presets may compile `fast`, `balanced`, and `strict` choices
into this explicit policy, but persisted runs always contain the resolved
thresholds and fingerprint. V1 presets may change batching and repeated review
of borderline rows; they do not authorize inclusion of unevaluated rows.

The useful outcome-oriented controls are quality strictness, precision versus
coverage, independent review of borderline rows, authenticity strictness, and
maximum audit cost. Provider temperatures and transport retry details remain
advanced configuration.

## Audit plan and source-set integrity

Planning loads accepted source rows through the Dataset Management port, sorts
them canonically, and creates an immutable source-set manifest. Every entry
pins the source row ID, complete source-row fingerprint, cell identity, and a
normalized provenance stratum such as import or generation job/backend/model.

The plan additionally pins:

- dataset definition ID and fingerprint;
- quality policy and fingerprint;
- optional resolved semantic-context identity and fingerprint;
- optional approved authenticity-context identity and fingerprint;
- evaluator protocol version;
- canonical source-set fingerprint;
- creation time and plan fingerprint.

The runner reloads each source row and verifies its fingerprint before sending
it to an evaluator. New accepted rows do not mutate an existing plan and are not
silently included. A new audit plan is required to cover them.

## Replaceable evaluator contract

`dataset-quality-core` owns a provider-neutral `QualityEvaluator` port. It
receives a bounded normalized batch containing the task, label and dimension
semantics, approved authenticity guidance, policy-relevant criteria, and exact
candidate rows. It returns normalized assessment drafts plus usage and backend
metadata.

The evaluator must not:

- query SQLite or read arbitrary files;
- generate or rewrite source rows;
- change labels or dimensions;
- choose the audit source set or thresholds;
- persist assessments or curation decisions;
- create snapshots, start training, or inspect evaluation evidence.

Provide a deterministic fake and one OpenAI-compatible adapter. Prompt policy
belongs in the quality capability rather than the HTTP adapter. A generator and
evaluator may use different backends or models; the application records whether
the assessment is independent from generation provenance.

Ordinary tests never call a paid model. Live-provider smoke testing remains
explicitly opt-in.

## Durable runner

One local runner processes the immutable plan in bounded batches. A run has
`queued`, `running`, `completed`, `failed`, and `cancelled` states and tracks:

- planned, assessed, qualified, borderline, quarantined, and invalid rows;
- completed and failed evaluator requests;
- input/output token usage and optional declared cost;
- cancellation, lease, retry, and interruption facts.

Each evaluator request is persisted as `started` before external I/O. Success,
normalized assessments, and reconciled run counters commit atomically. An
uncertain interrupted request consumes its attempt budget and is not silently
replayed under the same request identity. Configuration errors and malformed
responses are permanent; transport and rate-limit failures may retry within the
pinned finite policy.

Repeated assessments of a borderline row remain separate immutable evidence.
V1 may deterministically aggregate them by median score and conservative
worst-case risk. No model response may expand the request, token, or cost budget.

## Curation and review

After a completed audit, deterministic policy creates a curation proposal with
one entry for every pinned source row:

- `include` for a reproducible qualified assessment;
- `exclude` for quarantined, invalid, or unevaluated rows;
- `needs_review` for borderline or conflicting evidence.

Every entry records its assessment IDs, effective policy facts, decision basis,
and concise reasons. Aggregate summaries expose these counts for every label,
cell, provenance stratum, and issue code.

Row-level reviews are append-only and may confirm, include, exclude, or request
reassessment. An inclusion override requires a reviewer and reason and never
changes the original assessment. Recompiling after reviews creates a new
immutable curation proposal linked to its predecessor.

A manifest-level review is also append-only. Only the latest explicit approval
for the exact proposal may produce an approved curation manifest. Approval
cannot be inferred from evaluator confidence, a workflow state, or a previous
proposal.

## Qualified snapshots

Dataset Management remains the owner of splitting and immutable snapshots. A
qualified snapshot command loads an approved curation manifest, verifies its
complete provenance, selects only included source row IDs, reloads and verifies
those rows, and calls the ordinary deterministic snapshot builder.

An immutable curation application links the approved manifest, latest approval,
ordinary snapshot, and exact selected-member fingerprint in the same SQLite
transaction that persists the snapshot. `DatasetSnapshot` itself remains
unchanged so historical fingerprints reproduce. Snapshot provenance exposes
the application and manifest. Snapshots without a curation application remain
valid and are visibly `unqualified`; governed workflows may require
qualification through an explicit policy.

Coverage distinguishes structural acceptance from qualification:

```text
target / generated / structurally accepted / assessed / qualified /
borderline / quarantined / remaining-qualified
```

The Dataset Architect may later consume only approved qualified-coverage facts
when the operator requests a quality-aware replacement plan. It may recommend
new rows but cannot invoke the quality evaluator or approve curation.

## Workflow integration

Workflow definitions gain an optional immutable quality-gate configuration.
Existing definitions without it retain their current behavior. When required,
the legal stage order becomes:

```text
generation -> quality audit -> curation review -> qualified snapshot -> training
```

The same gate applies to dataset-diff generation before an iteration snapshot.
The workflow may start and observe an already configured audit runner, but it
cannot decide row scores, copy evaluator logic, infer curation approval, or
continue while required review is unresolved.

Quality assessment is adaptive training-data processing. It cannot inspect
sealed evaluation rows, predictions, findings, or metrics. Development
diagnostics may later suggest which replacement data to generate, but they do
not alter the assessment threshold applied to an already-pinned audit.

## CLI boundary

The complete capability is CLI-first. The intended surface is:

```text
synth quality policy-preview ...
synth quality audit-create <DATASET_ID> ...
synth quality audit-start <AUDIT_ID>
synth quality audit-status|audit-watch <RUN_ID>
synth quality audit-cancel|audit-recover <RUN_ID>
synth quality assessments <RUN_ID> [filters]
synth quality summary <RUN_ID>
synth quality curate <RUN_ID>
synth quality row-review <ASSESSMENT_ID> ...
synth quality proposal <PROPOSAL_ID>
synth quality manifest-review <PROPOSAL_ID> --approve|--reject ...
synth quality manifest <MANIFEST_ID>
synth snapshot create ... --quality-manifest <MANIFEST_ID>
```

Do not add an HTTP API or graphical UI in this phase.

## Provenance and integrity

Provenance must trace:

```text
qualified snapshot
  -> approved curation manifest and review
  -> curation proposal and row reviews
  -> row assessments and evaluator attempts
  -> audit run, immutable policy, and source-set plan
  -> generated/imported source rows
  -> semantic/authenticity contexts when present
```

`synth doctor` verifies fingerprints, foreign-key ownership, source-set
completeness, score shape, counter reconciliation, append-only review chains,
manifest/proposal agreement, approved selection membership, and qualified
snapshot references. Tampering must be reported rather than normalized away.

## Test priorities

- stable source-set and plan identity regardless of input ordering;
- exact label/dimension score-shape validation;
- deterministic threshold and borderline classification;
- conservative repeated-assessment aggregation;
- complete curation decisions and default exclusion of unevaluated rows;
- append-only row and manifest review rules;
- manifest approval and idempotent application;
- batching, retry, cancellation, interruption, and finite-budget behavior;
- evaluator adapter normalization and unsafe-output rejection;
- qualified snapshot membership and legacy snapshot compatibility;
- per-cell and provenance-stratum quality summaries from persisted facts;
- full provenance and tamper detection;
- offline CLI process acceptance using the fake evaluator.

## Explicit non-goals

Do not add semantic or embedding deduplication, automatic relabeling, source-row
mutation, sealed-evidence inspection, benchmark acceptance, model training,
automatic regeneration, distributed workers, a graphical UI, an unbounded
agent, or a claim that an LLM assessment is ground truth.

## Completion criterion

The capability is complete when a user can pin accepted rows, run a bounded
replaceable evaluator, inspect immutable semantic-quality evidence, review
borderline decisions, approve a complete curation manifest, build a snapshot
containing only explicitly qualified rows, inspect qualified coverage and
provenance, detect tampering, and execute the entire flow offline with a fake
evaluator. Existing unqualified snapshots continue to work, while a configured
governed workflow can require the quality gate before training.
