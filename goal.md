# Goal — Slice 6.1: Constrained Optimization and Human-Governed Experiment Cycles

Advance the Rust encoder-development platform from basic error-weighted data
allocation to a decision-grade, reproducible optimization workflow.

This extends Slice 6. Do not rewrite the platform or weaken the completed data
generation, dataset management, training, evaluation, comparison, selection,
error-analysis, provenance, recovery, or persistence boundaries.

Keep this phase CLI-only. Do not add or extend graphical UI, TUI, or HTTP API
behavior.

The guiding principle is:

Optimization may recommend and explain a finite next experiment, but a human
must explicitly approve every mutation and start every expensive operation.

The platform already supports a basic immutable proposal that allocates an
additional-example budget in proportion to cell error counts and can create an
unequal Slice 1 generation plan. Preserve that workflow while making the
optimizer safer, more expressive, comparison-aware, and empirically auditable.

## 1. Architecture and compatibility audit

Before implementation:

1. Read `AGENTS.md`.
2. Read `docs/platform-spec.md`.
3. Read `docs/slice-6-spec.md`.
4. Read `docs/architecture.md`.
5. Read `docs/development.md`.
6. Read `docs/error-analysis.md`.
7. Inspect the current analysis diagnostic contract, optimization proposal,
   generation planner, project configuration, snapshot, training,
   checkpoint, evaluation, comparison, selection, review, persistence,
   provenance, recovery, doctor, export, and CLI contracts.

Preserve historical optimization proposals and the existing commands where
practical. Add append-only migrations and explicit legacy defaults.

Do not leak SQLite, CLI, generation-backend, trainer, Candle, tokenizer, HTTP,
or presentation types into `optimization-core`.

Record material architectural decisions when persistence or public contracts
change.

## 2. Immutable optimization protocol

Introduce a normalized, validated optimization protocol that captures at
least:

- the recommendation kinds to calculate;
- the finite additional-example budget;
- minimum evidence support;
- scoring policy and deterministic tie rules;
- uncertainty/risk policy;
- allowed and excluded labels, cells, and dimension values;
- optional per-label and per-cell lower/upper allocation bounds;
- maximum share of the total budget assignable to one cell;
- minimum useful allocation increment;
- whether comparison regression evidence affects priority;
- whether accepted-limitation or resolved findings are excluded;
- optional bounded training-candidate generation;
- deterministic seed for any tie-breaking or candidate enumeration;
- behavior for infeasible constraints and unallocated budget.

Validate contradictory, unsafe, or impossible settings before loading large
evidence sets.

Persist the complete resolved protocol and a deterministic SHA-256 fingerprint
on every new proposal.

The proposal identity must include:

- analysis-report ID and fingerprint;
- analysis protocol and evaluation evidence identity;
- source dataset and snapshot IDs/fingerprints;
- current accepted-cell coverage fingerprint;
- optional comparison ID/fingerprint;
- optimization protocol fingerprint;
- exact supported training configuration space when training candidates are
  requested.

Never consult mutable project defaults after proposal construction begins.

## 3. Stable evidence boundary

Continue consuming Slice 5 through a project-owned diagnostic contract rather
than report layout, SQLite rows, CLI types, or presentation order.

Expand that contract only where the optimizer genuinely needs stable facts,
such as:

- structured cell identity;
- support, error count, and error rate;
- error-rate lift and share of errors;
- high-confidence error severity;
- marginal and cumulative error coverage;
- comparison fixed/regressed/persistent counts where applicable;
- finding/review disposition relevant to eligibility;
- immutable evidence fingerprints.

The optimizer must reject incomplete, mismatched, non-finite, stale, or
internally inconsistent evidence with actionable errors.

Do not let optimization query normalized analysis tables directly. Application
composition may resolve immutable artifacts, but decision logic consumes only
optimization-owned inputs and upstream project-owned contracts.

## 4. Evidence-aware scoring policies

Replace the single implicit error-count weighting rule with explicit,
deterministic scoring policies.

Support at least:

- error count;
- error rate;
- error-rate lift versus evaluation baseline;
- high-confidence error severity;
- unique marginal error coverage;
- comparison regression priority;
- a documented conservative composite policy.

