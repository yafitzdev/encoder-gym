# Long-Range Goal — Prove and Productize One Real Encoder Improvement Loop

Finish one honest, bounded improvement experiment against the isolated Nomos
encoder project, and then turn that exact proven path into Encoder Gym's first
cohesive, resumable `optimize` workflow.

The product promise is:

> Given a verified encoder project, immutable training inputs, development
> benchmarks, a baseline checkpoint, and separately governed sealed evidence,
> an operator can run one finite optimization cycle and receive a reproducible
> `promote_candidate` or `retain_baseline` decision with a complete provenance
> chain.

This is the next highest-impact goal because Encoder Gym already has many of
the individual slices. What it has not yet proved is that they can improve—or
correctly decline to replace—a real production encoder without leakage,
hand-waving, or manual UUID plumbing.

Do not add another speculative smart subsystem. Close the loop already in
progress.

## Current verified starting point

Preserve the work that already exists.

- The authoritative completed Nomos campaign is
  `8a6e1004-a9df-4b3c-a127-42b121ed337c` in
  `C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment\encoder-gym-final.sqlite`.
- It honestly retained the baseline: the 2.5% interpolation candidate preserved
  generic behavior but did not improve the retired suite enough; the 5%
  candidate improved the retired suite but regressed generic Recall@2.
- Successor benchmark generation
  `10cba5de-e501-4169-a603-27f75c2abd37` remains active, fresh, unused, and
  has zero candidate exposures. Do not consume it until a candidate passes
  every development gate and the operator explicitly authorizes sealed use.
- Persisted development-only repair diagnosis
  `7accb2d5-37a7-4fb8-ae87-943bd25032b5` identifies the main repair target as
  retired `bounded_change_preflight` / text behavior while preserving generic
  near-neighbor, `finalize_selection`, and top-2 retrieval behavior.
- Immutable, stale-safe repair proposals, append-only reviews, idempotent
  application reservations, and CLI inspection already exist.
- Native repair-delta quality contracts and SQLite persistence are currently
  being implemented in the working tree. Inspect and finish those changes;
  do not discard, reset, or duplicate them.

The original Nomos repository at
`C:\Users\yanfi\PycharmProjects\fitz-tool` is read-only for this experiment.
Record its exact HEAD and working-tree state before and after all work and prove
that they are identical. All experiment changes belong only in
`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment` and Encoder Gym.
The isolated copy must have no remote and contain no credential or API key.

## Required end-to-end journey

Build and prove this exact finite flow:

```text
verify immutable project and baseline inputs
  -> load complete development-only observations
  -> replay deterministic diagnosis
  -> review one exact repair proposal
  -> create a small targeted training delta
  -> prove task validity, uniqueness, and non-contamination
  -> approve an immutable delta selection
  -> construct an immutable combined training snapshot
  -> genuinely train a deliberately small candidate set
  -> evaluate every candidate on every development suite
  -> reject candidates that fail any suite
  -> select at most one fully eligible candidate
  -> stop for explicit sealed-use authorization
  -> evaluate that candidate exactly once on sealed evidence
  -> deterministically promote or retain the baseline
  -> emit a human report and machine-verifiable evidence bundle
```

First make this low-level path work against the real isolated Nomos project.
Only then wrap the proven contracts in the convenience workflow. Do not design
the orchestration around hypothetical behavior.

## Milestone 1 — Finish native repair-delta qualification

Complete the in-progress provider-neutral quality boundary. It must support
retrieval/task-native rows without pretending that they are ordinary global
classification labels.

It must persist, replay, and deeply verify:

- payload-free candidate-row identity;
- task-adapter validation evidence and concrete reasons;
- exact and normalized duplicate counts;
- source, group, and lineage contamination counts;
- deterministic include/exclude decisions;
- the exact quality policy and thresholds;
- append-only human reviews;
- one immutable approved selection manifest.

Every candidate row must be assessed exactly once. Reports with missing,
foreign, duplicated, or reordered assessments must fail verification. An
ineligible report cannot be approved. Repeating the same creation or approval
request must return the same artifact rather than append duplicates.

