# Goal: make one-click agentic encoder optimization work in the app

The user opens a project's Overview, selects a baseline, starting dataset,
evaluation benchmark, Agent model and Data generation model, then presses
Optimize once. The Agent investigates development failures, proposes justified
dataset changes, requests generation, trains and evaluates a candidate, and
uses that result to decide the next bounded iteration.

Finish and connect the existing engine. Preserve the approved GUI and working
slice contracts. More settings, System narration, mockups or documentation alone
do not satisfy this goal. A completed experiment may reject every candidate:
model improvement is not guaranteed.

## Current status — 15 September 2026

**The one-click agentic desktop journey is not ready.** The production CLI has a
tested Agent loop; the desktop still needs compatible lifecycle handling and
connection to that coordinator.

| Capability | Verified checkpoint | What remains |
| --- | --- | --- |
| Agent inspection, separate generation, immutable edits and native data qualification | `18fc85b` | Preserve and reuse |
| One complete Agent-directed training/evaluation cycle | `a8bb5c4` | Preserve and reuse |
| Candidate registration, exact training-dataset links and benchmark report lookup | `6d90ead` | Verify navigation through the connected app |
| Evidence-dependent iterations, best eligible dataset selection and no-change completion | `f3d222b` | Complete cumulative accounting and interruption coverage |
| Root execution attempts and adaptive completion | `3e18b6c` | Integrate lifecycle and recovery into the desktop |
| Durable Stop and explicit Resume | `9ea9dc4`; all four Rust gates and 15 managed benchmark tests pass | Interrupted-training accounting, descendant ownership, every-boundary recovery and desktop controls |
| One-click Overview execution | Not verified end to end | Update the strict parser, connect the existing coordinator and test the rendered journey |

The committed CLI checkpoints passed their required Rust gates. Prior UI tests
and rendered Overview fixture checks passed, but fixture captions do not prove
that Optimize invokes the Agent. Do not transfer a checkpoint's test results to
later uncommitted changes. The Stop/Resume checkpoint also passed UI typecheck,
all 125 UI tests, nine native-clearance Python tests and rendered Overview
fixture checks. These are not connected Agent-loop desktop acceptance tests.

Run 17 (`998d54fb-363f-443f-abeb-195dd2574d79`) established fixed training and
evaluation, not Agent optimization: 6,800 existing rows, one epoch on CUDA, both
development suites failed, baseline retained, final holdout unused. Its 4,431
activity events contained no Agent-origin entries. System templates and
`agent_*` evaluation metrics are not evidence of optimization-Agent calls.

Latest implementation checkpoint: `9ea9dc4`. Interrupted-training accounting
has been investigated but is not implemented: fresh native training dispatch
still receives the candidate's original time ceiling on each retry. Do not
describe Stop/Resume as fully budget-safe until cumulative enforcement passes.

This update is documentation-only. Do not continue implementation, start real
Nomos training, make paid calls or expose protected evaluation in this turn.

## Immediate next deliverable

Make the existing Overview execute and display the tested Agent loop, with
working Stop/Resume and truthful activity. Do not rebuild the loop or introduce
another executor.

1. **Preserve the verified Stop/Resume checkpoint.** Stop has a stable command
   UUID; Resume pins the observed execution head. The production-CLI regression
   rejects stale Resume, preserves interrupted-call charges and reuses completed
   work. Do not rebuild it or equate this coverage with complete recovery.
2. **Implement interrupted-training accounting next.** Deliver one bounded,
   production-CLI-connected checkpoint:

   - Persist a training-attempt reservation before fresh native training starts.
     Enforce the remaining iteration and run allowances across retries without
     changing immutable candidate, protocol or dataset identities.
   - Record time consumed by successful, interrupted, failed and timed-out
     attempts. Unresolved attempts retain conservative charges; Resume must
     never reset the allowance or treat unknown consumption as zero.
   - Reuse verified completed artifacts without dispatching or charging a new
     training attempt. Keep training accounting distinct from qualification and
     evaluation time. Budget exhaustion needs a typed outcome, not a candidate
     rejection or an endlessly retryable generic error.
   - Prove these cases through deterministic production-path tests before
     claiming the checkpoint complete. Do not promise optimizer-state recovery:
     unfinished training may restart, but only within the remaining allowance.

