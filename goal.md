# Goal — Evidence-Driven Nomos Repair and Production Qualification

Turn the first renewable Nomos campaign into one real, bounded encoder
improvement cycle.

Do not add another speculative “smart” subsystem. Use the development evidence
we already earned to identify concrete weaknesses, repair the training input or
training recipe, train genuinely new candidate models, and qualify at most one
candidate against the still-unused successor sealed cohort.

The central question for this goal is:

> Can Encoder Gym convert persisted development failures into a reproducible
> model improvement without tuning against sealed evidence?

## Starting evidence

The completed production campaign is
`8a6e1004-a9df-4b3c-a127-42b121ed337c` in:

`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment\encoder-gym-final.sqlite`

It established these facts:

1. The 2.5% MNRL interpolation candidate passed `generic_holdout` but missed the
   minimum MRR improvement on `retired_post_scaling`.
2. The 5% candidate passed `retired_post_scaling` but failed the generic
   Recall@2 gate.
3. Neither candidate was eligible for sealed evaluation, so the baseline was
   correctly retained.
4. Successor generation `10cba5de-e501-4169-a603-27f75c2abd37` remains active,
   fresh, and unused, with zero candidate exposures.
5. Aggregate interpolation alone did not produce a candidate that transfers
   across both development populations.

Preserve these facts. Do not weaken gates, average away a cohort-specific
regression, infer sealed performance, or call either prior candidate a win.

## Primary objective

Implement and run a provider-neutral, evidence-driven repair path that composes
the existing slices:

```text
persisted multi-suite predictions and assessments
  -> deterministic comparative error analysis
  -> explicit finite repair proposal
  -> reviewed training-data delta and/or training-recipe candidate
  -> immutable candidate training inputs
  -> genuinely new trained checkpoints
  -> evaluation on every development suite
  -> deterministic candidate eligibility and selection
  -> explicit authorization for at most one sealed assessment
  -> promote or retain with complete provenance
```

This is one bounded campaign, not an autonomous forever loop. The resulting
components should be reusable for other encoder projects, while all Nomos file
formats, Python calls, labels, and model details remain inside the Nomos
adapter.

## Capability 1 — Comparative development diagnosis

Produce a deeply verified diagnostic artifact from persisted row-level
predictions for both development suites and the baseline plus prior candidates.

The diagnosis must make useful distinctions such as:

- errors shared by both development suites;
- errors unique to `generic_holdout`;
- errors unique to `retired_post_scaling`;
- regressions introduced by the 2.5% or 5% candidate;
- label, workflow, ambiguity, abstention, top-k, and confidence slices where
  supported by the source schema;
- candidate trade-offs where one metric improves while another production gate
  fails;
- minimum-support and uncertainty limits that prevent overreacting to tiny
  slices.

Use only development-eligible evidence. The active successor sealed rows,
predictions, metrics, and model behavior must remain structurally inaccessible
to diagnosis, candidate construction, ranking, and retries.

Do not ask an LLM to rediscover metrics already available deterministically.
An advisor may summarize or propose hypotheses only after the complete
deterministic evidence artifact exists, and any such use must be finite,
row-bounded, reviewable, and clearly marked as inference.

## Capability 2 — Explicit repair proposals

Compile diagnosis into a finite, reviewable repair proposal. Reuse the existing
error-analysis, optimization, allocation, generation, dataset-quality, and
training contracts wherever they already own the required logic.

At minimum, a proposal must pin:

- the exact source project, campaign, runs, reports, and diagnosis fingerprint;
- the failure groups it intends to improve;
- whether each action changes data, training settings, or both;
- explicit generation or import targets, including cell-level row counts;
- a hard total-row budget and per-group caps;
- duplicate, leakage, and quality policies;
- the exact base training snapshot and method for constructing the candidate
  training snapshot;
- a finite candidate set and training/evaluation budgets;
- expected development outcomes stated as hypotheses, not guarantees;
- an approval decision before applying any data or training mutation.

