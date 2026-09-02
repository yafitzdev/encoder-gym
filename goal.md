# Goal — Renewable Production Optimization Campaigns

Build the next production milestone exposed by the first real Nomos experiment:
a safe, repeatable optimization campaign that can replace exhausted acceptance
evidence, evaluate candidates across multiple development cohorts, and run the
next bounded iteration without tuning against a revealed sealed holdout.

This is not another speculative smart layer. It is the missing evidence and
orchestration foundation required before `optimize` can honestly mean “keep
improving over time.”

## Starting evidence

The completed Nomos run
`c4e1b908-ed55-434e-928d-ffe321b7a799` selected a candidate that improved the
generic development suite but regressed sealed MRR and Recall@1. Encoder Gym
correctly retained the baseline.

That result creates three hard facts:

1. `nomos_post_scaling_holdout_v1` has served its single-use acceptance role and
   cannot be reused to tune or select another candidate.
2. A single generic development aggregate did not predict transfer to the
   post-scaling cohort.
3. Another adaptive iteration is legitimate only after a genuinely successor
   sealed cohort is frozen, contamination-checked, qualified, and approved.

Preserve these facts. Do not weaken the failed gates, test the previous runner-up
on the exhausted holdout, or relabel the rejected candidate as an improvement.

## Primary objective

Add a provider-neutral campaign lifecycle that composes existing benchmark,
experiment, evaluation, and governance contracts without copying their business
logic.

Conceptually:

```text
completed iteration + exposure ledger
  -> successor benchmark requirements
  -> independently acquired and qualified successor cohort
  -> explicit atomic benchmark-generation activation
  -> former sealed cohort becomes development-eligible
  -> finite multi-development candidate experiment
  -> one explicitly authorized successor-sealed assessment
  -> promote or retain deterministically
  -> repeat only with fresh authority and finite budgets
```

The campaign is an append-only chain of immutable benchmark generations and
experiment iterations. It may coordinate existing slice contracts, but it must
not become a god service or silently mutate historical projects, protocols,
runs, reports, cohorts, or decisions.

## Capability 1 — Benchmark generation and renewal

Represent benchmark generations explicitly. At minimum, persist:

- generation identity and predecessor;
- development cohorts and the active sealed cohort;
- exact bundle, qualification, approval, contamination, and freshness
  authority;
- exposure counts and reasons;
- lifecycle state such as `draft`, `ready`, `active`, `exhausted`, and
  `superseded`;
- immutable activation and retirement events;
- the campaign iterations that consumed the generation.

A sealed cohort becomes exhausted after its declared acceptance-use limit. An
exhausted generation must be rejected for new adaptive work. Activation of a
successor must be atomic and must require a clean, qualified, independently
approved benchmark bundle before the previous sealed cohort can become
development-eligible.

Do not mutate the old cohort's historical role. Record a new role assignment in
the successor generation while preserving every old workflow and report.

Use the existing Benchmark Architect acquisition handoff and ordinary benchmark
preparation contracts where they fit. Do not duplicate them. If a missing
handoff-to-preparation bridge is required, implement the smallest explicit
component that preserves ownership and human approval.

## Capability 2 — Multiple development suites

Extend production encoder experiments to support more than one named
development suite. Candidates must be evaluated independently on every declared
suite, with persisted per-suite reports and gates.

The protocol must declare:

- ordered development suite keys and immutable fingerprints;
- gates scoped to a specific suite or explicitly to all suites;
- a deterministic candidate eligibility rule;
- a deterministic primary selection rule across passing candidates;
- finite evaluation budgets that account for every candidate-suite pair;
- how missing, failed, or unsupported suite evidence fails closed.

Do not concatenate cohorts and hide a localized regression inside one weighted
average. Keep cohort results visible. The first Nomos successor protocol should
use the generic holdout and the retired post-scaling holdout as distinct
development suites only after a new sealed cohort is active.

Sealed evidence remains structurally incapable of entering candidate creation,
diagnostics, selection, ranking, or retry logic.

## Capability 3 — Campaign orchestration

Provide a small CLI-first campaign surface that makes the safe path easy without
hiding authority boundaries. It should support operations conceptually similar
to:

- create/show a campaign;
- show why the next iteration is ready or blocked;
- bind an approved benchmark generation;
- prepare and start a finite experiment iteration;
- advance deterministic development work;
- stop at explicit sealed authorization;
- finalize and show promotion/retention evidence;
- request or inspect the next benchmark-renewal handoff.

