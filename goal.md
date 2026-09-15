# Goal: complete actual agent-driven encoder optimization

Implement the missing optimization engine and connect it to the existing
Overview journey. The user selects a baseline, starting dataset, shared
evaluation benchmark, Agent model, and Data generation model, then presses
Optimize. The agent uses development evidence to decide which training rows to
remove and which gaps to fill, requests generation, trains a candidate, evaluates
it, and uses that result to decide the next bounded iteration.

The production CLI now supports bounded iterations driven by previous results,
with durable no-change outcomes and completion-write recovery. All four Rust
gates passed for that committed component. A follow-on execution journal is in
the working tree; it is not yet a completed Stop/Resume implementation or a
connected desktop journey. Finish run recovery, then connect one-click
Overview execution using this same engine. More settings, system
narration, documentation, or UI mockups alone do not complete this goal.
Preserve the approved GUI rather than redesigning it.

## What the user must be able to do

Open Nomos → Overview → New run, select the five inputs, optionally adjust
Advanced settings or enable Quick test, and press Optimize once. Watch the Agent
investigate development failures, explain its proposed dataset changes, request
generation, and follow the resulting candidate through training and evaluation.
Inspect the dataset diff and the candidate's comparison against the original
baseline. Stop, close the app and resume the same run without losing completed
work. No manual sequence of CLI commands or repeated stage-confirmation buttons
should be required for work already covered by the run's authorization.

The user should not have to start another real run merely to find out whether
the Agent is connected. Prove that integration through the production execution
path with deterministic test adapters first. A successfully rejected candidate
is a completed experiment; improvement itself is not guaranteed.

## Next deliverable

Make the existing Overview execute and display the tested Agent loop. Keep the
approved `Setup → Status → Report` layout. Deliver in bounded checkpoints:

1. Verify and commit the in-progress execution journal without rebuilding the
   working loop. Preserve completed, failed and interrupted attempt history.
2. Finish durable Stop/Resume, worker-liveness reconciliation and explicit budget
   outcomes. Stopping is not permanent cancellation; resuming must reuse completed
   work and account conservatively for uncertain calls.
3. Update the desktop's strict run parser and lifecycle handling, then connect
   Optimize to the same `drive-agent` coordinator. Preserve historical fixed-run
   compatibility; do not introduce a second executor.
4. Prove the rendered journey: actual Agent decisions, targeted generation,
   inspectable dataset edits, candidate training and comparison, a subsequent
   evidence-dependent iteration, and Stop/restart recovery.

Do not tell the user to start another Nomos run until that connected path has
passed deterministic integration tests. These checkpoints are not permission
to run paid providers, real training or final holdout.

## Current state — 15 September 2026

The agentic user journey is **not ready yet**. Run 17 demonstrated fixed training
and evaluation, not Agent-driven optimization. Implementation must finish and
verify the connected engine before asking the user to try another real run.

| Capability | Evidence | Remaining work |
| --- | --- | --- |
| Fixed training and development evaluation | Real Nomos Run 17 | This path is not Agent-driven |
| Agent → generation → dataset publication → native clearance | Committed in `18fc85b`; deterministic production-CLI tests | Preserve and reuse this path |
| One Agent-directed cycle through training and development evaluation | Committed in `a8bb5c4`; complete-cycle/recovery tests and required Rust gates passed at that checkpoint | Root-run completion and production desktop integration |
| Candidate registration, exact training-dataset links and benchmark report lookup | Committed in `6d90ead`; focused cycle, lineage and recovery regressions pass | Production Overview/viewer interactions and later-iteration associations |
| Repeated iterations, best-dataset selection and no-change | Committed in `f3d222b`; all four Rust gates and 14 managed benchmark CLI tests pass, including seven Agent-path tests | Resumable run lifecycle, final selection handoff and complete interruption recovery |
| Root execution attempts and adaptive completion | Uncommitted follow-on: journal-derived Agent states, verified terminal-iteration link, failure and abandoned-attempt recovery; final managed CLI suite passes | Confirm full validation/commit, durable Stop/Resume, liveness reconciliation and explicit budget outcomes |
| One-click agentic Overview journey | Not connected end to end; strict desktop run parser does not yet accept the new Agent execution states/record | Update parser and lifecycle handling, then wire the same coordinator, model/report links and iteration activity |