The proposal must fail closed if its evidence, training snapshot, coverage, or
benchmark authority changed after review. Applying the same approved proposal
twice must be idempotent.

Do not create an opaque scalar “quality score.” Keep the concrete failed gates
and targeted groups visible.

## Capability 3 — Provenance-rich training-data delta

Create a small, targeted training-data delta inside the isolated Nomos
experiment only. Prefer the smallest data change capable of testing the repair
hypothesis.

The data path must:

- generate deterministic fields without an LLM wherever possible;
- use explicit dimension cells and absolute targets;
- preserve source, prompt, backend, model, research-context, and repair-proposal
  provenance for every generated or imported row;
- validate schema, labels, dimensions, length, and task-specific invariants;
- perform normalized duplicate detection against both the delta and the base
  training population;
- prove no overlap or lineage contamination with either development suite or
  the active successor sealed cohort;
- pass the existing dataset-quality review before entering a training snapshot;
- produce an immutable delta snapshot and an immutable combined candidate
  training snapshot without mutating the base snapshot.

If useful real-world source material already exists in the isolated copy, it
may be imported only after proving its lineage and eligibility. Do not copy new
material from the original Nomos working tree merely because it is convenient.

No network or paid model call is authorized by this goal. If external generation
would materially improve the experiment, prepare the exact bounded request and
stop for explicit authorization. Continue with a meaningful offline path where
possible.

## Capability 4 — Genuine candidate training

Extend the Nomos experiment adapter only as needed to train candidate models
from an exact base snapshot plus an approved delta and explicit training
settings.

At least one candidate must be a genuinely retrained model whose immutable
artifact derives from the approved repair inputs. Do not make checkpoint
interpolation the only candidate mechanism.

Candidate identity must include:

- base model and training snapshot fingerprints;
- delta snapshot and repair proposal fingerprints;
- trainer/backend identity and implementation revision;
- all behavior-affecting hyperparameters and random seeds;
- training budget and observed usage;
- produced checkpoint files and content fingerprints.

Keep the generic training candidate contract provider-neutral. Nomos-specific
parameter names, Python scripts, checkpoint layouts, and evaluator invocation
belong in `encoder-experiment-nomos` or another narrowly owned Nomos adapter.

Use a deliberately small candidate set. A reasonable initial experiment is one
targeted data-repair candidate plus, only if scientifically useful, one bounded
training-setting variant or intermediate interpolation control. Do not launch a
large hyperparameter sweep.

## Capability 5 — Development-first production qualification

Run a new finite production campaign against the same active successor
generation while it remains fresh and unused.

Every candidate must:

1. train from its exact immutable inputs;
2. produce independent persisted reports for `generic_holdout` and
   `retired_post_scaling`;
3. pass every strict suite-specific production and agent gate;
4. remain ineligible if any report is missing, unsupported, or failed;
5. be ranked only among candidates that passed every development suite.

If no candidate passes both development suites, retain the baseline and leave
the successor sealed cohort unused again.

If exactly one candidate is selected, stop at the existing explicit sealed-use
authorization boundary. After authorization, evaluate that candidate exactly
once on `successor_sealed`, atomically persist consumption and the campaign
decision, and accept promotion only through the unchanged deterministic gates.

Do not automatically acquire another successor cohort in this goal. If the
sealed cohort is consumed, persist the renewal requirement and stop.

## Capability 6 — Operator experience and auditability

Keep the surface CLI-first and scriptable. Do not add GUI, TUI, or HTTP APIs.

The operator must be able to:

- inspect the comparative diagnosis;
- preview and review the exact repair proposal;
- inspect planned data additions and candidate training budgets;
- apply an approved proposal idempotently;
- start or resume the bounded campaign;
- see per-candidate, per-development-suite results;
- understand why a candidate was rejected;
- stop at external-call and sealed-use authorization boundaries;
- run Doctor and provenance from source evidence through diagnosis, proposal,
  data delta, training snapshot, checkpoint, reports, and final decision.

Progress and readiness must be reconstructed from SQLite and immutable files,
not process memory. Interrupted work must resume the exact reserved action and
must not silently create replacement rows, snapshots, candidates, or runs.