The conservative composite should avoid over-prioritizing tiny, noisy cells.
Use a deterministic and documented uncertainty adjustment such as a Wilson
lower bound, shrinkage toward the baseline, or another bounded analytical
method that requires no random simulation.

Every recommendation must expose an auditable score breakdown rather than one
opaque number. Include the raw evidence, transformations, penalties, boosts,
eligibility decision, and final score.

Define and test:

- zero support and zero errors;
- zero variance and perfect error rate;
- negative lift;
- missing comparison evidence;
- ties;
- floating-point clamping and rounding;
- very large counts and overflow behavior;
- reviewed findings excluded by policy;
- unknown or no-longer-valid dataset cells.

Do not claim that a score proves root cause or expected model improvement.

## 5. Constrained deterministic budget allocation

Build a pure allocation engine that converts eligible scored cells and explicit
constraints into a finite recommendation set.

It must support:

- exact total-budget conservation when constraints are feasible;
- explicit unallocated budget with reasons when constraints are infeasible;
- per-cell minimum and maximum additions;
- per-label minimum and maximum shares;
- global maximum share per cell;
- exclusions and frozen cells;
- deterministic largest-remainder or equivalent integer allocation;
- stable tie-breaking on canonical cell identity;
- current accepted coverage and absolute proposed targets;
- saturation detection when a cell cannot accept more allocation.

Do not hide infeasibility by silently violating a bound. Return a structured
constraint result explaining which constraint prevented allocation.

The allocation engine must be independent from SQLite, analysis persistence,
generation backends, and CLI parsing and must be exhaustively unit-testable.

Do not introduce a heavyweight optimization solver unless the required finite
integer allocation cannot be expressed clearly with small project-owned logic.

## 6. Recommendation types

Support explicit, independently reviewable recommendation sections.

### Data-generation recommendations

Recommend unequal absolute cell targets consumable by the existing Slice 1
planner. Preserve arbitrary categorical dimensions and canonical cell keys.

Each recommendation must include:

- source cell identity;
- current accepted count;
- proposed additional count and absolute target;
- complete score breakdown;
- supporting analysis finding IDs/keys/fingerprints;
- applicable comparison and review context;
- constraints that affected the allocation;
- a concise deterministic rationale assembled from structured facts.

### Training-configuration candidates

Optionally propose a small, bounded set of explicit training configurations for
an approved future run.

Training candidates must:

- use only settings supported by the selected registered backend;
- stay inside user-supplied finite ranges or enumerated choices;
- identify the baseline training run/configuration/checkpoint;
- state exactly which fields differ;
- include deterministic rule-based rationale and cautions;
- preserve dataset/snapshot identity requirements;
- be immutable and fingerprinted;
- never start training automatically.

Do not infer an allegedly optimal learning rate, epoch count, regularization,
or transformer mode from one metric. Present candidates as bounded experiments,
not conclusions.

Do not recommend unsupported provider/model-specific settings through generic
maps that bypass backend validation.

### Review-only recommendations

Allow the optimizer to surface cells or confusions as candidates for label or
schema review without mutating labels, dimensions, rows, or snapshots.

Review-only recommendations consume existing Slice 5 finding/review evidence
and remain advisory.

## 7. Alternatives and sensitivity analysis

One score policy must not masquerade as the only rational choice.

Allow generating a bounded set of alternative proposal scenarios from the same
immutable evidence, for example:

- conservative support-adjusted allocation;
- raw error-volume allocation;
- high-confidence-regression allocation;
- balanced per-label allocation.

For each scenario expose:

- protocol/fingerprint;
- eligible and excluded cells;
- allocation and unallocated budget;
- concentration by label and dimension;
- overlap with other scenarios;
- cells whose allocations are sensitive to policy choice.

Scenario comparison must be deterministic and must not duplicate or mutate the
underlying analysis report.

Do not run generation, training, or evaluation merely to compare scenarios.

## 8. Approval and review boundary

Keep immutable proposals separate from append-only human decisions.

Add proposal review records with states such as:

- open;
- approved for plan creation;
- rejected;
- superseded;
- partially accepted with explicit selected recommendation IDs;
- accepted as a training experiment candidate;
- completed and awaiting outcome assessment.

Record timestamps, optional notes, selected recommendation identities, and
optional superseding proposal/campaign references.

