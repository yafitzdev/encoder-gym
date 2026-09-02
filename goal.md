# Goal — One Bounded “Optimize This Encoder” Workflow

Turn Encoder Gym’s existing slices and the real Nomos experiment into one
coherent, resumable, operator-facing optimization workflow.

The product promise for this goal is deliberately simple:

> Given a verified encoder project, its baseline model, eligible training data,
> development benchmarks, and separately governed sealed acceptance evidence,
> an operator can start one finite optimization run, review the proposed repair,
> and receive a reproducible `promote_candidate` or `retain_baseline` decision.

Do not add another speculative agent, score, optimizer, or quality subsystem.
First finish the evidence-driven Nomos repair path already underway. Then
productize that exact proven path so the operator no longer has to manually
connect many low-level commands and UUIDs.

This remains CLI-first. “Press optimize” means one clear command and a small
number of explicit approval commands, not a GUI and not an autonomous endless
loop.

## Why this is the next goal

Encoder Gym already contains independently useful components for generation,
dataset management, qualification, training, evaluation, error analysis,
optimization, governed workflows, production encoder experiments, renewable
benchmark generations, and finite campaigns.

The remaining product gap is composition:

- the operator still needs detailed knowledge of internal artifact IDs;
- the first Nomos campaign produced an honest negative result but did not yet
  convert its development failures into a targeted retraining experiment;
- repair diagnosis has a provider-neutral core, but it is not yet persisted,
  exposed through the CLI, or connected to reviewed data/training actions;
- there is no single durable run that explains the current stage, required
  approval, budget usage, candidate results, and final decision.

Building more “smart” components before closing this loop would increase
complexity without proving the central value proposition.

## Starting state that must be preserved

The authoritative completed Nomos campaign is
`8a6e1004-a9df-4b3c-a127-42b121ed337c` in:

`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment\encoder-gym-final.sqlite`

It established that:

1. The 2.5% MNRL interpolation candidate passed `generic_holdout` but failed
   the required retired-suite MRR improvement.
2. The 5% candidate passed `retired_post_scaling` but regressed generic
   Recall@2 beyond the allowed gate.
3. Neither candidate became eligible for sealed evaluation.
4. The baseline was correctly retained.
5. Successor generation `10cba5de-e501-4169-a603-27f75c2abd37` remains active,
   fresh, unused, and has zero candidate exposures.

The existing `encoder-repair-core` is the start of this goal. It owns complete,
text-free development observations and deterministic comparative diagnosis for
production encoder tasks. Finish that boundary; do not replace it with
Nomos-specific orchestration or an LLM summary.

Preserve the completed campaign and its row-free provenance. Prefer a new
database or an explicitly append-only continuation. Never rewrite historical
evidence.

## Target user journey

The supported journey should become:

```text
verify project and immutable inputs
  -> preview one finite optimization definition
  -> start or resume one durable optimization run
  -> collect complete development-only observations
  -> compile deterministic comparative diagnosis
  -> compile a small explicit repair proposal
  -> operator reviews and approves exact data/training actions
  -> create and qualify an immutable targeted training delta
  -> construct an immutable candidate training snapshot
  -> genuinely retrain a bounded candidate set
  -> evaluate every candidate on every development suite
  -> reject candidates that fail any suite
  -> select at most one fully eligible candidate
  -> stop for explicit sealed-use authorization
  -> evaluate the selected candidate once on sealed evidence
  -> deterministically promote or retain
  -> emit a complete management report and provenance chain
```

At every pause, the CLI must say in plain language:

- what has completed;
- what failed or remains uncertain;
- what immutable artifacts were created;
- what budget remains;
- why execution stopped;
- whether an operator decision is required;
- the exact safe next command.

## Capability 1 — Persisted comparative repair evidence

Complete the provider-neutral repair evidence path.

Add immutable persistence, deep verification, and CLI inspection for:

- complete development observation sets;
- baseline-versus-candidate row outcomes;
- shared, suite-specific, persistent, fixed, and regressed failure groups;
- rank movement, abstention behavior, top-k failures, margin evidence, and
  supported native dimensions;
- minimum-support and uncertainty limitations;
- exact gate and metric trade-offs for every prior candidate;
- a reproducible diagnosis fingerprint derived from its exact inputs.

The Nomos adapter may normalize native development results into this contract,
but native rows, labels, paths, Python types, and evaluator details must not
leak into the core.

The observation collector must be structurally unable to consume a sealed
suite. Persist no row text, tool identifiers, legal content, or other native
payload. Doctor must recompute the diagnosis and detect missing, reordered,
foreign, or tampered evidence.