## Real Nomos experiment boundary

Use only:

`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment`

Treat this as a disposable but evidence-preserving experiment repository.

The original repository at
`C:\Users\yanfi\PycharmProjects\fitz-tool` is read-only. Before and after all
work, record its exact HEAD and working-tree status and prove they are
unchanged. Never configure a remote for the isolated copy, never push from it,
and never place credentials in either repository, SQLite, command history,
artifacts, logs, or documentation.

Preserve the existing authoritative campaign database. Prefer a new database
or an explicit append-only continuation strategy if experimentation could
damage the completed campaign evidence. Never rewrite the completed campaign.

## Architecture and implementation rules

- Read `docs/platform-spec.md`, `docs/architecture.md`, `docs/development.md`,
  `docs/error-analysis.md`, `docs/optimization.md`,
  `docs/dataset-quality-spec.md`, `docs/production-encoder-experiment-spec.md`,
  and `docs/renewable-production-campaigns.md` before implementation.
- Map the complete flow onto existing crate ownership before adding modules.
- Add a new core crate only if no existing slice can own the missing contract
  without violating dependency direction.
- Do not copy analysis, allocation, validation, training, evaluation, or
  campaign business logic into an orchestration module.
- Keep domain contracts independent from SQLite, Python, Nomos, CLI, SDKs, and
  presentation.
- Make all evidence and derived artifacts immutable, fingerprinted, and
  append-only.
- Use deterministic fakes for ordinary tests. No ordinary test may require the
  Nomos copy, network, model download, GPU, API key, or paid provider.
- Implement and commit one coherent component at a time. After every component,
  run the standard Rust gates and review public interfaces and dependency
  direction.

## Non-goals

Do not add an autonomous endless optimization loop, reinforcement learning,
bandits, automatic gate changes, automatic approval, semantic deduplication,
distributed execution, cloud deployment, authentication, multi-user support,
GUI, TUI, HTTP endpoints, or a general plugin/workflow language.

Do not train a fleet of token-saving encoders in this goal. Do not build a new
research agent unless a concrete missing evidence source makes the existing
research contracts insufficient. Do not broadly refactor unrelated slices.

Promotion is not a required outcome. Scientific honesty, reusable repair
contracts, and sealed-evidence isolation are required outcomes.

## Completion criterion

The goal is complete when:

1. A persisted, reproducible comparative diagnosis explains the important
   cross-suite failure and regression groups without accessing successor sealed
   evidence.
2. A finite approved repair proposal compiles that evidence into exact data
   and/or training actions and becomes stale if any pinned input changes.
3. A targeted, quality-reviewed training-data delta and combined immutable
   candidate snapshot are produced without mutating the base data.
4. At least one genuinely retrained Nomos candidate is derived from the exact
   approved repair inputs with complete checkpoint provenance.
5. Every candidate is evaluated independently on both development suites and
   any suite-specific failure rejects it before sealed use.
6. A new bounded campaign reaches a deeply verified `promote_candidate` or
   `retain_baseline` decision. Sealed evidence is consumed only if a candidate
   first passes both development suites and an operator explicitly authorizes
   the call.
7. Recovery, idempotency, finite budgets, contamination defenses, tampering,
   migrations, Doctor, and complete row-free provenance are covered by
   deterministic offline tests.
8. The original Nomos repository finishes at its original HEAD and exact
   pre-existing working-tree state; the isolated copy contains no secret and
   has no remote.
9. `cargo fmt-check`, `cargo check-all`, `cargo lint`, `cargo test-all`, the
   explicit isolated Nomos integrity test, and relevant Python freeze/audit
   tests all pass.

Finish with a management summary that clearly separates:

- platform capabilities added;
- the repair hypothesis and data/training changes actually tested;
- development-suite results;
- sealed result, if one was legitimately obtained;
- whether the production baseline changed;
- remaining evidence limitations;
- exact commits in Encoder Gym and the isolated Nomos copy;
- the next safe action.
