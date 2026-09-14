# Goal: complete actual agent-driven encoder optimization

Implement the missing optimization engine and connect it to the existing
Overview journey. The user selects a baseline, starting dataset, shared
evaluation benchmark, Agent model, and Data generation model, then presses
Optimize. The agent uses development evidence to decide which training rows to
remove and which gaps to fill, requests generation, trains a candidate, evaluates
it, and uses that result to decide the next bounded iteration.

The next deliverable is a working end-to-end optimization cycle, followed by
verified iteration and recovery. More settings, system narration, documentation,
or UI mockups alone do not complete this goal.

## Current state — 14 September 2026

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
- In-progress code now contains a selected-model Pi transport, bounded
  development/training inspection tools, validated edit proposals, an
  append-only Agent-call journal and a native development-diagnostic reader.
  These are foundations, not a connected optimization engine. They still need
  integration and end-to-end verification; their existence must not be reported
  as a working Optimize journey. Preserve and finish this work rather than
  rebuilding it or replacing it with another fixed recipe.
- Run 17's candidate reports exist in the scientific journal but are absent
  from its entry in project benchmark results. Diagnose and repair that result
  association without rewriting historical evidence or weakening comparability.

Preserve existing projects, runs, model custody, dataset versions, provider
connections, credentials, navigation and the user's approved Overview design.
Do not require another real Nomos run to rediscover the known missing executor.

## Immediate next task

Continue from the existing settings and in-progress execution components; connect
them through the production CLI before adding more presentation. The first
checkpoint must connect the configured Agent and Data generation
providers to the real CLI workflow: inspect development failures and training
rows, propose edits, publish a validated dataset version, train and evaluate.
Prove this composition with deterministic adapters before asking the user to
try another run. Do not spend the next checkpoint on another settings-only
change, mockup or synthetic Agent message. This checkpoint is not completion:
continue through bounded iteration, recovery and the production Overview journey.

The immediate integration must resolve the run's pinned credential connection,
not whichever connection is currently assigned in project settings. Persist the
Agent proposal, generation attempts and resulting dataset version as resumable
steps of the same run. Remove the fixed-executor rejection only when the new
path actually enforces the authorized settings. Do not unblock the UI by
bypassing that guard or silently ignoring unsupported settings.

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
