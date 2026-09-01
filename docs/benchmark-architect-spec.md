# Benchmark Architect specification

## Objective

Design trustworthy, renewable evaluation evidence before benchmark rows become
workflow authority.

```text
task semantics + deployment risks + operator objectives
  + aggregate candidate/benchmark summaries + exposure history
  + bounded external research
  -> evidence-backed benchmark blueprint
  -> deterministic feasibility and safety validation
  -> append-only human review
  -> immutable acquisition handoff
  -> ordinary cohort import/preparation/qualification
```

The Benchmark Architect answers what the encoder must prove, where it is likely
to fail in real use, how much evidence each decision needs, and when adaptive
evidence must be replaced. It does not create or label rows and it does not
decide whether a model passes.

## Authority boundary

Pi is a bounded advisory research process. Application-owned tools may expose
the immutable brief, aggregate cohort-readiness facts, aggregate exposure-risk
summaries, permitted web search/fetch results, recorded evidence, and a pure
blueprint preview. The agent receives no benchmark row text, predictions,
member identities, source paths, or sealed diagnostics.

The agent cannot:

- create, import, relabel, inspect, or mutate benchmark rows;
- read sealed row content or adaptive diagnostics derived from sealed rows;
- lower or waive contamination, qualification, or acceptance requirements;
- approve its own blueprint or create a `BenchmarkBundle`;
- tune thresholds after observing sealed results;
- start generation, training, evaluation, optimization, or a workflow.

The host treats every proposal as untrusted. `workflow-core` acceptance,
protocol, contamination, and qualification validators remain authoritative.
An approved handoff is a sourcing contract, not executable benchmark authority.

## Immutable brief

One resolved brief pins:

- task name, ordered labels, optional semantic and authenticity context;
- deployment surfaces, users, regions, languages, channels, time horizon, and
  concrete failure consequences;
- ordered operator objectives and relative weights;
- aggregate candidate-source summaries, if available;
- aggregate summaries of an existing benchmark bundle and qualification;
- exposure-risk summaries for existing cohorts;
- source-domain/class policy;
- finite model-turn, tool-call, search, fetch, byte, preview, token, cost, and
  wall-clock budgets;
- Pi provider/model identity and an API-key environment-variable name only.

Existing sealed evidence is represented only by immutable authority pins,
population counts, support/distribution summaries, and exposure risk. The
brief schema has no field capable of carrying row text or predictions.

## Research evidence

Search and page fetch use the existing replaceable provider-neutral research
ports. Fetched content remains explicitly delimited untrusted input. A source
becomes durable benchmark-design evidence only through an explicit record that
pins URL, content hash, bounded excerpt, source class, observation,
applicability, risk category, confidence, retrieval time, run, and tool call.

Blueprint claims name their supporting and conflicting evidence. Unsupported
claims must be marked as inferences. Source diversity and evidence conflicts
remain visible to review; an agent summary cannot erase them.

## Blueprint

A proposal defines explicit development and optional sealed suite blueprints.
Each suite pins:

- its role and purpose;
- one or more named acquisition cohorts;
- cohort origin, disclosure, and adaptation eligibility;
- an evaluation protocol;
- minimum total and per-label support;
- required dimension values and canonical slices;
- desired source classes and producer diversity;
- freshness limits based on age, workflow iterations, and evidence exposures;
- an acceptance contract containing explicit metrics, support, thresholds, and
  optional regression requirements.

The proposal also contains a benchmark-qualification policy, real-world risk
coverage, assumptions, evidence gaps, trade-offs, and evidence-backed rationale.
Every cohort and risk requirement has a stable key so later acquisition can
report exact satisfied and missing requirements.

## Deterministic validation

Preview and submission run the same pure validator. At minimum it rejects:

- task or label mismatch with the brief;
- missing development evidence, duplicate keys, invalid roles, or unsafe sealed
  disclosure/adaptation settings;
- malformed evaluation protocols or slice identities;
- unknown labels and incomplete per-label acquisition coverage;
- vacuous or invalid acceptance contracts;
- support below the declared protocol, metric, qualification, or conservative
  binomial uncertainty floor;
- thresholds outside metric domains or contradictory minimum/maximum values;
- freshness policies that are unbounded or permit sealed adaptive use;
- claims that reference missing/foreign evidence;
- acquisition requirements that do not reproduce from the blueprint.