Keep this contract task-neutral and row-free. Native content may be processed
inside the trusted Nomos adapter, but it must not leak into the core database,
diagnosis, CLI output, or portable evidence bundle.

Add migration and tamper tests before moving on. Upgrade existing production
databases append-only; never rewrite historical campaign evidence.

## Milestone 2 — Build the smallest real Nomos repair delta

Implement a deterministic, inspectable Nomos recipe for the diagnosed weakness.
The delta should be deliberately small and causal, not a broad synthetic-data
campaign.

Target both sides of the observed trade-off:

1. improve retired `bounded_change_preflight` text routing and its important
   distinctions from publish, delete, finalize, and unrelated actions;
2. preserve generic near-neighbor, `finalize_selection`, modality constraint,
   and top-2 retrieval behavior.

Generate deterministic fields with code wherever possible. Use fresh training
recipes, wording, identifiers, seeds, groups, and lineage. Do not derive or
paraphrase development or sealed rows. Each row must retain:

- proposal and target-group identity;
- recipe and generator fingerprint;
- deterministic seed;
- project revision;
- content and normalized-content fingerprint;
- source, group, and lineage fingerprint;
- task-validation outcome;
- final include/exclude decision.

Audit the delta against itself, all base training inputs, both development
suites, and the active sealed cohort. A trusted contamination checker may
inspect sealed identities internally, but adaptive code receives only the
permitted aggregate clean/blocked result. No sealed text, label behavior,
score, or example may influence generation, diagnosis, or candidate choice.

Produce an immutable delta snapshot and combined candidate-training snapshot
without mutating the base training population or silently changing the project
manifest.

No network or paid model call is authorized. If the deterministic/local recipe
is genuinely insufficient, persist the exact bounded external request and stop
for explicit authorization instead of making the call.

## Milestone 3 — Genuinely retrain bounded candidates

At least one candidate must be genuinely trained from the exact approved delta
and combined snapshot. Weight interpolation may be included only as an
explicit control; it cannot be the sole candidate mechanism.

Extend generic training contracts only where a provider-neutral concept is
missing. Keep Nomos Python modules, model layouts, device settings, and native
parameters inside the trusted Nomos adapter.

Every training attempt must pin:

- baseline checkpoint and base training population;
- approved proposal and delta selection;
- delta and combined snapshot;
- backend and implementation revision;
- all behavior-affecting parameters and seeds;
- finite time/resource budgets and observed usage;
- attempt/resume identity;
- produced checkpoint files and content fingerprints.

Use a small predeclared candidate set that tests clear hypotheses—for example
one conservative genuine fine-tune plus, only if useful, one interpolation
control. Do not launch a search sweep. Do not rewrite gates or hypotheses after
seeing results. A negative result is a valid outcome.

Training must be resumable without creating a second logical candidate or
spending the same reservation twice. Test crashes before and after checkpoint
creation.

## Milestone 4 — Evaluate and decide under unchanged evidence rules

Evaluate every candidate independently on both development suites:

- `generic_holdout`;
- `retired_post_scaling`.

A candidate is ineligible when any required suite is missing or any strict
suite gate fails. Do not average away a cohort-specific regression. Persist
separate reports, assessments, metric deltas, gate failures, and artifact
fingerprints.

If no candidate passes both suites:

- retain the baseline;
- leave the sealed generation unused;
- finish the run honestly;
- report which hypotheses failed and what remains uncertain.

If exactly one or more candidates pass both suites:

- select at most one by the predeclared deterministic policy;
- stop for explicit sealed-use authorization;
- verify the authorization against the exact candidate, reports, generation,
  journal head, and policy fingerprint;
- evaluate that candidate exactly once;
- atomically consume the generation and persist `promote_candidate` or
  `retain_baseline` under unchanged gates.

Never acquire a replacement sealed cohort automatically. Never retry sealed
evaluation as a new exposure. Recovery may complete the same reserved attempt
only.

## Milestone 5 — Productize the proven path as `encoder optimize`

After the real low-level experiment works, introduce the smallest thin,
durable application contract that composes the existing slices. It owns stage
lifecycle, links, reservations, approvals, budgets, and recovery—not copied
generation, quality, training, evaluation, analysis, or selection logic.