3. **Close the remaining recovery gaps before exposing controls.**

   - Reconcile coordinator and descendant-process ownership after abrupt exit.
     A dead coordinator does not prove its children stopped.
   - Test interruption around publication, qualification, training, candidate
     registration and each development report.
   - Reconcile activity with durable outcomes; distinguish paused work,
     exhausted budgets, execution failure and successful experiment completion.
   - Preserve the verified control-intent fence through desktop integration:
     a stale Resume cannot clear a new Stop; retrying an old Stop cannot halt
     an already resumed attempt.
4. **Connect the desktop.** Extend `ui/src/input-optimization.ts` to accept the
   actual Agent execution states and `agentExecution` record. Update lifecycle
   handling and the launch controller to invoke the existing `drive-agent`
   coordinator. Preserve historical fixed-run compatibility.
5. **Prove the connected journey.** Use production orchestration with
   deterministic provider/native adapters to demonstrate one-click execution,
   an evidence-dependent second iteration, linked artifacts, complete activity
   and Stop/app-restart/Resume.

Do not ask the user to start another real Nomos run to discover whether the
Agent is connected. Deliver these checkpoints in bounded increments, reporting
what is component-tested, CLI-integrated and app-verified.

### Verified Stop/Resume checkpoint

The CLI supports `stop-agent --request-id <UUID>`, explicit
`drive-agent --resume <EXECUTION_HEAD>` and `reconcile-agent`. Clients reuse the
Stop UUID for retries; each new Stop changes the head even while paused. Resume
checks the observed head in the write transaction, rather than adopting a newer
intent. Stop intent is persisted before pause acknowledgement; reconciliation
does not start work or displace a live owner.

Focused tests cover queued Stop, an interrupted active Agent call, duplicate
worker rejection, preservation of completed inspection, conservative unknown
provider usage, interruptible local/native reads, owned-child termination and
scientific interruption without falsely failing the candidate.
The expanded process test also rejects an old Resume after another Stop and
proves that retrying an old Stop after Resume leaves the new attempt running.
All four required Rust gates passed on the unchanged executable tree, including
the 15 managed benchmark tests and the legacy optimization recovery suite.

These tests do not establish every-boundary recovery, safe orphan reconciliation
or cumulative interrupted-training budgets. Unfinished training may restart;
do not promise optimizer-checkpoint continuation. Stored `running` state is
not by itself proof of a live worker.

## Required execution contract

1. Pin the exact baseline, starting dataset version, benchmark version, selected
   provider connections/models and finite limits in one durable authorization.
2. Give the Agent permitted development metrics, persisted predictions/error
   analysis and bounded training-row inspection tools. Make missing evidence
   explicit. Aggregate scores alone are not a substitute for investigating
   concrete failures.
3. Persist the Agent's public explanation, evidence references and validated
   edit proposal. It may remove rows and request targeted additions from the
   separately selected Data generation model. No-change is valid; do not force
   edits or generation just to create activity.
4. Validate generated output through the owning dataset/native contracts:
   row semantics, membership, duplicates and benchmark contamination. Qualify
   the complete proposed population before training; invalid output must not
   become training data.
5. Publish an immutable derived dataset version with inspectable additions,
   removals, parentage and generation provenance. Never overwrite source data.
6. Train through the existing adapter using the exact recorded version and
   pinned settings. Save and register the artifact and dataset link even when
   development evaluation rejects the candidate.
7. Evaluate against the same original comparison baseline and unchanged
   development benchmark. Persist scores, reports and deterministic verdicts.
8. Feed the latest candidate's development result into the next iteration,
   including rejected-candidate evidence. Use the best eligible full dataset
   so far, falling back to the original dataset. Continue from the pinned
   starting model unless an explicitly supported contract says otherwise.
   Do not imply candidate warm-start or use Quick-test samples as source data.
