# Goal: complete actual agent-driven encoder optimization

Implement the missing optimization engine and connect it to the existing
Overview journey. The user selects a baseline, starting dataset, shared
evaluation benchmark, Agent model, and Data generation model, then presses
Optimize. The agent uses development evidence to decide which training rows to
remove and which gaps to fill, requests generation, trains a candidate, evaluates
it, and uses that result to decide the next bounded iteration.

The next deliverable is a working end-to-end optimization cycle, followed by
verified iteration and recovery. More settings, system narration, documentation,
or UI mockups alone do not complete this goal. The approved GUI is the interface
to this engine; it is not a separate workflow to redesign.

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

## Current state — 15 September 2026

The agentic user journey is **not ready yet**. Run 17 demonstrated fixed training
and evaluation, not Agent-driven optimization. The next action belongs to
implementation, not to the user: connect and test the engine before asking for
another real run.

| Capability | Verified level | Still missing |
| --- | --- | --- |
| Fixed training and development evaluation | Real Nomos Run 17 | Agent-directed dataset changes |
| Agent inspection/proposals and concurrent generation | Component-tested | Invocation by the production optimization coordinator |
| First-iteration input and evidence binding (`6dc5585`) | CLI-integrated and regression-tested | Automatic handoff to Agent execution; later-iteration lineage |
| Derived candidate report association (`24d61db`) | CLI-tested and read-only verified against Nomos | Full multi-iteration app journey |
| One-click bounded Agent loop | Not implemented end to end | Composition, recovery and Overview verification |

- The current Optimize executor performs a fixed one-candidate training and
  evaluation sequence. It does not invoke the configured optimization Agent or
  Data generation model, or perform agent-directed dataset edits.
- Nomos Run 17 (`998d54fb-363f-443f-abeb-195dd2574d79`) completed that fixed path:
  6,800 existing training rows, one epoch on CUDA, both development suites
  failed, baseline retained, final holdout unused. Its 4,431 activity events
  contain zero Agent-origin entries. The four narrative entries are System
  templates, including the entry labeled `reasoning`.
- Project connections, model discovery and independent Agent/Data generation
  assignments exist. Configuring those connections does not prove that the
  optimization executor calls them. Evaluation metrics named `agent_*` are
  benchmark measurements, not evidence of an optimization-agent invocation.
- Commit `2986f1b` adds validated advanced settings, quick-test authorization,
  immutable CLI preview/history and desktop parsing compatibility. It does not
  implement the agent loop or expose working advanced controls in production.
  The fixed-recipe executor explicitly rejects agentic settings.
- Commit `bb96421` contains a selected-model Pi transport, bounded
  development/training inspection tools, validated edit proposals, an
  append-only Agent-call journal, a native development-diagnostic reader,
  bounded concurrent generation with durable call accounting, and recoverable
  publication through ordinary imports and dataset versions. These are
  foundations, not a connected optimization engine. The full Rust checks and
  desktop smoke/restart journeys passed for this checkpoint; they do not
  establish that Optimize invokes these components. Preserve and connect this
  committed work rather than rebuilding it or replacing it with another fixed
  recipe.
- Native generation currently creates questions from inspected training
  templates while preserving their labels, tool registry and router state.
  Schema checks and duplicate rejection are implemented components, not proof
  of semantic correctness or complete training-to-benchmark isolation. The
  complete derived dataset still needs qualification before training.
- The same commit pins provider credentials by connection UUID and lets
  the desktop retrieve a run's original provider revision through the CLI.
  Preserve this implementation: changing project defaults must not redirect a
  resumed run to another key. This is execution infrastructure, not evidence
  that the Agent or generator participates in Optimize yet.
- The Agent journal still restricts execution to the first iteration. There is
  no connected production coordinator that carries an accepted proposal through
  generation, dataset qualification, training, evaluation and the next Agent
  turn. This integration, rather than another provider-settings redesign, is
  the next priority.
- Commit `6dc5585` gives the first iteration a project-owned immutable input
  record, separate from individual Agent calls. It binds the prepared baseline revision, model,
  dataset, benchmark, provider revision and complete development-report set.
  `optimization-run bind-iteration` builds it from the pinned scientific source;
  `optimization-run iterations` reads it without native execution. Agent-store
  opening and scope admission now require this record. Binding is currently a
  standalone CLI operation; Prepare and the GUI do not invoke it automatically.
  This is the first input handoff, not the Agent/generation/training coordinator
  or a working app loop.
  Later iteration completion and selection lineage are still unimplemented.
  Its production-CLI regression passes: exact retry identity, unchanged native
  database bytes, complete suite coverage, substituted-input rejection,
  immutable storage, foreign-key integrity and live-call ownership on reopening.
  Formatting, compilation, lint, the full Rust suite, all 124 UI tests and
  desktop smoke/restart checks passed. An offline research-sidecar startup
  timeout on the first full run passed both an isolated retry and the full
  rerun; runtime limits were not changed. No live Nomos execution was performed.
- Run 17's candidate reports were missing from project benchmark results even
  though they existed in the scientific journal. The lookup searched only the
  original scientific project, while Optimize recorded its candidate under a
  derived project for the selected dataset.
- Commit `24d61db` fixes report association by following the optimization run's
  recorded preparation, materialization and child-experiment receipts. It preserves the
  original baseline context and does not scan arbitrary scientific projects.
  The CLI regression now verifies linked reports, original verdicts after a
  baseline change, forged-link rejection, holdout exclusion and unchanged
  database/WAL bytes. The fixture explicitly completes its own WAL checkpoint
  before the standalone-file comparison. Native materialization's fresh baseline
  UUID is handled through the existing exact-content-equivalence contract.
  Read-only verification against real Nomos now returns Run 17's two candidate
  reports (`generic_holdout`, `retired_post_scaling`) with their original failed
  verdicts and baseline revision. Both database hashes remained unchanged; no
  training or evaluation was rerun.
  The full Rust gates, UI typecheck, all 124 UI tests and both desktop
  smoke/restart journeys passed for this change.