Provide one strict Nomos optimization manifest that resolves:

- trusted project and adapter identity;
- immutable baseline and training-input authority;
- development suites and strict per-suite gates;
- optional sealed generation authority;
- diagnosis and repair policies;
- candidate, training, evaluation, provider, and sealed-use budgets;
- approval mode;
- deterministic selection and final-decision policy.

Persist a finite lifecycle with precise durable states such as:

```text
planned
awaiting_repair_review
applying_repair
awaiting_delta_review
training
evaluating_development
awaiting_sealed_authorization
evaluating_sealed
completed
failed
cancelled
```

Each side-effecting stage must reserve its exact child artifact before doing
work. SQLite and immutable files are the source of truth. Resume may continue
only the reserved action; it may not silently create replacements or spend a
budget twice.

Expose a cohesive CLI family equivalent to:

```text
encoder optimize preview --manifest <FILE>
encoder optimize start --manifest <FILE>
encoder optimize status <RUN_ID>
encoder optimize inspect <RUN_ID>
encoder optimize review-repair <RUN_ID> ...
encoder optimize review-delta <RUN_ID> ...
encoder optimize resume <RUN_ID>
encoder optimize authorize-external <RUN_ID> ...
encoder optimize authorize-sealed <RUN_ID> ...
encoder optimize cancel <RUN_ID>
encoder optimize doctor <RUN_ID>
encoder optimize provenance <RUN_ID>
encoder optimize report <RUN_ID>
```

`preview` is read-only and resolves a complete fingerprinted definition.
`start` is idempotent. `resume` performs only the next legal reserved action.
Approval commands append immutable decisions and reject stale artifacts. The
operator should not have to manually copy internal artifact IDs when the run
can resolve them unambiguously.

At every pause, status must explain in plain language:

- what completed;
- what failed or remains uncertain;
- which immutable artifacts were created;
- which budgets remain;
- why execution stopped;
- whether human authorization is required;
- the exact safe next command.

Keep all lower-level slice commands available for independent use and
debugging.

## Milestone 6 — Reports, evidence, and recovery proof

Produce a deterministic final management report and a machine-readable,
row-free evidence bundle.

The report must explain:

- baseline and exact project revision;
- diagnosed weaknesses and their development evidence;
- approved repair hypothesis;
- data and training changes actually made;
- candidate checkpoints created;
- per-suite baseline and candidate results;
- why every candidate passed or failed;
- whether sealed evidence was used and exactly once;
- final promotion/retention decision;
- budget use, interruptions, retries, and approvals;
- known evidence limits and the next safe action.

The evidence bundle contains immutable identities, fingerprints, policies,
decisions, and relative artifact references—not raw training, development, or
sealed content. Verification on the same trusted checkout must detect missing
files, changed revisions, tampered database records, foreign artifacts,
incomplete journals, and reordered evidence.

Add deterministic offline tests for at least:

- sealed evidence rejected from adaptive diagnosis and delta construction;
- incomplete and tampered native row assessments;
- exact, normalized, source, group, and lineage contamination;
- stale reviews, proposals, delta selections, and run definitions;
- duplicate apply/start/resume requests;
- trainer crashes before and after checkpoint creation;
- evaluator crashes before and after report creation;
- cancellation at every side-effecting stage;
- retry without duplicate artifacts or budget spend;
- a candidate failing only one development suite;
- missing suite evidence remaining ineligible;
- sealed authorization mismatch and repeated authorization;
- atomic sealed consumption and decision persistence;
- migration from the current production database schema;
- Doctor and evidence-bundle tamper detection;
- reproducible final report output.

Include CLI end-to-end tests with deterministic fake adapters for both terminal
paths:

1. development failure produces `retain_baseline` without sealed use;
2. all development gates pass, explicit sealed authorization occurs, and a
   deterministic final decision is persisted.

Then run the same workflow through the real Nomos operator surface.

## Architecture and execution rules

- Read the required platform, architecture, development, and active slice
  specifications before changing implementation.
- Inspect current uncommitted native-delta work first and preserve valid work.
- Map missing ownership to existing crates before adding a crate.
- Add a new core crate only when no existing owner can express the missing
  provider-neutral lifecycle without reversing dependencies.