One convenience `advance` or `optimize` command may resume already-authorized
local work, but it must stop at every human approval boundary, sealed-use
authorization, missing successor, exhausted budget, or failed invariant. It
must be idempotent and recover the exact reserved action rather than starting a
replacement action.

Persist campaign state in SQLite with append-only, tamper-evident provenance.
Doctor and provenance inspection must reproduce the complete chain from source
artifacts through benchmark generation, protocol, reports, decision, exposure,
and successor requirements.

## Capability 4 — Real Nomos successor experiment

Use only the isolated copy at
`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment`.
The original `fitz-tool` repository remains read-only and must finish with its
original HEAD and exact pre-existing working-tree status.

Before adapting to the failed sealed result:

1. Audit whether any already-copied or source Nomos cohort is genuinely unused
   by training, model selection, calibration, or prior acceptance decisions.
2. Do not assume `nomos_live_calibration_v1/v2` is clean merely because it has a
   convenient name; prove lineage and exposure eligibility.
3. If no eligible cohort exists, create a new production-relevant successor in
   the isolated copy through a deterministic, reviewed, provenance-rich
   acquisition specification. Freeze its generator/specification, population,
   hashes, support, and contamination evidence before any new candidate is
   proposed. Do not inspect its rows or evaluate a candidate on it early.
4. Make no paid or network model call without explicit authorization. Ordinary
   tests and the default acceptance path remain offline.

After successor activation, run one real bounded Nomos campaign iteration. Use
the former generic and post-scaling cohorts as separate development suites,
choose a finite candidate set from development-eligible evidence only, evaluate
all strict production and agent gates, authorize at most one candidate on the
new sealed cohort, and persist the honest final decision.

Promotion is desirable but cannot be guaranteed. A valid retained-baseline
decision remains scientifically acceptable; a development-only improvement is
never called a production win.

## Architecture and implementation rules

- Read `docs/platform-spec.md`, `docs/architecture.md`, `docs/development.md`,
  `docs/benchmark-architect-spec.md`, and
  `docs/production-encoder-experiment-spec.md` before implementation.
- First map ownership onto existing crates. Add a new core crate only if no
  existing component can own the contract cleanly.
- Keep campaign orchestration, benchmark governance, experiment logic, native
  adapters, persistence, and CLI presentation separate.
- Preserve provider-neutral core contracts. Nomos paths, Python types, model
  formats, and evaluator details remain inside the Nomos adapter.
- Derive readiness, exposure, and progress from persisted facts, never frontend
  or process memory.
- Use deterministic fakes for ordinary tests. No ordinary test may require the
  Nomos copy, model download, GPU, network, or paid provider.
- Implement and commit one coherent component at a time. Run all standard Rust
  gates after each coherent stage.

## Non-goals

Do not add GUI, TUI, HTTP endpoints, cloud deployment, authentication,
multi-user collaboration, distributed workers, autonomous threshold tuning,
reinforcement learning, bandits, semantic deduplication, an unbounded agent
loop, automatic approval, or hidden sealed-row analysis.

Do not broadly refactor unrelated slices. Do not build a general plugin loader
or workflow language. Do not copy Nomos business logic into generic campaign or
experiment crates.

## Completion criterion

The goal is complete when:

1. An exhausted sealed generation is durably ineligible for another adaptive
   iteration.
2. A successor generation cannot activate without exact clean, qualified,
   approved, contamination-checked authority.
3. The old sealed cohort can become development-eligible only in that successor
   generation, without rewriting history.
4. A finite experiment evaluates every candidate against multiple independent
   development suites and rejects any suite-specific gate failure.
5. The campaign stops for explicit sealed authorization and permits exactly one
   selected-candidate assessment on the active successor sealed cohort.
6. Recovery, idempotency, budget accounting, tampering, migrations, provenance,
   and Doctor are covered by deterministic offline tests.
7. A real isolated Nomos successor iteration reaches a deeply verified
   `promote_candidate` or `retain_baseline` decision without touching the
   original Nomos repository.
8. `cargo fmt-check`, `cargo check-all`, `cargo lint`, `cargo test-all`, the
   explicit isolated Nomos integrity test, and any relevant Pi checks pass.

Finish with a management summary that distinguishes platform capability,
scientific result, remaining evidence limitations, exact commits, and the next
safe optimization opportunity.