### Bounded CLI loop checkpoint

`workspace optimization-run <PROJECT> drive-agent <RUN_ID>` composes the existing
Agent, generator, qualification, training and registration functions. Completion
records pin proposal calls, requested edit charges, results, best eligible data
and the reason the loop ended. Next-iteration bindings preserve the original
baseline and benchmark, use the latest candidate's development evidence and the
best eligible full dataset, and continue training from the pinned starting model.
They do not imply candidate warm-start or use Quick-test samples as source data.

All 14 managed benchmark CLI tests pass on the latest focused build, including
seven Agent-path tests. They cover the existing single cycle,
two evidence-dependent iterations, first/later no-change without forced work,
cumulative edit-limit termination, and retaining an earlier better dataset
while inspecting the latest result. Injected completion-write failure preserves
completed provider, training and evaluation work on retry. Candidate and report
links span iterations. Completion reads reconstruct the original proposal and
selection; the focused suite also rejects self-consistently re-fingerprinted
completion records. A large diagnostic fixture verifies that development metrics
remain visible on the first inspection page instead of being buried beneath
failure samples.

UI typecheck, 125 UI tests, nine native-clearance Python tests and rendered
Overview fixture checks passed during this checkpoint. The rendered checks are
not connected Agent-loop acceptance tests. The final unchanged-tree validation
process was recovered and exited successfully: `cargo fmt-check`,
`cargo check-all`, `cargo lint` and `cargo test-all` all passed. This supersedes
the earlier run that overlapped an executable fixture edit; it does not prove
the remaining root lifecycle, complete recovery or desktop integration.

The next implementation work is resumable root-run lifecycle, budget/stop outcomes
and recovery at every boundary, then production Overview integration. The new
execution journal projects Agent status separately from the root's legacy
preparation state. Final holdout still
needs its separate selected-candidate handoff; adaptive protocols remain
development-only. Do not rebuild the working loop or ask the user to discover
these missing integrations by starting another real Nomos run.

### Root execution checkpoint — in progress

`drive-agent` now records an execution attempt before preparation. Its ordinary
run view distinguishes Agent running, interrupted, failed and completed states.
Completion pins the last verified terminal iteration, including no-change and
development rejection; it is not final acceptance or baseline promotion.
Completed replay preserves the execution history and does not dispatch new work.
The old fixed-recipe journal and serialized history remain compatible.

Two domain lifecycle tests pass. The final full-suite process has also passed
all 14 managed benchmark CLI tests, including the added parent-completion
interruption case: retry saves the parent completion without repeating completed
Agent, generation, training or evaluation work. UI typecheck, all 125 UI tests,
nine native-clearance Python tests and rendered Overview fixture checks pass.
The full Rust process has not yet been confirmed terminal; do not claim all
gates passed for this working-tree component until its exit result is recorded.

This component does not yet implement durable resumable Stop/Resume or active
child cancellation. An unclosed attempt is reconciled after exclusive lease
acquisition on retry; the stored `running` state alone is not evidence of a live
worker. Add proactive liveness reconciliation and typed budget-stop outcomes.
Continue toward the same connected Overview goal, not another fixed-run path.

The desktop parser in `ui/src/input-optimization.ts` currently rejects the new
Agent execution states and `agentExecution` field. Extend and test that contract
before exposing these runs through Overview. Passing unchanged UI fixture tests
does not establish compatibility with the new backend payload or prove that
Optimize invokes the Agent.

### Committed foundations

- `2986f1b`: validated advanced settings and Quick test authorization.
  These contracts do not by themselves implement the loop or expose working
  advanced controls in production. The fixed-recipe executor must reject
  settings it cannot enforce.
- `bb96421`: selected-model Pi transport, bounded inspection tools, validated
  proposals, append-only Agent-call journal, native development diagnostics,
  concurrent generation with durable accounting, and immutable dataset
  publication. Provider connections are pinned by UUID; changing project
  defaults must not redirect a resumed run.
