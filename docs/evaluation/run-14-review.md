# Run 14 and live Overview interaction

## Recorded execution

Read-only inspection on 14 September 2026 identified the project run
`28cc6a1a-a8f4-44a5-9f06-577fe99f54ce`, scientific experiment
`3548341b-7adc-8787-b9dc-cdcfcf14ee93`, and candidate
`37e0b744-2b02-85fb-a98f-4a36ea670794`.

The candidate's native `nomos_training_manifest.json` records `device: cuda`,
6,800 triplets, and 36.0918163 seconds of training. CUDA is explicitly chosen in
`NomosBackend::initial_training_candidate_definition`; it is not an
automatic hardware choice in the UI. The native parameter contract supports
CPU and CUDA, but this initial recipe selects CUDA. This observation does not
claim that input validation, hashing, or all evaluation use the GPU.

The scientific journal records training started at 10:40:01 UTC and its
completed output at 10:41:54 UTC. This wider interval includes adapter work,
not just the trainer's measured duration. Development results were recorded at
10:47:14 UTC (`generic_holdout`) and 10:57:57 UTC (`retired_post_scaling`). Both
failed their gates. The final decision was `retain_baseline`; no final holdout
was needed. No run was started, stopped, resumed, or changed for this review.
The desktop action subsequently recorded success at 11:05:25 UTC after model
registration succeeded. The registered model is
`15575f74-da22-4c59-b401-1f97ee14fe87`, linked to dataset version
`67da35d2-ed05-8b2b-bab3-124061e88261`.

## Interaction defect and fix

Progress redraws replaced the entire Overview DOM. A button could disappear
between mouse-down and mouse-up, and the native scrollbar could lose the
element being dragged. Restoring scroll offsets and spinner phase afterward
did not preserve those interactions.

Overview now opts into in-place DOM reconciliation. Run rows, stages, controls,
and the activity scroller stay connected across progress updates; event
handlers receive fresh state. Other pages retain their existing lifecycle.
Each stage restores its saved scroll position on entering that stage, not on
every progress tick. The active stage owns the spinner. Elapsed time and Stop
share the Activity heading; the duplicate current-stage heading is removed.

Electron regression tests press a stage, redraw three times before releasing,
and verify one click succeeds. They also drag the native scrollbar across
redraws, revisit stage-specific scroll positions, and check spinner continuity,
stop/recovery, and narrow layouts.

## LLM usage boundary

The current input-first Nomos executor trains its adapter-defined candidate and
runs local evaluation. The configured advisor and data-generation providers are
not invoked by this path; bounded advisor/generation iterations remain open in
`../features/optimization/optimization-launch-spec.md`. The activity stream's system summaries are not
evidence of an LLM request. Local evaluation prompt-token metrics are not
provider-billed agent/data-generation usage.

Per-role token and price metering is not implemented for this path. It should
consume durable provider-call receipts linked to this project run: input,
output and cached-token categories, provider/model identity, and reported cost
or an estimate using a pinned tariff. Unknown usage or price must remain
unknown, including interrupted requests; configured ceilings must never be
presented as actual consumption. No fabricated counters or assumed prices were
added to the Overview.

## Validation

Passed UI typecheck/build, all 116 UI unit tests, the isolated Electron Overview
mouse/scroll/layout checks, and the full desktop smoke suite. The four Rust
gates (`fmt-check`, `check-all`, `lint`, `test-all`) passed. The initial full
test attempts encountered a Windows executable lock; they were rerun after
the existing processes finished naturally, without terminating user work.