9. End on finite ceilings or a validated no-change/stop decision. Select the
   best eligible candidate deterministically. Final holdout is a separate,
   at-most-once authorization after adaptive work ends. Never feed its rows,
   scores or diagnostics back into an adaptive step. Promotion remains manual.

Keep root setup immutable. Each iteration owns its dataset, actual training
input, candidate and result. Preserve proposal identities, requested-edit
charges, predecessor validation and completion receipts. Completed replay must
not duplicate calls, datasets, models, training or evaluation. Do not invent
the imported baseline's unknown training history.

## Setup and Quick test

Ordinary Setup contains only:

- Baseline.
- Starting dataset.
- Evaluation.
- LLM Agent.
- LLM Data generator.

API keys belong in project connection settings. Allow multiple connections,
discover their available models and select each role independently, including
different models from the same connection. Pin those choices per run; changing
project defaults must not redirect resumed work. Keep secrets outside the
renderer and logs. An unavailable pinned connection needs an actionable error.

Put tuning under **Advanced**, and expose only settings execution enforces:

- Optional Agent objective, subordinate to run authority and evidence isolation.
- Maximum optimization iterations and separate Agent-turn limits.
- Total row-change budget and separate provider request/token/spend ceilings.
- Generation concurrency from 1 to 16, default 1 for self-hosted compatibility.
  This is bounded concurrent requests in one local run, not multiple workers.
- Training device, epochs, batch size, learning rate and training-time limit.
  Resolve Auto and record the actual device.

Preserve the settings contracts in `docs/agentic-optimization-spec.md`:
standard defaults are three iterations, eight Agent turns per iteration and
192 total row changes.

Quick test uses the same execution path with one iteration, up to four Agent
turns, eight edits, 64 deterministic training rows and two minutes of training.
Qualify the full population before deriving the recorded sample. Keep the
development benchmark unchanged. Two minutes is a training ceiling, not a
total-runtime promise. Quick tests are diagnostic, cannot use final holdout
and cannot qualify a candidate for promotion. One normal iteration is not
equivalent to Quick test.

## Overview and live feedback

Keep one expandable row per run and the approved `Setup → Status → Report`
layout. No new redesign or duplicate progress panels.

Inside Status:

- Put the iteration selector above that iteration's six stages: Checking inputs,
  Preparing data, Starting, Training, Saving candidate, Evaluating. A new
  iteration must not make the same bar appear to jump backwards.
- Make every stage clickable. Show all persisted activity for that stage,
  paginating as needed. Historical selection and scroll position must remain
  stable during live updates; provide an explicit return-to-live action.
- Make the activity stream the primary insight surface: actual Agent decisions,
  evidence and row references, generation targets, filenames, training and
  evaluation details. Use truthful origins: Agent, Generation, Training,
  Evaluation and System, not Work/Now.
- Show public model explanations and validated tool decisions, not private
  chain-of-thought or System templates labeled as Agent reasoning. Tool-only
  responses still need an inspectable decision summary derived from the actual
  tool call. Label sampled diagnostics as samples.
- Keep stage progress above subordinate step progress. Omit processed-byte and
  row counters. Keep spinners continuous while work runs and stop them for
  terminal/paused states. Keep elapsed time and Stop/Resume beside Activity.
- Show separate Agent and Data generation token usage. Costs must be reported
  reliably or calculated from pinned rates; otherwise show unknown. Reservations
  are not actual spend, and interrupted calls do not imply zero usage.
- Avoid redundant checksum work through verified reuse where safe. Preserve
  provenance and invalidation checks; do not remove integrity guarantees merely
  to speed up the display. File verification must not drown out meaningful work.
- Show a simple, nonduplicated `ERROR: <reason>` when needed. Avoid filler prose
  and misleading states such as paused while a worker is still executing.

Report contains benchmark results, comparisons against the original baseline
and direction-aware green/red changes, plus the deterministic KEEP/REJECT
outcome. Each model, dataset diff and report opens its ordinary viewer.
Rejection is a scientific outcome, not a missing model or execution error.

## Recovery and accounting

Persist step identities and provider reservations before dispatch, with stable
run, iteration, action and child IDs. Resume the same run after Stop, failure
or app restart, reusing verified completed work.