- `6dc5585`: immutable first-iteration input/evidence binding, separate from
  individual Agent calls. The current loop extends it with verified predecessor
  lineage; preserve that validation rather than accepting caller-supplied
  replacement evidence.
- `24d61db`: derived candidate report lookup follows recorded preparation,
  materialization and child-experiment receipts. Read-only verification against
  real Nomos recovered Run 17's two candidate reports without changing its
  database or rerunning evaluation. Preserve historical fixed-run compatibility.
- `18fc85b`: the production CLI connects the pinned Agent, separate generator,
  immutable removal/addition publication and full-population native clearance.
  `edit-dataset` publishes the diff; `prepare-candidate` also qualifies it.
  Native clearance checks row validity, duplicates and source/group/lineage
  benchmark overlap. It exposes aggregate facts, not protected payloads.
  The activity projection includes actual Agent and Generation journal actions.

The exact `18fc85b` checkpoint passed the required Rust formatting, compilation,
lint and full test gates. Its nine native-clearance Python tests, 125 UI tests
and rendered Overview checks also passed. Those rendered checks use fixtures;
they do not prove that Optimize invokes an Agent loop.

### Verified single-cycle CLI checkpoint — `a8bb5c4`

The `optimization-run complete-iteration` route extends that same
production composition through:

1. Normal run reservation and input preparation; no manually seeded reservation
   is required by the new integration fixture.
2. First-iteration evidence binding, Agent inspection/proposal, separate
   generation, dataset publication and complete-population native clearance.
3. An immutable training binding, including actual training dataset, native
   materialization, candidate/protocol identities and resolved device.
4. Native training with the pinned settings and both development suites against
   the original comparison baseline.
5. A journal-derived iteration result with reports and deterministic verdicts.

Quick test derives its deterministic training subset **after** qualifying the
full population. The actual sampled dataset is recorded separately; the
development benchmark is not sampled. Native training receipts check the
effective input and training parameters. Adaptive iteration protocols have no
final-holdout allowance.

A focused production-CLI test passed with deterministic Pi responses, a
loopback generator and native test adapters: inspect evidence, remove/add rows,
qualify, train the exact sample, evaluate both suites, and reuse completed work
on retry without additional provider or native calls. It also verifies that
changing project provider defaults does not change the run's selected models.

At that checkpoint, the focused suites passed: nine managed benchmark tests,
six optimization setup tests, and the zero-holdout runner regression. These include publication
lineage/device checks, saved-model verification and injected failure at iteration
result persistence. The holdout regression exposed an invalid authorization
being appended before rejection; the runner now validates that transition before
persistence, leaving the original journal unchanged on rejection.

UI typecheck, all 125 UI tests and rendered Overview interaction checks pass.
The rendered checks use fixture activities, not a connected Optimize button.
All four Rust gates (`cargo fmt-check`, `cargo check-all`, `cargo lint`,
`cargo test-all`) passed for `a8bb5c4`, as did the nine native
clearance Python tests. The full run includes the complete-cycle recovery and
zero-holdout regressions. These checks prove the tested mechanics, not real
Nomos improvement or an end-to-end agentic desktop journey.

Run reservation now accepts an authorized agentic request. The legacy
fixed-recipe materialization path remains guarded; the new coordinator is the
only path intended to honor those settings. Do not bypass that guard.

At `a8bb5c4`, the remaining gaps were:

- A second iteration driven by the first result, or a durable no-change outcome.
- A complete root-run lifecycle, final selection or separately authorized holdout.
- The production Overview button invoking this coordinator.
- Proven Stop/restart recovery at every boundary or reconciliation of uncertain
  provider outcomes and separately persisted activity/accounting.

### Candidate and report checkpoint — `6d90ead`

The single-cycle CLI now registers its verified trained output, including a
development-rejected candidate, and returns its ordinary model and dataset link.
Materialized training evidence verifies the exact trainer version and ordered
native contents, including Quick-test subsets, without replacing original row
IDs or inventing the imported baseline's unknown training history.