Validation reports normalized blocking issues and warnings. Warnings identify
limitations such as unverified reference distributions, missing external
sources, or uncertain real-world prevalence. Neither Pi nor a review can turn a
blocking preview into an acquisition handoff.

## Review and acquisition handoff

Runs are durable and finite: `queued`, `running`, `awaiting_review`, `failed`,
or `cancelled`. Tool calls, evidence, usage, stop reason, proposal, reviews, and
handoff are persisted append-only facts. Recovery does not replay an unknown
paid turn.

Reviews are `approve`, `reject`, or `request_revision`. Only the latest
reproducible approval of a valid proposal can create the immutable acquisition
handoff. The handoff contains:

- the exact approved blueprint and authority fingerprints;
- one normalized sourcing requirement per proposed cohort;
- exact support, label, slice, distribution, source-diversity, and freshness
  requirements;
- research evidence/claim bindings and unresolved evidence gaps;
- an explicit statement that normal contamination, benchmark construction,
  qualification, and approval still apply.

Applying a handoff never creates a suite, cohort, snapshot, bundle,
qualification, workflow definition, or exposure. Those remain owned by their
existing components. A deterministic conformance check may later compare
prepared candidate summaries to the handoff and report satisfied, missing, or
stale requirements without reading sealed rows.

## Freshness and renewal

Freshness is cohort-specific and exposure-aware. Development cohorts may set a
maximum age, maximum adaptive exposures, and maximum workflow iterations.
Sealed cohorts allow only aggregate acceptance exposure and default to one
assessment before renewal is required. Manual disclosure continues to trigger
the existing mandatory retirement policy.

The architect may recommend a replacement and issue a successor acquisition
handoff. It cannot mutate the active bundle. A new workflow definition must bind
a newly constructed, clean, qualified, and independently approved bundle.
Historical definitions and results remain immutable.

## CLI-first acceptance

The CLI provides brief validation, start/status/watch/cancel/recover, evidence
and proposal inspection, append-only review, handoff creation/show, and
candidate-conformance inspection. Scripted Pi turns exercise the real JSONL Pi
process with fake search/fetch adapters. Ordinary tests use no network, paid
model, model download, GPU, or credential.

No graphical UI, TUI, HTTP endpoint, benchmark generation, automatic
acquisition, automatic approval, distributed worker, or open-ended agent loop
is part of this capability.

## Operator journey and ordinary preparation handoff

The checked-in offline journey is:

```text
synth benchmark-architect brief-validate examples/benchmark-architect/support-benchmark-brief.json
synth benchmark-architect start examples/benchmark-architect/support-benchmark-brief.json --script examples/benchmark-architect/support-scripted-turns.json --corpus examples/benchmark-architect/support-corpus.json
synth benchmark-architect evidence <RUN_ID>
synth benchmark-architect proposal <RUN_ID>
synth benchmark-architect review <PROPOSAL_ID> --approve --reason "evidence and renewal policy reviewed"
synth benchmark-architect handoff <PROPOSAL_ID>
synth benchmark-architect conformance <HANDOFF_ID> --file acquired-cohort-facts.json
```

Acquisition remains an operator-owned activity. For each handoff cohort key,
collect candidate rows through the existing import or dataset path, create an
immutable snapshot, and produce only row-free aggregate candidate facts for
the conformance command. A conformant result means the candidate meets the
architect's sourcing contract; it does not make the candidate trustworthy.

The ordinary stewardship path must still:

1. define active cohort roles with the required disclosure and adaptation
   eligibility;
2. run strict cross-cohort and training-population contamination checks;
3. create development and optional sealed suites with the handoff's exact
   protocols and acceptance contracts;
4. construct the immutable benchmark bundle;
5. compute deterministic qualification from persisted snapshot members and
   record a separate human qualification approval; and
6. prepare a new workflow definition that pins that exact clean, qualified,
   approved bundle.

Age, adaptive exposure, acceptance exposure, retirement, and workflow
iteration counters belong in the conformance facts. Once a limit is exceeded,
the candidate is blocked and must be replaced through a successor acquisition
handoff and a new ordinary bundle. Existing workflow definitions keep their
original immutable authority; neither `handoff` nor `conformance` rewrites or
hot-swaps it.
