# Run activity by stage

Overview's six status stages are selectors for the run's recorded activity.
Each stage includes its original attempt and later retries. Selecting an older
stage stays selected during polling; its scroll position is retained. Latest
returns to the newest work. Stage selection is navigation, never execution.

The desktop reads the selected run with:

```text
synth workspace activity <PROJECT> run <RUN_UUID>
```

This read-only query returns complete verified action chains linked to that run,
including actions whose run reference was appended at completion. It does not
apply the recent-project-action limit. The existing project-wide list remains
bounded. No historical JSON, event fingerprint or scientific record is changed.

The renderer no longer truncates history to 30 recorded or 100 merged entries.
Repeated counter ticks for the same uninterrupted task update one activity row;
different files, tasks, stages, narratives and retries remain available.
Stage-specific streams retain all these rows rather than a recent tail.

New progress events carry a `run_stage` reference. The native executor's training,
checkpoint-saving and evaluation substages refine the parent command stage.
Generic checksums inherit their enclosing stage. Older runs use their recorded
CLI command intervals plus native task transitions; missing historical telemetry
cannot be recreated. Queued telemetry coalesces consecutive counter updates, not
different tasks or narratives.

## Nomos Run 12 verification

Read-only inspection on 14 September 2026 found 14 linked actions and 4,701
immutable events. The compact task projection contains 678 entries:

| Stage | Entries |
| --- | ---: |
| Checking inputs | 124 |
| Preparing data | 84 |
| Starting | 142 |
| Training | 104 |
| Saving candidate | 166 |
| Evaluating | 58 |

The report's 26 comparison rows are 13 acceptance checks across two development
suites: `generic_holdout` and `retired_post_scaling`. Each suite checks five
retrieval metrics and eight agent/tool-use metrics. The original benchmark also
records two diagnostic metrics that are not acceptance checks. These definitions
come from the pinned benchmark, not new categories invented by the run or UI.
No benchmark, score, gate, baseline or run outcome was changed for this UI work.