Verification levels are currently distinct: Agent/generation components are
component-tested; the full agent-driven CLI coordinator is not connected; the
production app journey is not agent-loop-verified. The report lookup is a
verified CLI capability, not proof of an Agent invocation.

Preserve existing projects, runs, model custody, dataset versions, provider
connections, credentials, navigation and the user's approved Overview design.
Do not require another real Nomos run to rediscover the known missing executor.

## Immediate next task: connect the engine

Connect the committed execution components through the production CLI
before adding more presentation. The next engine checkpoint must connect the
configured Agent and Data generation
providers to the real CLI workflow: inspect development failures and training
rows, propose edits, publish a validated dataset version, train and evaluate.
Prove this composition with deterministic adapters before asking the user to
try another run. Do not spend the next checkpoint on another settings-only
change, mockup or synthetic Agent message. This checkpoint is not completion:
continue through bounded iteration, recovery and the production Overview journey.

Use the existing exact-connection resolver and pinned-provider CLI read in the
new execution path; do not build a second credential-selection mechanism.
The immediate integration must resolve the run's pinned credential connection,
not whichever connection is currently assigned in project settings. Persist the
Agent proposal, generation attempts and resulting dataset version as resumable
steps of the same run. Remove the fixed-executor rejection only when the new
path actually enforces the authorized settings. Do not unblock the UI by
bypassing that guard or silently ignoring unsupported settings.

Close these integration gaps in that order:

1. Use the durable first-iteration input binding already implemented. Connect
   its exact development references and dataset to the existing Agent runner;
   do not introduce another parallel input/credential mechanism.
2. Connect Agent inspection and proposals to generation, full dataset
   qualification, version publication, native training and evaluation through
   the production CLI. Reuse the existing slice contracts and journals.
3. Drive subsequent iterations from their persisted predecessors. Extend
   the first-iteration record with completion/selection lineage, not caller-
   supplied replacement datasets or evidence. Recover completed steps without
   repeating provider calls,
   dataset publication or training; account separately for uncertain attempts.
4. Connect this same execution path to Overview and verify report associations
   across all iterations.
   Provider calls, Agent activity and results must refer to the same run and
   iteration; the UI must not maintain a separate imitation of the workflow.

Use the existing components as the implementation starting points:

- `encoder-optimization-core` and `encoder-optimization-runner`: bounded Agent
  inspection/proposals, generation and provider-neutral execution contracts.
- `project-workspace-local`: persisted Agent/generation calls, exact provider
  pins and immutable dataset publication. Add verified iteration lineage here;
  do not relax the first-iteration restriction to accept arbitrary datasets.
- `encoder-experiment-nomos`: native development diagnostics, training-row
  inspection, generation admission and training/evaluation adapters. Complete
  derived-population qualification before handing data to training.
- The workspace optimization CLI: compose these components into the production
  run path, then have the desktop invoke that same coordinator.

Keep the original root setup and preparation immutable. Generated dataset
versions and later candidates belong to their own iteration records; do not
overwrite the initial dataset or reuse a single-candidate receipt as if it
represented every iteration. The existing report fix must continue to work for
historical fixed-recipe runs as well as new iteration-linked results.

Report implementation status separately as component-tested, CLI-integrated,
and app-verified. Do not call the agentic system fixed until the configured
providers actually participate in the verified end-to-end journey.

For each checkpoint, report the concrete user-visible capability added, the
production path exercised, and the remaining integration gaps. Record completed
foundation work here instead of repeatedly treating it as new work. Keep the
full end-to-end acceptance criteria below unchanged until they are satisfied.

The next implementation checkpoint must demonstrate one complete composed
cycle, not just another isolated component. Intermediate components may be
committed when verified, but do not describe them as a working agent loop.

The checkpoint's executable proof must enter through the production coordinator
with deterministic provider and training adapters. It must load the recorded
iteration inputs, inspect a saved development failure and relevant training
rows, persist the Agent's evidence-linked removal/addition proposal, invoke the
separately pinned generator, qualify and publish the derived dataset, train its
candidate, and persist development comparisons against the original baseline.
Assert the resulting linked records and actual Agent/Generation activity, not
just success messages. Connect these operations inside the coordinator; a test
that manually calls disconnected components in sequence is not sufficient.

Do not start a new real Nomos run, change its provider assignments or modify its
historical records to demonstrate this checkpoint. Keep the existing fixed-run
compatibility and agentic-settings guard until the new route genuinely enforces
its authorization. After the composed cycle, finish repeated iterations,
stop/resume and Overview integration before calling the overall goal complete.

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

Keep implementation checkpoints bounded and communicate concrete progress.
When the user asks for status or says stop, pause implementation and answer
immediately. A passing component test, another documentation update or a build
is not a substitute for completing the missing production connection.

### Acceptance checklist

Keep these unchecked until the corresponding production-path evidence exists:

- [ ] One Optimize action invokes the pinned Agent and, when additions are
  proposed, the independently pinned generator through the real coordinator.
- [ ] A deterministic end-to-end test traces inspected failure evidence to an
  actual public Agent explanation, removal, generated addition, qualified
  dataset version, trained model and development comparison.
- [ ] A second iteration consumes the first result and produces an
  evidence-dependent next proposal while preserving the original baseline and
  benchmark. Explicit no-change decisions end the loop without invented edits.
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
