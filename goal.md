# Goal — Benchmark Stewardship and Decision-Grade Evaluation

Build the trust boundary that makes encoder iteration scientifically useful.
An immutable benchmark bundle is necessary, but it is not sufficient: every
training population must be proven clean against that exact bundle, and every
benchmark must eventually be shown capable of answering its declared
acceptance questions.

Do not add a graphical UI, HTTP API, distributed execution, automatic benchmark
approval, hidden threshold changes, or an adaptive agent with access to sealed
row content. Follow `docs/benchmark-stewardship-spec.md` as the authoritative
capability specification.

## Product flow

```text
task and semantic contract
  -> candidate development and sealed evidence
  -> immutable benchmark bundle
  -> benchmark qualification and explicit approval
  -> generated/curated training snapshot
  -> mandatory training <-> benchmark leakage check
  -> training only when the exact check is clean
  -> development iteration under exposure budgets
  -> one aggregate-only sealed assessment
```

The first implementation milestone is the leakage firewall. For every initial
and iterative snapshot, compare all `train` and non-empty `validation` members
with the exact development plus optional sealed cohort union. Persist a
zero-tolerance report and an immutable check binding before any trainer or
previous training run can be used. Source-row, exact-text, normalized-text, and
configured group overlap all fail closed. There is no override path.

## Required architecture

- Reuse `workflow-core` contamination contracts; do not create a second text
  normalization or overlap algorithm.
- Represent the training inputs as immutable internal `Training` cohorts, one
  per consumed snapshot split, and pin their active role decisions.
- `TrainingBenchmarkCheck` binds the verified snapshot population, training
  cohorts, exact `BenchmarkBundle`, exact contamination report, status, and
  fingerprint.
- Persist newly required cohorts, roles, report, and check atomically. Repeated
  attempts reuse the same deeply verified authority.
- Gate both initial and iterative training before querying or invoking a
  backend. Evaluation and promotion reject a missing or mismatched check.
- Keep findings free of raw benchmark text. A blocked check remains inspectable
  but cannot be overridden into eligibility.
- Index contamination comparisons so realistic dataset sizes do not require a
  quadratic all-pairs text scan.

## Long-range stewardship

After the firewall is complete, add deterministic benchmark qualification:
contract-support feasibility, label and slice coverage, representativeness,
uncertainty/power evidence, source quality, and adaptive-use budgets. Then add
a bounded Benchmark Architect that may propose cohorts, targets, and acceptance
contracts from task semantics. The agent is advisory only; deterministic code
validates evidence, sealed content never enters its context, and a human freezes
the exact decision contract.

## Acceptance

Prove that source-ID, exact-text, normalized-text, validation-split, and group
leakage stop a workflow with zero training runs or checkpoints. Prove clean
checks are reusable, iteration snapshots are checked independently, unused
internal test rows are outside the trainer-input protocol, tampering and stale
roles fail closed, provenance reaches the snapshot and bundle, Doctor verifies
the full chain, and all Rust quality gates pass.

Implement and commit coherent stages. Follow `AGENTS.md`,
`docs/benchmark-stewardship-spec.md`, `docs/architecture.md`, and
`docs/development.md`.
