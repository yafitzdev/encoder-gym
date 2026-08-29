# Crash recovery

Generation, training, and evaluation are foreground local workflows. Before a
runner starts, the CLI persists an execution lease with its PID and OS process
start time. On every CLI startup, SQLite reconciles `running` records with live
processes. This avoids both timeout guessing and PID-reuse errors.

```text
synth recovery scan
synth recovery list
synth recovery list --all
```

When the owning process is gone, the run is marked failed with an explicit
interruption message and a pending recovery record is created.

Generation is resumable in place:

```text
synth recovery resume-generation <JOB_ID> --config project.toml
```

Resume uses the original backend identity and recomputes remaining work from
persisted per-cell accepted coverage. Existing accepted rows are never deleted
or silently repeated. A backend/model mismatch aborts before work starts.

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