Normal `workspace benchmark <PROJECT> results <VERSION>` follows explicit root,
iteration and training receipts into the derived scientific project. It replays
development reports without rerunning evaluation and does not expose sealed
scores. Results remain available if the process stops after native completion
but before the iteration summary is saved. Successful activity records link the
model, exact dataset version, scientific run and development reports.

The focused CLI tests prove rejection still produces a cataloged model, a
one-row Quick-test version preserves its qualified source identity, and retry
does not duplicate models, imports, versions or completed calls. They also
reject re-fingerprinted foreign project/protocol/candidate/training-source/run
associations without partially changing the report projection. Local tests cover
reordered native rows, foreign versions, failed-link retry and full verification
after moving the project. All nine managed benchmark tests and nine local
model-dataset tests pass. UI typecheck, all 125 UI tests, rendered Overview
fixture checks and nine native-clearance Python tests also pass. All four Rust
gates (`cargo fmt-check`, `cargo check-all`, `cargo lint`, `cargo test-all`)
passed for `6d90ead`, not the current unverified iteration changes. Opt-in
live-provider tests remain unrun; no real Nomos run or paid provider execution
was used for this evidence.

That checkpoint did not connect production Overview or prove later iterations.
The bounded CLI loop above now extends it; root lifecycle, complete recovery
and desktop integration remain required.

### What Run 17 actually established

Nomos Run 17 (`998d54fb-363f-443f-abeb-195dd2574d79`) used 6,800 existing
training rows, one epoch on CUDA, failed both development suites, kept the
baseline and left final holdout unused. Its 4,431 activity events contained
zero Agent-origin entries. Its narrative entries were System templates,
including one labeled `reasoning`. Evaluation metrics named `agent_*` measure
benchmark performance; they are not optimization-Agent invocations.

Project connections, model discovery and independent Agent/Data generation
assignments exist. Their presence does not prove execution. The Agent's actual
public explanation, evidence references and validated tool decisions must be
visible; System narration must never masquerade as Agent reasoning.

No new real Nomos training, provider calls or protected evaluation were used to
demonstrate the in-progress cycle. Keep real project state read-only during
development and distinguish deterministic test evidence from live execution.

## Immediate next task: finish the connected engine

The target is one executable chain:

`Optimize → Agent proposal → dataset diff → clearance → training → evaluation → report`

Do not rebuild the committed Agent, generator, provider resolver or clearance.
Do not redesign the approved Overview. The single-cycle and repeated-iteration
CLI components have passed the required gates. Finish run ownership, recovery
and app integration:

1. **Preserve the verified single-cycle checkpoint.** The production-CLI test
   uses normal reservation and the coordinator the desktop must invoke. It
   covers exact dataset/sample lineage, enforced training settings,
   original-baseline comparisons, no adaptive holdout access and recovery after
   native completion but before iteration-result persistence. Do not rebuild
   this chain or claim it already implements the whole loop.
2. **Finish iteration ownership.** Preserve the new candidate registration,
   materialized-version links and receipt-based benchmark projection. Persist
   root-run progress and interruption/budget outcomes. Preserve the in-progress
   execution journal's completed projection instead of leaving a finished Agent
   loop labeled `ready`. Distinguish successful experiment
   completion (including rejection/no-change), paused work, exhausted budgets
   and execution errors using persisted facts. Preserve completion,
   selection and multi-iteration/report validation through verified predecessor
   results. Do not invent baseline training history or
   relabel a flattened native import as the selected training version.
3. **Preserve and complete bounded execution.** The CLI now feeds persisted
   development results and eligible dataset lineage into subsequent Agent scopes.
   Extend its tests for cumulative request/token/spend and training-time limits,
   unknown outcomes and concurrency. Keep the original baseline and benchmark
   and the existing iteration/edit ceilings; do not introduce another loop.