Review records must not mutate proposal contents or fingerprints. Do not add
users, authentication, assignment, or collaborative workflow features.

Applying a proposal or subset must require an explicit compatible approval
record unless a clearly named legacy compatibility path is retained.

## 9. Safe application

Applying approved data recommendations must create a normal immutable unequal
generation plan through the existing Slice 1 contract.

Application must:

- verify proposal, protocol, analysis, dataset, coverage, and approval
  fingerprints;
- detect stale current coverage before creating a plan;
- reject cells that no longer belong to the dataset definition;
- preserve exact absolute targets;
- be transactional and idempotent;
- retain proposal, review, recommendation, and source-analysis provenance;
- return the existing plan on an identical repeated application;
- never start a generation job.

If coverage has changed, provide an explicit rebase/refresh command that creates
a new proposal. Never rewrite the stale proposal or silently reinterpret its
absolute targets.

Applying a training candidate may create an immutable configuration draft or
explicit candidate artifact if useful, but must never create or start a
training run automatically.

## 10. Human-governed experiment campaigns

Add a lightweight immutable/append-only campaign ledger that lets a human link
one optimization decision to its later outcomes.

A campaign should be able to record explicit links to:

- baseline analysis and optional comparison;
- chosen optimization proposal/scenario;
- approval decision;
- applied generation plan;
- manually started generation jobs;
- resulting immutable dataset snapshot;
- manually started training runs and checkpoints;
- evaluation runs and paired comparison against the baseline;
- follow-up analysis report;
- final outcome assessment.

Campaign commands only record and validate links to artifacts that already
exist or were explicitly created by a separate normal command. They must not
start, poll, retry, schedule, or orchestrate those workflows.

Validate compatibility at every link:

- generated rows belong to the expected dataset/plan lineage;
- snapshots derive from the expected dataset;
- training runs consume the linked snapshot;
- evaluations consume linked checkpoints and compatible cohorts/protocols;
- comparisons contain the expected baseline and candidate evaluations;
- follow-up analyses reference the candidate evaluation/comparison.

Provide a deterministic outcome summary comparing the baseline and candidate:

- overall and per-label metric deltas;
- confidence intervals/significance already persisted by Slice 4.1;
- fixed, regressed, and persistent errors;
- target-cell coverage changes;
- whether proposal constraints were actually realized;
- explicit outcome classification such as improved, regressed, mixed, or
  inconclusive under a persisted assessment policy.

Do not attribute causality. A campaign records an observed experiment lineage,
not proof that one recommendation caused the result.

## 11. Persistence and migrations

Persist at least:

- normalized optimization protocols and fingerprints;
- immutable evidence/source identities;
- normalized recommendation rows and score breakdowns;
- allocation constraints and feasibility results;
- scenario groups and scenario comparisons;
- optional bounded training candidates;
- append-only proposal review records;
- idempotent applications and selected recommendation subsets;
- campaign identity and append-only artifact links;
- immutable outcome assessments and fingerprints.

Use append-only SQLite migrations. Preserve historical proposal JSON and current
proposal/application behavior through explicit legacy defaults or upgrade
backfills.

Do not store raw secrets, model tensors, duplicated datasets, or unbounded
prediction payloads in optimization tables.

Normalize fields required for filtering, pagination, uniqueness, foreign-key
integrity, doctor checks, and provenance. JSON payloads may preserve complete
immutable artifacts but must not be the only queryable representation of core
relationships.

## 12. CLI workflow

Add focused CLI behavior for:

- creating a proposal from a resolved optimization protocol;
- previewing feasibility before persistence;
- creating bounded alternative scenarios;
- listing/showing proposals and normalized recommendations;
- filtering recommendations by kind, label, dimension, eligibility, and score;
- explaining one recommendation's score and evidence;
- approving, partially accepting, rejecting, or superseding a proposal;
- rebasing stale evidence into a new immutable proposal;
- applying an approved data recommendation set into a generation plan;
- listing/showing/exporting training candidates without starting training;
- creating a campaign ledger;
- linking already-created artifacts to a campaign;
- validating campaign compatibility;
- recording and showing a deterministic outcome assessment;
- exporting proposal summaries and recommendation rows to JSONL and CSV;
- tracing proposal, recommendation, review, application, campaign, and outcome
  provenance.

