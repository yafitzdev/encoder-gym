# Crash recovery

Generation, training, evaluation, and the governed encoder workflow are
foreground local workflows. Before a runner starts, the CLI persists an
execution lease with its PID and OS process start time. On every CLI startup,
SQLite reconciles `running` records with live processes. This avoids both
timeout guessing and PID-reuse errors.

```text
synth recovery scan
synth recovery list
synth recovery list --all
```

When the owning process is gone, the run is marked failed with an explicit
interruption message and a pending recovery record is created.

An interrupted encoder workflow keeps its current fingerprint-linked running
stage attempt and is moved to `awaiting_user`. Continue it with:

```text
synth workflow resume <WORKFLOW_RUN_ID>
```

Resume acquires the same single-process lease, discovers completed slice
artifacts by immutable identity, and links them rather than repeating work. A
retryable failed stage creates a new attempt of the same stage up to
`maximum_stage_attempts`; a completed attempt is never replayed. Successful
resume marks the pending `encoder_workflow` recovery record `resumed`. Use
`synth recovery dismiss encoder-workflow <RUN_ID>` only when intentionally
abandoning recovery.

Generation is resumable in place:

```text
synth recovery resume-generation <JOB_ID> --config project.toml
```

Resume uses the original backend identity and recomputes remaining work from
persisted per-cell accepted coverage. Existing accepted rows are never deleted
or silently repeated. Backend endpoint, model, generation parameters, batching
and retry policy, prompt-template identity, and semantic-context identity must
match the immutable execution specification or resume aborts before work
starts.

A provider request is recorded before network I/O. If its process disappears,
recovery marks the still-open attempt `interrupted`; it cannot assume whether
the provider completed the request. Failed and interrupted requests remain in
the attempt history and count toward the job's cumulative per-cell attempt
ceiling. Inspect the exact facts before resuming:

```text
synth job execution <JOB_ID>
synth job attempts <JOB_ID>
synth job prompt <JOB_ID> --cell-index 0
```

Training and evaluation are not resumed under the same ID: a partial active
batch is not an immutable continuation point, and an evaluation should not mix
prediction attempts. Their interrupted records remain inspectable. A completed
transformer checkpoint contains optimizer state and may be continued explicitly
as a new provenance-linked run:

```text
synth training continue <CHECKPOINT_ID>
```

This never changes the interrupted run or parent checkpoint. Without a
compatible completed checkpoint, start a new run after reviewing the failure.
A record can be acknowledged without a retry:

```text
synth recovery dismiss training <RUN_ID>
```

Analysis performs bounded read-only passes before writing. A crash during those
passes leaves no partial report; the completed report, normalized findings, and
evidence links are committed in one transaction. Re-run `analysis create` to
produce a new immutable report. Finding reviews are independent append-only
records. Optimization application stores its
generation plan and application marker in one transaction and is idempotent;
repeating approved `optimize apply` returns the original plan. Historical
unapproved applications use the clearly isolated `optimize legacy-apply`
compatibility path.

Decision-grade proposals, scenario groups, training candidate sets, campaign
headers, and outcome assessments are committed transactionally as immutable
artifacts. Reviews and campaign links are individually transactional,
append-only records. A terminated process therefore leaves either the complete
record or no record; rerun the explicit creation/link command after inspecting
`doctor`. Optimization has no execution lease because it never runs a model or
workflow. Recovery never approves, applies, rebases, links, assesses, resumes,
or schedules an optimization decision automatically.

`synth encoder optimize` is a different, explicitly executable finite parent.
Its definition reserves campaign, protocol, and experiment-run IDs before any
child side effect. Each `resume` performs at most one legal durable stage and
then returns. If the process exits, run `status` first and invoke the printed
next command; re-entry adopts only the exact reserved child or its complete
content-addressed output. It cannot mint a replacement child or spend a budget
twice. `cancel` closes the parent before another stage begins, while
`authorize-sealed` remains a separate immutable human action. Use `doctor` for
the expensive native replay and `provenance` for the row-free evidence bundle.

V1 stages are synchronous local calls. Cancellation is observed between stages,
not by terminating an already-running Python trainer. A crash after a native
checkpoint is fully materialized is recovered through the same candidate's
staging receipt and identity; a partial or foreign output fails closed.
