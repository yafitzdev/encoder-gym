# Production encoder experiment specification

## Purpose

The first production pilot uses the isolated Nomos/Fitz-Tool copy to prove that
Encoder Gym can improve an encoder whose task is not fixed-label text
classification. Nomos ranks a row-specific legal tool registry from an
objective and agent state, may abstain, and is ultimately judged by agent-level
completion. Flattening it into a global class label would invalidate the
experiment.

The production extension must preserve the existing classification slices.
It adds a narrow compiled task-adapter boundary for production encoder projects
whose native row, trainer, predictor, and metric contracts differ.

## Ownership boundary

Encoder Gym owns:

- immutable source, input, suite, baseline, candidate, and model identities;
- finite candidate, training-time, development-evaluation, and sealed-use
  budgets;
- normalized metric definitions and direction;
- deterministic absolute, improvement, and regression gates;
- development-only candidate selection;
- exactly one explicitly authorized sealed evaluation for the selected
  candidate;
- durable lifecycle, provenance, recovery, and promotion evidence.

A compiled task adapter owns:

- validation and normalization of its native dataset rows;
- conversion of an Encoder Gym candidate into native trainer inputs;
- local trainer invocation and model packaging;
- native prediction/evaluation execution;
- strict normalization of native output into the declared metric contract.

Task-specific row structures, subprocess handles, Python types, CUDA types, and
filesystem paths do not cross the core port. V1 uses a statically composed
Nomos adapter. It does not load arbitrary code or accept operator-authored shell
commands.

## Nomos pilot facts

The isolated workspace is
`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment`. It has no Git
remote and contains independent copies of the initial production artifacts.
The source Nomos repository is read-only for this pilot.

The copied FP32 ONNX baseline reproduces the 1,000-row generic development
holdout:

| Metric | Value |
|---|---:|
| Recall@1 | 0.836 |
| Recall@2 | 0.906 |
| Recall@3 | 0.942 |
| MRR | 0.8956535714285719 |
| Mean positive margin | 0.21479683880507947 |

The decisive historical agent evidence is 23/32 completed sessions for raw
top-three proposals and 26/32 for the complete coprocessor. A prior 25,000-row
scaling experiment improved several retrieval metrics but reduced raw agent
completion. Therefore Recall@k alone cannot authorize promotion.

## Finite experiment flow

```text
verify isolated project and immutable inputs
  -> reproduce baseline development report
  -> materialize a finite candidate set
  -> train each candidate into its own output directory
  -> evaluate each candidate on development evidence
  -> apply all deterministic safety/regression gates
  -> select the best passing candidate by the declared primary metric
  -> explicit sealed-evaluation authorization
  -> evaluate the selected candidate once on sealed evidence
  -> promote or retain the baseline deterministically
```

Sealed rows, traces, disagreements, and diagnostics are never candidate inputs.
A sealed report can decide final acceptance but is structurally rejected from
candidate selection.

## Initial Nomos candidate space

The first loop is deliberately bounded around the failure found by the previous
scaling experiment. Candidate parameters may cover triplet versus listwise or
multiple-negatives training, learning rate, margin, query representation,
positive representation, replay share, and finite curriculum membership. The
Nomos adapter owns validation of those parameter names and combinations.

The first loop must use existing copied development evidence to choose changes.
It must not repeatedly query the sealed holdout or generate another large
paraphrase cohort before a small controlled result establishes value.

## Completion criterion

The pilot is complete only when Encoder Gym can register and verify the copied
Nomos snapshot, reproduce the baseline, execute and persist a finite candidate
experiment through the Nomos adapter, normalize ranking and agent-level
metrics, select without sealed evidence, run at most one selected-candidate
sealed assessment, and deterministically promote an actually better candidate
or retain the baseline. The original Nomos repository must remain byte- and
Git-status unchanged.

Ordinary tests use a deterministic fake adapter. The real Nomos process test is
local, explicit, and uses only the isolated copy.
