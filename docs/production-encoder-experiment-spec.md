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
  -> reproduce baseline development and frozen sealed-reference reports
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

The baseline sealed report is frozen into the immutable protocol before any
candidate work begins and is never exposed to candidate selection. The run's
sealed-use budget authorizes exactly one new report: the development-selected
candidate. Every external action is first reserved in a hash-chained durable
journal. Recovery may finish the same reserved local action, but it may not
create another candidate or sealed-report identity.

## Initial Nomos candidate space

The first loop is deliberately bounded around the failure found by the previous
scaling experiment. Candidate parameters may cover triplet versus listwise or
multiple-negatives training, learning rate, margin, query representation,
positive representation, replay share, and finite curriculum membership. The
Nomos adapter owns validation of those parameter names and combinations.

The first loop must use existing copied development evidence to choose changes.
It must not repeatedly query the sealed holdout or generate another large
paraphrase cohort before a small controlled result establishes value.

The first real loop trained two continued-triplet candidates. Both completed,
were recovered from their immutable output directories after a process-output
normalization failure, and regressed development MRR and Recall@1/2. Encoder
Gym therefore retained the baseline without evaluating either candidate on
sealed evidence. That result closes the naive "train the same rows again"
branch rather than weakening its gates.

The next bounded branch uses linear checkpoint interpolation. A candidate may
name only an immutable reference-model key declared by the isolated project;
the adapter verifies that copied tree and invokes one fixed interpolation
module. It rejects arbitrary paths, weights above 0.5, extra parameters, and
native manifests that do not reproduce the exact baseline, reference, weight,
and candidate output identity. This generalizes the trainer port to safe model
transformations without leaking Nomos-specific types into the core.

## Composite production evaluation

Nomos exposed that a single offline metric family is not sufficient for a
production encoder. The adapter therefore composes two native evaluators into
one normalized report:

1. frozen retrieval evaluation produces Recall@k, MRR, and positive margin;
2. deterministic local agent sessions produce completion, completed-stage,
   tool-selection, schema-validity, execution, invalid-call, wrong-execution,
   oracle-visibility, prompt-token, and description-reduction metrics.

The local ONNX chat model, evaluator configuration, suite pairing, session
count, and top-k policy are immutable project dependencies. Each suite records
separate retrieval and agent-component fingerprints plus a composite report
fingerprint. Raw evidence is content-addressed by model and component identity,
so changing only the agent policy reuses an unchanged retrieval report while a
model, dataset, evaluator-contract, chat-model, or agent-policy change creates a
different evidence path. Raw reports and traces remain below the isolated
experiment root. If an agent report exists without its trace, the adapter fails
closed instead of treating it as recoverable evidence.

The first four interpolation probes improved retrieval while exposing one
different third-ranked tool, which caused one additional wrong execution in the
development agent suite. A direct policy canary found that top-two visibility
was worse, while top-one with the existing two-attempt recovery completed all
16 development sessions and improved execution accuracy over top-three. The
baseline and the 2.5% triplet interpolation candidate produced identical
top-one agent outcomes. The next bounded protocol therefore declares top-one
plus recovery as its production inference policy. All downstream outcome gates
remain strict no-regression comparisons, including oracle visibility; the
policy change is applied symmetrically to baseline and candidates before the
new protocol is frozen.

The production protocol uses development agent outcomes as regression gates in
addition to retrieval gains. The promotion agent suite remains sealed and can
run only for the one development-selected candidate after explicit operator
authorization.

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

Development-only repair diagnosis is a separate post-experiment operation. It
uses complete native row observations for every baseline/candidate and
development-suite pair, reproduces normalized retrieval metrics within an
explicit floating-point tolerance, and persists text-free evidence plus a
deterministic comparative diagnosis. It cannot request the sealed suite.

A diagnosis may be compiled into one finite repair proposal. The proposal pins
the exact historical diagnosis and source run separately from the clean current
execution revision, exact diagnosed slice targets and absolute row counts,
base training inputs, explicit data/training actions, task-neutral native
quality policy, candidate hypotheses, all finite budgets, expiry, and the
active renewable benchmark generation plus its journal head. Operator reviews
are append-only. Approval becomes stale when any pinned project, evidence,
policy, proposal, or benchmark authority changes. Applying an unchanged exact
approval only reserves one content-addressed application; it does not yet
generate rows or train a model.

## First completed production loop

The first complete protocol ran on 2026-09-02 with project
`2452cb9f-6ffe-4c27-82d2-3b966dadf9cf`, protocol
`a7e4c189-5e67-406d-9ba2-6851bada70ef`, and run
`c4e1b908-ed55-434e-928d-ffe321b7a799`. Both finite triplet-interpolation
candidates passed every development gate. The selected 5% candidate improved
development MRR from `0.8956535714` to `0.8980035714`, Recall@1 from `0.836`
to `0.840`, Recall@2 from `0.906` to `0.908`, and mean positive margin from
`0.2147968518` to `0.2222092217`, with identical agent outcomes.

The one authorized sealed report did not confirm the ranking improvement. MRR
changed from `0.9664993081` to `0.9656004384`, and Recall@1 changed from
`0.9597222222` to `0.9583333333`; all measured agent outcomes were identical
and margin improved. The strict MRR and Recall@1 gates failed, so the durable
decision is `retain_baseline`. The selected candidate remains an immutable
rejected artifact with model fingerprint
`sha256:95943e3eeeb1224cc8da28cdf1c448545ef2e3c33031fe06aa28b8ac99ff856c`.

This is a successful validation of the platform contract, not a promoted
encoder improvement. Encoder Gym rejected a plausible development-only win,
preserved the original model, and exhausted exactly the declared sealed-use
budget. The revealed sealed cohort may not be used to try the runner-up or tune
another interpolation weight. A subsequent adaptive loop requires a declared
successor acceptance cohort; the old cohort can enter development only after
that replacement is frozen and its role change is explicit.
