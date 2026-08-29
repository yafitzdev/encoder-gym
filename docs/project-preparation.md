# Declarative project preparation

Project preparation turns one strict TOML or JSON manifest into the existing
slice artifacts needed to start a finite encoder-development workflow. It is a
compiler and transaction boundary, not a new workflow engine.

The manifest owns:

- the synthetic training-dataset schema and replaceable generation backend;
- exact total and reserved row budgets plus balanced, weighted, constrained,
  or explicit cell allocation;
- immutable development and optional sealed snapshot/split sources;
- cohort roles, disclosure levels, adaptation eligibility, and leakage limits;
- development and sealed acceptance contracts; and
- training/evaluation settings, iteration governance, stop policy, and finite
  row/request/advisor/stage budgets.

Start from `examples/project-preparation.toml`. First create or import the
benchmark dataset and an immutable snapshot, then replace the example snapshot
UUID. Snapshot labels must match the project label order exactly, and the
selected split must contain rows.

```text
synth project preview project-preparation.toml
synth --output json project preview project-preparation.toml
synth project prepare project-preparation.toml
synth project show <PREPARATION_ID>
synth provenance project-preparation <PREPARATION_ID>
```

Preview is read-only at the domain level. It reports every generation cell and
target, initial/reserved totals, estimated initial requests, configured
backends, cohort counts, contamination status, disclosures, governance mode,
and the finite stage graph. UUIDs and timestamps generated only during artifact
creation are intentionally absent, so repeated previews are stable.

Prepare reruns the same validation and inserts one bundle in one SQLite
transaction: dataset, default configuration plan, non-secret backend settings,
resolved project configuration, cohorts and roles, contamination reports,
benchmark suites, workflow definition, and preparation summary. A failure at
any point rolls back the whole bundle. The stable manifest fingerprint is
unique; repeating the same manifest returns the existing preparation even
though a fresh compilation would otherwise generate new artifact UUIDs.

Preparation rejects missing or changed snapshots, duplicate snapshot/split
assignments, label-order mismatches, empty selected splits, incompatible cohort
origins/roles, unsafe sealed disclosure, invalid budgets, infeasible initial
allocation, and contamination above policy. It never reads API keys and never
starts generation, training, evaluation, or sealed acceptance.

The successful command prints the separate authorization needed to execute:

```text
synth workflow start <DEFINITION_ID>
```

The generated definition uses the normal workflow contracts. Later code can
replace this manifest compiler without changing any slice implementation or
historical artifact.