4. **Complete recovery and final selection.** Reuse completed provider calls,
   dataset publication, training and evaluation. Test fine-grained Stop,
   interruption and restart, including uncertain external outcomes. Do not
   repurpose permanent cancellation as resumable Stop. Prevent new work after
   Stop is requested and resume only from verified boundaries. Reconcile
   activity with durable outcomes. Select the best eligible candidate
   deterministically; final holdout is a separate, at-most-once authorization
   after adaptive work ends. Promotion remains manual.
5. **Connect and verify the existing Overview.** First make the strict desktop
   run parser, state handling and controllers accept the actual Agent execution
   contract while retaining old run compatibility. One Optimize action must invoke
   this same coordinator. Expose iteration-specific stages and complete activity,
   actual Agent decisions, separate provider usage and linked reports. Add only
   Advanced controls whose settings execution really enforces. Verify rendered
   interactions with the connected production path, not injected Agent captions.
   Prove one-click execution, a second evidence-dependent iteration, historical
   stage browsing during live updates, and Stop/restart recovery before asking
   the user to test another real Nomos run.

Keep root setup/preparation immutable. Each iteration owns its derived dataset,
training input, candidate and result. Do not overwrite the starting dataset or
reuse one fixed-recipe receipt to represent multiple iterations.

Use the existing ownership boundaries:

- `encoder-optimization-core` / `encoder-optimization-runner`: provider-neutral
  Agent inspection, proposal and generation contracts.
- `project-workspace-core` / `project-workspace-local`: immutable authority,
  iteration lineage, provider pins, accounting and publication.
- `encoder-experiment-nomos`: native diagnostics, data admission,
  materialization and training/evaluation adapters.
- Workspace optimization CLI: production composition; the desktop invokes the
  same coordinator rather than maintaining another workflow.

Known recovery gaps remain work, not guarantees: successful replay is not proof
of interruption safety at every boundary; provider usage and activity written
separately need reconciliation; interrupted training may lack a resumable
checkpoint. A reserved cost is not actual spend. Unknown prices stay unknown.

Report progress separately as component-tested, CLI-integrated and app-verified.
For each checkpoint state what capability was added, the exact production path
tested and the remaining integration gaps. Keep the end-to-end acceptance
criteria below unchecked until their complete evidence exists. The next action
belongs to implementation, not another user-triggered Nomos experiment.

## Required execution

1. Pin the exact baseline, starting dataset version, benchmark version, provider
   selections and finite settings in one durable run authorization.
2. Give the configured Agent permitted development metrics/diagnostics and
   bounded training-data inspection tools. Persist its actual public explanation
   and explicit edit proposal, with the evidence and row identities it used.
   Consume persisted development predictions and errors through the analysis
   contracts; do not substitute aggregate scores or repeated file checks for
   investigating concrete failure cases. Make missing evidence explicit.
3. Apply justified removals and request targeted additions from the separately
   selected Data generation model. Validate native row semantics, membership,
   duplication and contamination through the owning slice/task contracts.
   Reject invalid output; never trust LLM output as its own qualification.
4. Publish an immutable derived dataset version with inspectable additions,
   removals, parentage and generation provenance. Do not modify the source.
5. Train a candidate through the existing training adapter using that exact
   version and the pinned training limits. Save and register its artifact and
   dataset link whether the candidate passes or fails development evaluation.
6. Evaluate on the unchanged development benchmark. Persist comparable scores
   and deterministic gate decisions; ensure results link correctly in Overview,
   Models and Evaluation.
7. Give the next iteration the previous development result and explicit lineage
   of its starting dataset/model. Preserve the original comparison baseline.
   Use the best eligible dataset so far, falling back to the original starting
   dataset when no derived candidate is eligible. Failed candidate evidence
   still informs the next proposal. Record the actual training starting model
   explicitly; do not confuse it with the fixed comparison baseline.
   Stop on the configured ceilings or an explicit no-change/stop decision.
   Do not force unnecessary edits merely to demonstrate agent activity.
8. After adaptive work ends, select the best eligible candidate deterministically.
   Use final holdout at most once, only under matching persisted authorization.
   Never feed sealed rows, scores or diagnostics back into the agent or another
   iteration. Promotion remains a separate user action.