Run this diagnosis against the real completed Nomos campaign before designing
the repair. Report what the evidence actually says; do not preselect a favored
training technique.

## Capability 2 — Finite, reviewable repair proposals

Compile one diagnosis into a small repair proposal that pins:

- the exact project, campaign, experiment run, reports, assessments,
  observation sets, and diagnosis;
- the failure groups targeted and why they were selected;
- whether each action changes data, training settings, or both;
- exact base training inputs and immutable source fingerprints;
- exact generation/import cells and absolute row targets;
- total row, per-group, candidate-count, training-time, evaluation, provider,
  and sealed-use budgets;
- duplicate, leakage, contamination, and task-quality policies;
- a finite candidate set and explicit hypotheses for each candidate;
- the active benchmark-generation authority;
- an expiry/staleness fingerprint over every decision-relevant input.

An approved proposal is immutable. It must become stale if any pinned evidence,
base data, project implementation, evaluation authority, or relevant policy
changes. Reapplying the same approved proposal must return the same artifacts
without duplicating rows, snapshots, candidates, or runs.

Keep concrete failure groups and failed gates visible. Do not collapse them
into an opaque scalar quality score. An LLM may summarize deterministic
evidence or suggest bounded hypotheses only through an explicit external-call
authorization and can never approve or apply a proposal.

## Capability 3 — Task-neutral quality for native repair deltas

The fixed-label dataset-quality path must not be forced onto Nomos retrieval
rows by pretending they are global classification labels.

Extract or extend the smallest genuinely reusable contract needed to qualify a
native repair delta. It should own:

- immutable candidate-row identity without retaining payload in core;
- task-adapter validation evidence;
- exact and normalized duplicate results;
- source, group, and lineage contamination results;
- deterministic policy thresholds and concrete failure reasons;
- complete include/exclude decisions;
- append-only human review;
- an approved immutable selection manifest.

Reuse existing dataset-quality primitives where their semantics are actually
the same. Do not duplicate policy/review logic, and do not weaken the existing
classification contract merely to accommodate Nomos.

For the real experiment, construct the smallest targeted Nomos training delta
that tests the diagnosis. Generate deterministic fields with code wherever
possible. Every row must retain its source recipe, seed, target group, proposal,
generator/importer identity, project revision, and content fingerprint.

Prove that the delta:

- satisfies native schema and task invariants;
- contains no exact or normalized duplicates within itself or against base
  training data;
- has no prohibited source/group lineage overlap with either development
  suite or the active sealed cohort;
- was reviewed under the exact pinned quality policy;
- produces an immutable delta snapshot and combined candidate snapshot without
  mutating the base population.

An independent contamination audit may inspect sealed identities internally,
but adaptive code may receive only the permitted aggregate clean/blocked
result. No sealed content or behavior may enter diagnosis or row construction.

No network or paid model call is authorized by this goal. If external
generation becomes necessary, persist the exact bounded request and stop for
explicit authorization. Continue with a meaningful deterministic/local repair
experiment wherever possible.

## Capability 4 — Genuine retraining from exact repair inputs

At least one candidate must be genuinely trained from the approved immutable
training inputs. Interpolation may exist only as a declared control; it cannot
be the sole candidate mechanism.

Candidate and checkpoint identity must pin:

- base model and base training population;
- approved repair proposal and quality manifest;
- delta and combined training snapshot;
- trainer/backend identity and implementation revision;
- all behavior-affecting parameters, seeds, schedules, and stopping rules;
- exact training budget and observed usage;
- produced checkpoint files and content fingerprints;
- interrupted/resumed attempt identity.

Extend the generic training/experiment port only with provider-neutral
concepts. Nomos-specific Python modules, model layouts, CUDA settings,
parameters, and native manifests remain inside the Nomos adapter.

Use a deliberately small candidate set. The objective is a causal, inspectable
experiment, not a hyperparameter sweep. A negative result is valid and must not
cause gates or hypotheses to be rewritten after the fact.

## Capability 5 — One durable optimization-run contract

Introduce the smallest application-level contract that links the existing
artifacts into one finite optimization run. It owns lifecycle and links, not
the business logic of the slices.

The run definition must include:

- project and adapter identity;
- baseline and training-input authority;
- development suites and their strict per-suite gates;
- optional sealed suite through its active benchmark generation;
- diagnosis and repair policies;
- candidate and execution budgets;
- approval mode and permitted external actions;
- final selection and promotion policy.

The lifecycle should use explicit durable states such as:

```text
planned
awaiting_repair_review
applying_repair
training
evaluating_development
awaiting_sealed_authorization
evaluating_sealed
completed
failed
cancelled
```

