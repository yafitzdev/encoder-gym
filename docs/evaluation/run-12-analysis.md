# Nomos Run 12 — 14 September 2026

Read from the managed project and scientific journals; no run was started or
resumed during investigation.

- Project run: `b248e1d3-a54e-4d35-b90e-ac4fc7e5266b`.
- Scientific child: `07a795a1-0260-8040-9efc-66b47625eef2`.
- Candidate: `c9d7978f-bfa3-874e-a644-fc4e6afce017`.
- Selected training version: `9448e8ca-bae2-8689-90ae-73a26d0b3c49`, 6,800 rows.

## What happened

Times below are UTC.

| Time | Recorded work |
| --- | --- |
| 07:57:08 | Run reserved |
| 08:00:33 | Input preparation recorded |
| 08:04:47 | Selected dataset materialized |
| 08:10:01 | Scientific experiment attached |
| 08:14:31 | Native model loading |
| 08:15:36 | Native training progress |
| 08:15:54 | Checkpoint saving |
| 08:17:04 | Generic retrieval evaluation |
| 08:19:02 | Generic agent evaluation |
| 08:21:36 | Retired-suite retrieval evaluation |
| 08:29:38 | Retired-suite agent evaluation |
| 08:32:10 | Development outcome recorded: baseline retained |
| 08:33:22 | Candidate registration failed |

The native manifest records 33.21 seconds of training. Total desktop execution
to the registration failure was about 36 minutes. These are different clocks:
loading, mining, verification, evaluation and custody work are outside the
trainer's reported duration. The failed desktop action does not mean training
failed, nor does successful training mean the candidate improved the baseline.

## Registration defect

Registration opened the original runtime backend without restoring the managed
training override. It then inspected the materialized project, which correctly
required the run's exact dataset. Execution restored that override; registration
did not. The adapter rejected the mismatch before copying the model.

Registration also only understood the older repair-snapshot training handoff.
Managed direct-dataset fine-tunes require a separate handoff: verify the exact
materialization, model, native manifest and input membership; then retain the
selected dataset version as provenance. Both paths now exist. Resuming this run
can reuse the saved candidate and development outcome; it must not train again
or use final evaluation after the baseline-retained decision.

## Repeated verification and feedback

Each start/completion journal write previously opened the entire workspace in
full verification mode, hashing models and scanning imported rows. Those
bookkeeping calls now validate persisted identities and receipts without full
content scans. The executor still checks inputs before using them, and custody
still verifies newly produced checkpoint bytes. Hashing a new checkpoint is
necessary to record its identity; rehashing every unrelated input just to append
a status event was redundant.

The stage tracker remains above Activity. Filenames, native tasks, evaluations,
and advisor summaries appear in Activity; the latest task owns the small meter.
Spinner animations use the document clock through redraws. Stop is available
during setup before a run UUID exists as well as during execution, and interrupts
the exact worker instead of waiting for a macro stage. Incomplete native work
remains subject to the existing recovery contract.