Suggested conceptual commands include:

```text
synth optimize preview <ANALYSIS_REPORT_ID> --budget 500 --policy conservative
synth optimize propose <ANALYSIS_REPORT_ID> --protocol optimization.toml
synth optimize scenarios <ANALYSIS_REPORT_ID> --budget 500
synth optimize recommendations <PROPOSAL_ID> --label billing --eligible
synth optimize explain <PROPOSAL_ID> <RECOMMENDATION_ID>
synth optimize review <PROPOSAL_ID> --state approved-for-plan-creation
synth optimize apply <PROPOSAL_ID> --approval-id <REVIEW_ID>
synth optimize rebase <PROPOSAL_ID>
synth optimize training-candidates <PROPOSAL_ID>
synth campaign create <PROPOSAL_ID> --approval-id <REVIEW_ID>
synth campaign link <CAMPAIGN_ID> --generation-plan-id <PLAN_ID>
synth campaign link <CAMPAIGN_ID> --snapshot-id <SNAPSHOT_ID>
synth campaign assess <CAMPAIGN_ID> --comparison-id <COMPARISON_ID>
```

Exact command names may differ if existing CLI conventions suggest a clearer
shape.

Keep stdout machine-readable under JSON output. Progress and warnings belong on
stderr. File output must continue to use `--file`.

Do not create an interactive terminal interface.

## 13. Provenance, doctor, and recovery

Extend provenance so every recommendation and campaign outcome can be traced
through:

```text
outcome assessment
  -> campaign
    -> proposal + approval
      -> analysis findings/reviews
        -> evaluation/comparison
          -> checkpoint/training run
            -> immutable snapshot/source rows
    -> applied generation plan/jobs
    -> candidate snapshot/training/evaluation/analysis
```

Extend `doctor` to verify:

- optimization protocol and proposal fingerprints reproduce;
- source analysis/evaluation/dataset/snapshot identities match;
- normalized recommendations reproduce immutable proposal contents;
- score breakdowns reproduce from stable diagnostic evidence;
- allocation obeys every persisted constraint;
- allocated plus unallocated budget equals the finite requested budget;
- application approval and selected recommendation sets are valid;
- applications are idempotent and generated plans match absolute targets;
- stale-coverage detection facts are internally consistent;
- training candidates pass the selected backend's configuration validation;
- campaign links form a compatible acyclic artifact lineage;
- outcome assessments reproduce from persisted comparison/evaluation facts;
- append-only reviews reference existing immutable artifacts;
- historical proposal JSON remains decodable.

Doctor must not call an LLM, load model tensors, run inference, train, generate
data, or mutate workflow state.

Optimization proposal construction is a bounded read plus transactional write.
A crash before the transaction leaves no partial proposal. Campaign links and
review records are append-only. Recovery must never auto-apply or auto-resume an
optimization decision.

## 14. Tests and practical verification

Ordinary tests must require no network, provider credentials, external model,
Python runtime, GPU, or paid service.

Add focused tests for:

- optimization-protocol validation and fingerprinting;
- stable evidence-contract conversion and mismatch rejection;
- every scoring policy and score breakdown;
- uncertainty adjustment and numerical edge cases;
- exact integer budget conservation;
- infeasible constraints and structured unallocated reasons;
- per-cell, per-label, concentration, exclusion, and saturation constraints;
- deterministic ties and canonical arbitrary-dimension cells;
- comparison regression boosts and missing-comparison behavior;
- review-disposition eligibility;
- scenario determinism, overlap, and sensitivity summaries;
- bounded training-candidate enumeration and backend validation;
- immutable proposal/recommendation fingerprint reproducibility;
- normalized persistence round trips and migration upgrades;
- proposal filtering, sorting, and bounded pagination;
- append-only review history and partial acceptance;
- stale-coverage rejection and explicit rebase behavior;
- transactional/idempotent plan application;
- campaign link compatibility and invalid-lineage rejection;
- deterministic outcome assessment;
- JSONL/CSV exports;
- provenance through recommendations, approvals, campaigns, and outcomes;
- doctor detection of altered evidence, scores, allocations, links, or
  fingerprints;