## Settings and rapid testing

Keep the ordinary setup to the five input selections. Place tuning controls
under Advanced settings and wire every exposed control to enforced execution:

- Optional agent objective; it cannot override authority or safety boundaries.
- Maximum optimization iterations, separately from Agent turns per iteration.
- Total row-change budget and independent provider request/token/spend ceilings.
- Generation concurrency: configurable from 1 to 16, default 1 for self-hosted
  compatibility. Concurrency means bounded requests inside one local run, not
  multiple workers or parallel trainers. Respect cumulative limits across retries
  and outstanding requests, not just completed calls.
- Training device, epoch ceiling, batch size, learning rate and training-time
  limit. Resolve Auto to the actual device and record what was used.

Use the settings contracts in `docs/agentic-optimization-spec.md`: standard
defaults are three iterations, eight Agent turns per iteration and 192 total row
changes. Quick test uses one iteration, at most four Agent turns, eight edits,
64 deterministically selected training rows and two minutes of training.
It must exercise the same agent/generation/training/evaluation path, not a fake
production shortcut. The development benchmark remains unchanged and may take
longer; two minutes is a training ceiling, not a total-runtime promise.
Quick tests are labeled diagnostic and cannot use final holdout or qualify a
candidate for promotion. Setting normal iterations to one is not quick mode.

## Recovery, provenance and feedback

- Persist each step and external-call reservation before dispatch, with stable
  run, iteration, action and child IDs. Preserve completed work across Stop,
  failure, process exit and restart. Resume the same run and lineage.
- Stop must prevent new work at fine-grained step boundaries and interrupt
  active calls/processes where safely supported; it must not wait for an entire
  macro stage. Be explicit when interrupted training lacks a resumable checkpoint.
- Account for failed and interrupted provider attempts conservatively. An
  unknown remote outcome is not zero usage or an exactly-once guarantee.
- Record actual model-produced public rationales and tool decisions as Agent
  activity. Do not relabel system templates as agent reasoning, expose private
  chain-of-thought, or display fabricated messages as live execution.
  A tool-only model response must still expose its validated public decision
  summary. Show which evidence motivated the change and provide inspectable
  row references; do not make a free-text model response a prerequisite for
  seeing Agent activity. Label sampled diagnostics as samples rather than
  implying that the agent inspected every evaluation failure.
- Pin each role to the selected connection and model for the run. Changing
  project defaults later must not silently switch a resumed run's provider,
  model or credential connection. Resolve secrets outside logs and the renderer;
  if a pinned connection is unavailable, pause with a specific actionable error.
- Show separate Agent and Data generation token usage. Show cost only when
  reported reliably or derived from pinned rates; otherwise mark it unknown.
  Credentials and protected evidence must never enter logs or the renderer.

## Overview and report

Keep one expandable row per run with `Setup → Status → Report`. Inside Status,
place an iteration selector above that iteration's stages and activity stream.
Each iteration owns its stage history; the same bar must not appear to jump
backwards when another iteration starts. Selecting historical activity must
remain stable during live updates; provide an explicit return-to-live action.

Stages are clickable and expose all their persisted activity with pagination as
needed. The stream is the primary place for Agent explanations, generation
targets, filenames, training work and evaluation details. Use truthful origins
such as Agent, Generation, Training, Evaluation and System. Keep the stage bar
visually above the subordinate step bar; omit processed-byte and row counters.
Spinners must remain stable while work runs. Keep elapsed time and Stop/Resume
beside the activity heading. Avoid duplicate progress panels and explanatory
filler. Do not redesign the rest of the app while wiring this loop.

Report shows candidate comparisons against the same baseline and benchmark,
direction-aware green/red changes and the deterministic KEEP/REJECT outcome.
Every candidate, dataset diff and evaluation must open its ordinary viewer.
Rejection is a valid scientific outcome, not an execution failure or missing model.

## Implementation and completion evidence

