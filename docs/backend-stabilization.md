# Backend stabilization handoff

Completed against the findings in [`backend-assessment.md`](backend-assessment.md)
on the isolated `codex/backend-stabilization` branch, starting at `d7810d9`.
The backend checkout is `.worktrees/backend-stabilization`; the original
checkout is reserved for the parallel GUI session. No GUI files, active GUI
goal, real experiment database, or model artifacts were changed here.

## Resolved findings

| Assessment finding | Result |
| --- | --- |
| Failed execution returned success | Generation, resumed generation, training, continued training, and evaluation now print their terminal result and return nonzero when execution failed or was cancelled. Reading an existing failed run remains a successful inspection. |
| Passive commands wrote to the database | Pure configuration operations bypass database startup. Passive readers use read-only SQLite connections, validate the existing schema, and perform no migrations or interruption recovery. Explicit database migration and recovery scan commands own those writes. |
| Diagnostics corrupted JSON stdout | Tracing uses stderr. A process regression verifies that SQL debug logging preserves exactly one JSON result on stdout. |
| Windows sidecar formatting failed after checkout | Package-local Git attributes enforce LF. The complete Pi check passed in a fresh checkout created with `core.autocrlf=true`. |
| Production optimization lacked full parent coverage | Five deterministic process journeys exercise ordinary parsing, SQLite persistence, slice contracts, parent orchestration, and a compiled fake task adapter. They cover new non-adopted runs, cancellation, development rejection, sealed rejection, promotion, training failure, and recovery between durable child work and parent journal updates. |
| Large modules and stale handoffs increased change risk | Extracted frozen repair-evidence loading, persisted experiment loading, and optimization reporting while fixing those paths. Shared the existing SQLite repair fixture with the process tests. Updated the handoff and removed the stale dependency on the GUI's `goal.md`. Broad unrelated module rewrites remain outside this change. |

## Lifecycle defects found by the new coverage

Sealed authorization can be retried by the same recorded authorizer without
appending another authorization or granting another evaluation. A different
authorizer cannot replace that authority. The parent can recover when child
authorization succeeded but linking it into the campaign failed.

An interruption after persisting the sealed assessment but before finalization
now resumes from that assessment. It does not evaluate the candidate again.
The successful parent journey also injects failures after campaign creation,
protocol preparation, run creation, development evaluation, and before the
parent completion event. Reserved artifact identities and backend call counts
remain stable across retries. Its total is one training operation, two candidate
development evaluations, and one explicitly authorized candidate sealed
evaluation, plus the three protocol baseline evaluations.

Optimization reports now work before protocol creation and after cancellation.
Their limitations describe the persisted run rather than repeating statements
from the historical failed Nomos experiment. Passive optimization commands and
terminal resume do not require native adapter files. Native execution and Doctor
retain project verification; Doctor's repair verification no longer registers
new project records as a side effect.

The full-suite run also exposed an intermittent loopback quality-test failure.
The isolated reproduction passed. Its fixture previously started its connection
deadline before several unrelated CLI setup processes; the fixture now starts
that deadline immediately before the audit and includes provider/audit details
in failure output. Production provider timeouts were not changed.

## GUI and scripting integration

- Check execution exit codes and retain the JSON result even on a nonzero exit.
  This preserves the failed run ID and recorded failure facts for inspection.
- Parse stdout as the command result; diagnostics and progress belong on stderr.
- Passive database commands require an existing database with the current
  schema. Initialize or upgrade explicitly with `synth database migrate`, using
  `--kind production` for the production experiment database. See
  [`operations.md`](operations.md) for recovery and database command details.
- A passive poll no longer marks interrupted work as failed. Use the explicit
  recovery scan or an execution path that owns startup recovery.
- Optimization status, reports, and terminal resume use persisted facts.
  Doctor and native execution remain the paths for checking the actual adapter
  workspace. Existing JSON field names and immutable artifact contracts remain
  intact; report limitation text is now derived from the run.

## Verification

- `cargo fmt-check`: passed.
- `cargo check-all`: passed for all workspace targets and features.
- `cargo lint`: passed with warnings denied.
- `cargo test-all`: passed across 112 test binaries, with 547 passing test
  executions, zero failures, and four explicitly opt-in tests ignored. The
  fixture binary repeats 17 shared CLI unit tests; the remaining 530 cases
  include the five new optimization process journeys.
- Local documentation links and `git diff --check`: passed.

The Pi sidecar's complete `npm run check` passed, including formatting, lint,
type checking, build, and all 11 tests, both in the isolated working package and
in a fresh Git checkout under Windows automatic CRLF conversion.

All ordinary acceptance paths use local fixtures. No paid provider, external
network request, model download, GPU, or real Nomos execution was required.
Loopback provider processes use synthetic test credentials. The opt-in live
provider, public model bundle, and real Nomos tests remain separate. Rust 1.98.0
was used; the historical Rust 1.85 minimum was not revalidated.

Core ownership and inward dependency directions are preserved. The test
composition uses the existing `EncoderTaskBackend` port and ordinary CLI
handlers; production `synth` has no runtime fake override. No new HTTP or GUI
surface, adapter SDK type in a core port, or alternative workflow implementation
was introduced. See [`development.md`](development.md) for the focused test
command and [`encoder-optimize.md`](encoder-optimize.md) for operator behavior.

This resolves the assessed findings; it is not an exhaustive proof that every
backend path is defect-free or that a new real optimization will improve a model.