Use more precise substates where needed, but do not create one giant state
machine that copies campaign, training, evaluation, or quality rules. Each
stage must reserve its exact child artifact before side effects. Recovery may
resume only that reservation.

The run must be finite, budgeted, cancellable, and reconstructible from SQLite
plus immutable files. Process memory is never the source of truth. Retry may
finish an interrupted action but may not silently create a replacement action
or spend a second sealed/external-call budget.

## Capability 6 — A usable CLI “optimize” surface

Expose one cohesive command family. Exact naming may follow existing CLI
conventions, but the operator should have equivalents of:

```text
encoder optimize preview --manifest <FILE>
encoder optimize start --manifest <FILE>
encoder optimize status <RUN_ID>
encoder optimize inspect <RUN_ID>
encoder optimize review-repair <RUN_ID> ...
encoder optimize resume <RUN_ID>
encoder optimize authorize-external <RUN_ID> ...
encoder optimize authorize-sealed <RUN_ID> ...
encoder optimize cancel <RUN_ID>
encoder optimize doctor <RUN_ID>
encoder optimize provenance <RUN_ID>
encoder optimize report <RUN_ID>
```

`preview` must be read-only and resolve defaults into a complete fingerprinted
definition. `start` must either create one run or return the existing identical
run. `resume` must execute only the next legal reserved action. Approval
commands must append immutable decisions and reject stale artifacts.

Do not require the operator to copy internal UUIDs between unrelated commands
when the owning run can resolve them unambiguously. Keep the lower-level slice
commands available for inspection, debugging, and independent use.

Provide one strict, documented Nomos optimization manifest as the production
example. Do not add runtime loading of arbitrary executables or user-authored
shell commands. V1 continues to use statically compiled, trusted adapters.

## Capability 7 — Real Nomos acceptance run

The goal is not complete with fake tests or scaffolding. Run the complete path
against only:

`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment`

The source repository at
`C:\Users\yanfi\PycharmProjects\fitz-tool` remains read-only. Record its exact
HEAD and working-tree state before and after the experiment and prove they are
identical. The isolated copy must remain without a remote and contain no API
keys or credentials.

Use the same active unused successor generation while it remains valid. Every
new candidate must first produce independent reports for `generic_holdout` and
`retired_post_scaling` and pass every strict gate in both suites.

If no candidate passes both development suites:

- retain the baseline;
- finish the optimization run honestly;
- leave the sealed generation unused;
- report the failed hypotheses and evidence limitations.

If one candidate passes both suites:

- select it deterministically among fully eligible candidates;
- stop at explicit sealed-use authorization;
- after authorization, evaluate exactly that candidate exactly once;
- atomically consume the generation and persist `promote_candidate` or
  `retain_baseline` under unchanged gates.

Do not acquire another successor cohort automatically. If the current sealed
cohort is consumed, persist that a new independently qualified generation is
required and stop.

## Capability 8 — Management report and portable evidence bundle

Produce a deterministic final report for humans and a machine-readable,
row-free evidence bundle.

The report must explain in simple terms:

- the baseline and exact project revision;
- what weaknesses were diagnosed and from which development evidence;
- the approved repair hypothesis;
- what data and training settings changed;
- which candidate checkpoints were created;
- per-suite baseline and candidate results;
- why each candidate passed or failed;
- whether sealed evidence was used and exactly once;
- the final promotion/retention decision;
- budget usage, interruptions, retries, and approvals;
- known evidence limits and the next safe action.

The evidence bundle must contain immutable identities, fingerprints, policies,
decisions, and relative artifact references—not raw training, development, or
sealed row content. Verification on the same trusted project checkout must
detect missing files, wrong revisions, tampered database records, foreign
artifacts, and incomplete journals.

## Architecture rules

- Read the platform, architecture, development, dataset-quality,
  error-analysis, optimization, workflow-governance, production-experiment,
  and renewable-campaign specifications before implementation.
- Map the full flow to current crate ownership before adding a crate.
- Add a new core crate only if no existing owner can express the missing
  provider-neutral lifecycle without reversing dependencies.
- Keep orchestration thin: it links immutable artifacts and calls ordinary
  application contracts; it does not reimplement analysis, curation, training,
  evaluation, selection, or campaign rules.
- Keep domain objects independent from SQLite, Python, Nomos, CLI parsing,
  subprocesses, SDKs, and presentation.
- Keep all evidence append-only, fingerprinted, and deeply replayable.
- Add migrations that upgrade prior databases without rewriting history.
- Use deterministic fakes for ordinary tests. No ordinary test may require the
  Nomos copy, network, model download, GPU, credential, or paid provider.