Follow `AGENTS.md`: preserve slice ownership and provider-neutral ports; implement
core logic, persistence and CLI composition before production UI integration.
The newer `docs/agentic-optimization-spec.md` governs this loop and Overview
behavior. Preserve the object/provenance contracts in
`docs/encoder-workspace-product.md` and existing launch history compatibility;
do not treat older header-Optimize/page descriptions as the current UI target.

Implement and commit coherent, verified checkpoints:

1. One evidence-driven cycle through the actual application/CLI composition.
2. Bounded repeated iterations, concurrent generation, accounting and recovery.
3. Production Overview wiring, report associations and rendered interaction tests.

Ordinary tests use deterministic Agent, generator and trainer adapters. Prove
an evidence-driven removal and addition, generated-row rejection, immutable
dataset/model ancestry, and a second iteration that consumes the first result.
Test concurrency 1 and greater than 1, budget exhaustion, stop/restart at step
boundaries, uncertain provider outcomes, no duplicate completed work and sealed
isolation. Exercise production orchestration, not disconnected mocks with
injected Agent captions. Test quick mode through the same orchestration path.

Run `cargo fmt-check`, `cargo check-all`, `cargo lint`, `cargo test-all`, relevant
UI checks and actual rendered interaction tests for executable changes. Check
dependency directions, preserve unrelated edits and commit verified components.
Use real Nomos state read-only during development. Do not launch paid calls,
real training or holdout evaluation without explicit execution authorization.
Updating this goal file does not itself authorize any such execution.
An instruction to update `goal.md` is a documentation task, not an instruction
to continue implementation or start an optimization run in that same turn.

Keep implementation checkpoints bounded and communicate concrete progress.
When the user asks for status or says stop, pause implementation and answer
immediately. A passing component test, another documentation update or a build
is not a substitute for completing the missing production connection.

### Acceptance checklist

Keep these unchecked until the corresponding production-path evidence exists:

- [ ] One Optimize action invokes the pinned Agent and, when additions are
  proposed, the independently pinned generator through the real coordinator.
- [x] A deterministic end-to-end test traces inspected failure evidence to an
  actual public Agent explanation, removal, generated addition, qualified
  dataset version, trained model and development comparison. Proven through the
  production CLI with deterministic provider/native adapters, not the GUI or a
  paid/live Nomos experiment.
- [x] A second iteration consumes the first result and produces an
  evidence-dependent next proposal while preserving the original baseline and
  benchmark. Explicit no-change decisions end the loop without invented edits.
  Proven through the production CLI with deterministic adapters, not the GUI.
- [ ] Every exposed advanced setting is enforced, including cumulative budgets
  and concurrency, and Quick test exercises this same engine with its limits.
- [ ] Stop/restart tests verify recovery at each boundary, stable identities,
  reuse of completed work and conservative accounting for uncertain calls.
- [ ] Complete derived datasets pass the native admission and
  training-to-benchmark isolation checks; invalid generated rows cannot train.
- [ ] The rendered Overview shows real Agent/Generation activity, independently
  browsable iteration/stage histories, responsive controls and stable spinners.
- [ ] Candidates, dataset diffs and benchmark reports open correctly from the
  same run, including rejected candidates and multiple iterations.
- [ ] Sealed evidence never reaches adaptive steps; final holdout is separately
  authorized, used at most once after iteration ends, and promotion stays manual.
- [ ] Required Rust gates, relevant UI checks and rendered journey tests pass;
  the handoff distinguishes test evidence from separately authorized live work.

The goal is complete only when Optimize invokes the chosen providers, produces
traceable agent-directed work, honors settings, resumes correctly, and exposes
truthful iteration activity and linked results through the real app journey.
Tests may prove mechanics without claiming a real model improvement. State
exactly what passed, what ran for real and what remains; configuration or a
successful fixed-recipe training run is not completion of this goal.

Completion evidence must let the user follow one run from its selected providers
to actual Agent decisions, generated additions and removals, the resulting
dataset version, trained candidate and benchmark comparison. Include a
two-iteration test showing how the first result changed the next proposal, and a
stop/restart test showing which completed work was reused. Clearly distinguish
deterministic test evidence from any separately authorized live run.