- Keep domain code independent from SQLite, CLI, Python, Nomos, SDKs, and
  presentation.
- Keep adapters, persistence, orchestration, and presentation separate.
- Keep all evidence append-only, fingerprinted, and deeply replayable.
- Derive status, metrics, decisions, and reports from persisted facts.
- Do not put business logic in `main.rs`, a generic `services.rs`, or a generic
  `utils.rs`.
- Use deterministic fakes in ordinary tests. Ordinary tests must not require
  Nomos, network, downloads, GPU, credentials, or paid providers.
- Work in coherent stages. After each stage run `cargo fmt-check`,
  `cargo check-all`, `cargo lint`, and `cargo test-all`, review dependency
  direction, and commit the working change.
- Set `CARGO_INCREMENTAL=0` for the Rust gates to avoid rebuilding the previous
  oversized incremental cache.
- Never reset or overwrite unrelated user changes.

Recommended order:

1. Finish and commit native repair-delta quality core, persistence, migration,
   tests, and inspection surfaces.
2. Implement and commit the deterministic isolated-Nomos repair recipe and
   contamination audit.
3. Create, qualify, review, and freeze the real delta and combined snapshot.
4. Add genuine training from exact approved inputs and recovery semantics.
5. Train the bounded real candidate set.
6. Evaluate both development suites and reach the correct pre-sealed outcome.
7. If eligible, stop for explicit sealed authorization and then finish exactly
   one governed assessment.
8. Productize the proven flow as the durable `optimize` contract and CLI.
9. Add fake end-to-end, recovery, migration, and tamper tests.
10. Re-run the real flow through the operator surface.
11. Verify repository isolation, secrets absence, provenance, and every gate.
12. Update documentation and finish with exact staged commits.

## Explicit non-goals

Do not add GUI/TUI, HTTP endpoints, cloud deployment, distributed workers,
authentication, multi-user support, an open plugin loader, arbitrary shell
execution, automatic benchmark acquisition, an endless autonomous loop,
reinforcement learning, bandits, automatic gate changes, automatic approval,
semantic deduplication, or token-saving encoder fleets.

Do not make an LLM authoritative for metrics, quality acceptance, candidate
selection, sealed use, or promotion. Do not add a research or prompt-repair
agent to this goal. Do not broaden the experiment because a candidate fails.

Promotion is not required. A deeply verified `retain_baseline` decision is a
successful result.

## Completion criteria

This long-range goal is complete only when:

1. The native repair-delta quality boundary is complete, persisted,
   migration-safe, payload-free, and deeply verifiable.
2. A reviewed, contamination-free targeted Nomos delta and immutable combined
   snapshot exist without changing base data.
3. At least one genuinely trained Nomos checkpoint derives from those exact
   approved inputs with complete provenance and bounded recovery semantics.
4. Every candidate has independent results for both development suites, and a
   failure in either suite makes it ineligible before sealed use.
5. The real experiment reaches a correct, deeply verified
   `promote_candidate` or `retain_baseline` decision under unchanged rules.
6. One manifest and one cohesive CLI command family can reproduce the proven
   journey without manual low-level UUID wiring.
7. Idempotency, staleness, budgets, cancellation, crash recovery,
   contamination, migration, sealed isolation, tamper detection, Doctor,
   reporting, and both fake terminal paths have deterministic offline tests.
8. The original Nomos repository has exactly its initial HEAD and working-tree
   state; the isolated copy has no remote and no credential.
9. `cargo fmt-check`, `cargo check-all`, `cargo lint`, `cargo test-all`, the
   isolated Nomos integrity checks, and relevant Python audit tests all pass.
10. Every coherent stage is committed, and documentation describes both the
    low-level contracts and the operator workflow.

Finish with a concise management summary separating:

- reusable platform capabilities added;
- operator workflow improvements;
- real Nomos repair hypothesis and approved delta;
- training changes actually tested;
- per-development-suite results;
- sealed result, if legitimately obtained;
- whether the production baseline changed;
- remaining evidence limitations;
- exact Encoder Gym and isolated Nomos commits;
- the next safe action.