- Implement one coherent component at a time, run all standard gates, review
  dependency direction, and commit it before starting the next component.
- Do not broadly refactor unrelated slices while implementing the convenience
  workflow.

## Required failure and recovery tests

At minimum, test:

- sealed evidence rejected from adaptive observation and diagnosis;
- incomplete, reordered, mismatched, and tampered observation sets;
- stale repair review and stale run definition;
- duplicate proposal application;
- native quality failure and incomplete row assessments;
- exact, normalized, source, and group contamination;
- trainer crash before and after checkpoint creation;
- evaluator crash before and after report creation;
- cancellation at every side-effecting stage;
- retry without duplicate candidate, report, or budget spend;
- one candidate failing only one development suite;
- missing suite evidence remaining ineligible;
- selection considering only all-suite-passing candidates;
- sealed authorization mismatch and repeated authorization;
- atomic sealed consumption and final decision;
- Doctor detecting database and file tampering;
- migration from the current production database schema;
- final report and evidence-bundle reproducibility.

Include one CLI end-to-end test with deterministic fake adapters that starts
from a manifest and reaches both terminal paths:

- `retain_baseline` without sealed use because development fails;
- an explicitly authorized sealed assessment followed by a deterministic final
  decision.

## Non-goals

Do not add GUI, TUI, HTTP endpoints, cloud deployment, distributed workers,
authentication, multi-user support, an open plugin loader, arbitrary shell
execution, automatic benchmark acquisition, an endless autonomous loop,
reinforcement learning, bandits, automatic gate changes, automatic approval,
semantic deduplication, or a fleet of token-saving encoders.

Do not make an LLM the authority for metrics, quality acceptance, candidate
selection, sealed use, or promotion. Do not hide strict cohort-specific
regressions inside averaged scores.

Promotion is not required. A correct, deeply verified `retain_baseline` result
is a successful product outcome.

## Execution order

Work in coherent, independently verified stages:

1. Persist and expose complete comparative repair evidence.
2. Run the real Nomos diagnosis and freeze its findings.
3. Implement reviewed, stale-safe repair proposals.
4. Implement native repair-delta qualification and contamination proofs.
5. Produce the real targeted delta and immutable candidate snapshot.
6. Add genuine Nomos retraining from exact approved inputs.
7. Run the low-level bounded Nomos repair experiment successfully end to end.
8. Introduce the thin durable optimization-run composition contract.
9. Add the cohesive CLI commands and deterministic end-to-end tests.
10. Run the same real flow through the new operator surface.
11. Verify provenance, repository isolation, migrations, and every quality gate.
12. Update documentation and finish with exact staged commits.

Do not build the convenience layer before the low-level real path works. This
prevents the product surface from fossilizing an unproven experiment design.

## Completion criterion

The goal is complete only when all of the following are true:

1. The completed prior Nomos campaign can be converted into a persisted,
   reproducible, development-only comparative diagnosis.
2. An exact, finite, reviewed repair proposal becomes stale on any relevant
   input change and applies idempotently.
3. A targeted, contamination-free, quality-approved training delta and
   immutable combined training snapshot exist without modifying base data.
4. At least one genuinely retrained Nomos checkpoint derives from those exact
   approved inputs with complete provenance.
5. Every candidate is independently evaluated on both development suites, and
   one failed suite rejects it before sealed use.
6. One real bounded Nomos optimization run reaches a deeply verified
   `promote_candidate` or `retain_baseline` decision under the unchanged
   evidence rules.
7. A user can reproduce that flow from one manifest through one cohesive CLI
   command family without manually wiring internal artifact IDs.
8. Recovery, cancellation, idempotency, budgets, staleness, contamination,
   migration, tamper detection, Doctor, provenance, report generation, and both
   terminal fake paths are covered by deterministic offline tests.
9. The original Nomos repository finishes at its original HEAD and exact
   pre-existing working-tree state; the isolated copy has no remote and no
   secret.
10. `cargo fmt-check`, `cargo check-all`, `cargo lint`, `cargo test-all`, the
    explicit isolated Nomos integrity test, and relevant Python freeze/audit
    tests all pass.

Finish with a concise management summary that separates:

- reusable platform capabilities added;
- operator workflow improvements;
- real Nomos diagnosis and repair hypothesis;
- data/training changes actually tested;
- per-development-suite results;
- sealed result, if legitimately obtained;
- whether the production baseline changed;
- remaining evidence limitations;
- exact Encoder Gym and isolated Nomos commits;
- the next safe action.