Stop prevents new work at fine-grained step boundaries and interrupts active
calls/processes where safely supported; it must not wait for a whole macro
stage. Acknowledge pause only after owned work has stopped. Distinguish durable
Stop intent from permanent cancellation.

Enforce cumulative budgets across iterations, retries and outstanding requests,
including interrupted training. Unknown external outcomes remain unknown and
consume conservative reservations; do not claim exactly-once external execution.
Persist typed budget-stop outcomes. Separately written usage, activity and
completion records require reconciliation.

## Architecture and implementation rules

Follow `AGENTS.md` and read the required platform, slice, architecture and
development specifications before executable changes.
`docs/agentic-optimization-spec.md` governs this loop; preserve the object and
provenance contracts in `docs/encoder-workspace-product.md`.

Reuse existing ownership:

- `encoder-optimization-core` / `encoder-optimization-runner`: provider-neutral
  Agent inspection, proposal and generation contracts.
- `project-workspace-core` / `project-workspace-local`: immutable authority,
  iteration lineage, provider pins, accounting and publication.
- `encoder-experiment-nomos`: native diagnostics, qualification,
  materialization, training and evaluation adapters.
- Workspace optimization CLI: production composition. The desktop invokes it,
  not a parallel workflow implementation.

Preserve legacy history and fixed-recipe guards. Do not add cloud execution,
multiple workers, unrelated abstractions or another provider stack. Keep core
logic independent of UI, SDK and persistence types.

After each coherent executable component, run `cargo fmt-check`,
`cargo check-all`, `cargo lint`, `cargo test-all` and relevant UI/native checks.
Review interfaces and dependency directions; commit the verified component
when Git identity is configured. Keep existing unrelated changes intact.

Real Nomos state remains read-only during development. Use deterministic
adapters and temporary projects for ordinary tests. Paid/provider execution,
real training and final holdout require explicit execution authorization.
A request to update this file is documentation-only; it does not authorize
continuing implementation or launching a run in the same turn.

Keep checkpoints bounded and updates concrete. When asked for status or to
stop, pause implementation and answer immediately.

## Acceptance evidence

Completed CLI mechanics are not completed desktop integration:

- [x] Production-CLI tests trace development evidence through Agent decisions,
  removals/additions, qualification, exact training lineage and comparison using
  deterministic adapters.
- [x] Production-CLI tests prove an evidence-dependent second iteration,
  unchanged baseline/benchmark and genuine no-change completion.
- [x] Production-CLI Stop/Resume tests prove durable control intent, stale-command
  rejection and reuse of completed inspection after an interrupted Agent call
  (`9ea9dc4`). This is narrower than every-boundary recovery below.
- [ ] One Optimize action invokes the pinned Agent and, when additions are
  proposed, the independently pinned generator through that same coordinator.
- [ ] Every exposed Advanced setting is enforced cumulatively, including
  concurrency, uncertain outcomes and interrupted-training limits; Quick test
  uses the same engine with its diagnostic restrictions.
- [ ] Stop/restart/Resume tests cover every boundary, control-intent races,
  worker/descendant ownership and reuse of completed work.
- [ ] Complete derived datasets pass native admission and benchmark-isolation
  checks through the connected path; invalid generated rows cannot train.
- [ ] Rendered production-path tests show real Agent/Generation activity,
  complete independently browsable iteration/stage history, responsive controls
  and continuous spinners.
- [ ] Candidate models, dataset diffs and benchmark reports open from the same
  run, including rejected candidates and multiple iterations.
- [ ] Adaptive work never sees sealed evidence. Final holdout has its separate
  at-most-once selected-candidate handoff; promotion remains manual.
- [ ] Required Rust, UI/native and rendered journey checks pass on the final
  executable tree. The handoff distinguishes deterministic evidence from
  separately authorized live execution.

Completion requires a usable, traceable, bounded and resumable optimization
journey in the actual app. Show one run's chosen providers, Agent decisions,
dataset changes, trained candidates and benchmark comparisons, plus proof of a
second evidence-dependent iteration and Stop/restart reuse. Configuration,
fixture-only rendering and a successful fixed training run are not completion.