- legacy proposal compatibility.

Expand the offline CLI end-to-end workflow using deterministic local fixtures:

1. create/import and generate a dataset;
2. create a baseline snapshot, training run, evaluation, and analysis;
3. preview multiple optimization scenarios;
4. persist a constrained proposal;
5. inspect and explain normalized recommendations;
6. record explicit approval;
7. apply approved data recommendations into a normal generation plan;
8. explicitly run generation with the normal Slice 1 command;
9. explicitly create a new snapshot, train, and evaluate with normal commands;
10. create a paired comparison and follow-up analysis;
11. link the already-created artifacts into a campaign;
12. record and inspect the deterministic outcome assessment;
13. export recommendations to JSONL and CSV;
14. trace provenance; and
15. pass doctor.

Keep the existing linear, tiny-transformer, comparison-aware analysis, and
legacy optimization workflows passing.

## 15. Documentation

Update:

- architecture;
- Slice 6 specification;
- configuration;
- CLI guide;
- operations;
- recovery;
- provenance;
- migrations;
- development workflow;
- README.

Add a focused optimization guide explaining:

- raw evidence versus adjusted scores;
- why uncertainty/support adjustment matters;
- allocation constraints and infeasibility;
- absolute targets versus additional counts;
- alternatives and sensitivity analysis;
- data, training, and review-only recommendations;
- approval and partial acceptance;
- stale evidence and rebasing;
- campaign lineage and outcome assessment;
- correlation versus causal attribution;
- why optimization never starts generation, training, or evaluation.

## Explicit non-goals

Do not implement:

- graphical UI, TUI, or HTTP API additions;
- automatic generation, snapshotting, training, evaluation, or analysis;
- a daemon, scheduler, watcher, or background optimization worker;
- autonomous agents or closed-loop experiment execution;
- bandits, reinforcement learning, Bayesian optimization, or population-based
  training;
- unbounded hyperparameter search;
- LLM-authored recommendations, explanations, or judges;
- external spending or provider calls from optimization;
- automatic label/schema/data mutation;
- causal claims;
- cloud or distributed execution;
- multiple workers;
- authentication or multi-user collaboration;
- semantic/embedding clustering or deduplication;
- external experiment-tracking services;
- arbitrary user-supplied scoring code.

## Working method

Implement one coherent component at a time.

Suggested sequence:

1. architecture audit and immutable optimization protocol;
2. stable evidence/source identity contract;
3. pure scoring policies and score explanations;
4. constrained deterministic integer allocation;
5. immutable normalized proposals and recommendations;
6. alternative scenarios and sensitivity summaries;
7. append-only approvals and partial acceptance;
8. stale detection, rebase, and safe application;
9. bounded training-configuration candidates;
10. campaign ledger and compatibility validation;
11. outcome assessment;
12. CLI filters, explanations, and exports;
13. provenance, doctor, and recovery integration;
14. end-to-end tests and documentation.

After every coherent component, run:

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

Review dependency directions and public interfaces after each component.

Commit coherent working changes only when Git identity is configured.

Preserve unrelated user changes.

## Completion criterion

This goal is complete only when a user can:

1. create a reproducible constrained optimization protocol from immutable
   analysis evidence;
2. compare deterministic scoring/allocation scenarios and understand why their
   recommendations differ;
3. inspect every recommendation's raw evidence, uncertainty adjustment, score,
   constraints, and rationale;
4. conserve a finite budget exactly or see explicit structured reasons for
   unallocated budget;
5. approve, reject, supersede, or partially accept a proposal without mutating
   it;
6. safely create an ordinary unequal generation plan only from explicitly
   approved, non-stale recommendations;
7. inspect bounded optional training candidates without starting training;
8. manually link later generation, snapshot, training, evaluation, comparison,
   and analysis artifacts into a validated campaign lineage;
9. assess observed outcomes deterministically without causal claims;
10. export normalized recommendations, trace complete provenance, and pass
    doctor and all standard checks;
11. complete the expanded offline CLI workflow without network access; and
12. retain compatibility with historical optimization proposals.

Architecture and human control matter as much as allocation quality. Slice 6
must remain an advisory, replaceable component that proposes finite experiments
while every generation, training, and evaluation action stays explicit.
