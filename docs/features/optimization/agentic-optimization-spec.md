# Input-first agentic optimization

Implementation status: settings contracts, strict CLI preview/authorization and
desktop history compatibility are implemented. The new execution components now
include a bounded Pi inspection/proposal loop, selected OpenAI-compatible model
transport, an append-only project Agent-call journal, read-only native
development-failure inspection, bounded concurrent generation, and recoverable
publication through ordinary imports and dataset versions. Bounded repeated
iteration is composed through the CLI below. Root lifecycle and desktop dispatch
are implemented, including ordered iteration/report navigation and immutable
Advanced/Quick-test settings and explicit post-loop final consent, recovery and
manual promotion. Offline connected Electron acceptance exercises the real
renderer/IPC/CLI/Pi path with deterministic loopback provider and native responses.
Windows abrupt-death regressions cover nine durable artifact boundaries; other
platforms retain recorded-child recovery without Windows spawn-time containment.
The fixed-recipe executor
still explicitly rejects agentic settings; it must not masquerade as an agent run.

The pre-training CLI composition now exists as `workspace optimization-run
<PROJECT> edit-dataset <RUN_ID>`. It binds the first iteration automatically,
loads exact saved development diagnostics and the selected training version,
invokes the pinned Agent through Pi, validates its evidence-linked proposal,
calls the independently pinned generator and publishes the ordinary dataset
diff. It acquires the existing local execution lease and reuses completed calls
and publication receipts. Normal `start` now reserves Agent authority without
executing it; legacy fixed-recipe materialization remains guarded. This command
only performs dataset edits, not a complete optimization run.

Its production-CLI test uses deterministic Pi wire responses and a loopback
generator, not paid models. It proves one inspected failure leads to a removal
and generated addition, changed project defaults do not switch pinned models,
and retry performs no new provider or native work. The fixture now enters
through normal production reservation and input preparation. Repeated
iterations and the Overview journey are not established by this test.

`workspace optimization-run <PROJECT> prepare-candidate <RUN_ID>` extends that
same composition through native materialization and complete-population
qualification. The derived version is rendered beneath the iteration's UUID,
never substituted into the root's preparation or fixed-candidate receipt. A
fixed embedded program imports the pinned Nomos validator and input renderer,
checks every training row, and audits exact/normalized native input plus declared
source, group and lineage identities against every pinned benchmark input.
Missing optional grouping identities are counted as limitations rather than
treated as one shared empty identity. Generated questions retain template source
ancestry. Only aggregate clearance facts leave the process; native diagnostic
text, stderr and protected rows do not. The request pins code, project, benchmark
and dataset identities, and a fingerprinted clearance is atomically published.
Retry validates the same inputs and reuses the clearance without invoking the
native program again. Invalid, duplicate or overlapping populations cannot
complete this command. It performs no training or model evaluation yet.

The production-CLI fixture exercises this extended command with a deterministic
native boundary and checks reuse and altered-receipt rejection. Separate Python
tests exercise the actual embedded audit algorithm, including five overlap
identities, invalid rows, duplicates, full-file hash verification and native-error
redaction. This remains pre-training integration, not the required composed
training/evaluation cycle or a live Nomos optimization.

`workspace optimization-run <PROJECT> complete-iteration <RUN_ID>` extends the
same composition through native training and development evaluation. An
append-only training binding pins the full qualified dataset, actual trainer
version/count, native artifact, resolved device, scientific project, candidate,
protocol and experiment. Quick test samples stable source identities only after
full-population clearance. A versioned native receipt verifies the exact input,
epochs, batch size, learning rate and device; old native receipts remain readable
only by their legacy recipes. Auto probes local CUDA availability once before
protocol reservation and retries reuse its resolved choice.

Each iteration protocol has zero sealed allowance. Its immutable result is
derived by replaying the ordinary scientific journal, requiring complete
development-suite coverage and no sealed authorization or report. A failed
scientific gate remains a completed development result, not an execution error.
The process regression uses deterministic native adapters, checks both suites
and exact sample membership, and injects failure after native work but before
result persistence; replay must not repeat provider, training or evaluation work.
The cycle also registers its verified trained model, including development
rejections, and links the exact trainer dataset version through ordinary model
custody. Materialized input evidence preserves original source-row identities
even for Quick-test subsets. Ordinary benchmark results follow the iteration's
explicit scientific receipts and expose saved development reports before the
iteration summary is written. Retry reuses model, dataset and report identities.
The bounded CLI composition now continues that same cycle through
`workspace optimization-run <PROJECT> drive-agent <RUN_ID>`. It records each
completed iteration, exact proposal-call identity, requested edit charges,
development result, deterministic best eligible dataset and terminal reason
before advancing. Requested additions consume the edit allowance even when
generation rejects them. Existing provider journals enforce cumulative request,
token and spend limits independently of this edit ledger.

The next iteration binds the previous completion, uses the latest candidate's
development metrics and saved failure sample, and starts from the best eligible
full dataset, or the original dataset when none qualifies. Training continues
from the pinned starting model, not an implicit warm-start from a candidate.
Quick-test subsets are actual trainer versions, never replacement source data.
No-change decisions stop without generation or training; iteration/edit limits
stop before another Agent dispatch. Historical first-iteration fingerprints
remain unchanged. Normal benchmark reports follow the whole predecessor chain.

Completion reads reconstruct choices from saved proposals and results. Before
admitting another iteration or returning a completed loop, the CLI replays the
original scientific journals and checks exact dataset custody. The deterministic
process tests cover changed second-iteration evidence and proposals, selection
of an earlier better dataset, first/later no-change, row-limit termination, and
interruption after scientific completion but before the completion record.
Retry reuses provider work, datasets, models and reports; re-fingerprinted
completion substitutions are rejected. Separate process-crash, post-loop final
and connected Overview tests extend this component evidence as described below.

Final consent is now a separate, row-free handoff after the adaptive loop closes:

- `workspace optimization-run <PROJECT> preview-final-agent <RUN_ID>` reads the
  exact selected full-data candidate, checkpoint and unchanged benchmark. It
  replays every completed iteration from its original scientific journal,
  including unselected candidates, and never writes or migrates the project.
- `authorize-final-agent <RUN_ID> --file <REQUEST> --authorized-by <ACTOR>` saves
  explicit consent for that preview. The strict JSON request contains a stable
  UUID `id` and the exact preview as `scope`. A changed preview is rejected;
  response-loss retries reuse the same grant. One immutable grant is permitted
  per root, including across competing requests.
- `final-agent-authorization <RUN_ID>` reads and revalidates the saved grant.
  It exposes identities, not protected rows, predictions or baseline scores.

The last completion and selected iteration are distinct: an earlier winner
remains the final candidate even when later results regress or stop without
changes. A terminal budget stop may retain an earlier eligible completion;
running, paused, interrupted, failed, diagnostic and no-winner runs cannot
authorize holdout. Authorization timestamps cannot precede adaptive closure.
The iteration protocols retain their original zero sealed allowance and are
never rewritten. These commands do not execute evaluation or promote a model.
`finalize-agent <RUN_ID> --authorization-id <GRANT_ID>` consumes that separate
grant, under the normal project and scientific execution leases. Before native
dispatch it verifies the original runtime, selected training materialization and
checkpoint bytes, then atomically reserves the only dispatch and the final report
and assessment UUIDs. A failed reservation cannot start work. Once reserved, an
allowance cannot be reused, including when a process died before native dispatch.

Only the transaction that creates the dispatch may invoke evaluation. Every
retry uses the backend's read-only completed-evidence recovery port; a missing
retrieval report, agent report or required trace leaves the outcome unknown.
Recovery cannot run a missing stage. Complete native evidence can reconstruct
the same reserved report and assessment after a lost project receipt. Acceptance
uses the original scientific metric contract and baseline final reference.

`final-agent-result <RUN_ID>` returns authorized, outcome-unknown or completed
custody, not an unverified claim of a live worker. Completed reads and retries
re-normalize native evidence and reproduce the deterministic assessment before
returning its decision. Scientific scores remain in the native evidence store;
the immutable project receipt contains only identities, normalization timestamps
and the acceptance boolean. Neither command projects scores into adaptive
history, changes an iteration protocol/journal or promotes the baseline.
`promote-final-agent <RUN_ID> --expected-baseline-revision <REVISION_ID>
--expected-final-receipt <FINGERPRINT>` is a separate manual action. It replays
the original development and final evidence, validates the full-data training
binding and registered checkpoint lineage, and appends an ordinary baseline
revision only for a final-accepted candidate. Exact retries reuse that revision;
changed requests, stale baseline authority and final rejection fail closed.
Promotion performs no evaluation or model duplication. Subsequent runtime
binding resolves the original accepted checkpoint through its final decision.

The typed desktop bridge now exposes explicit review, consent, execution,
read-only result lookup, recovery, owned-worker Stop and manual promotion.
Consent uses a main-process-owned scope and stable retry UUID, not renderer
authority fields. Recovery supplies the exact saved grant to the existing CLI;
unknown outcomes cannot acquire another allowance. Stop also cancels preflight
verification before dispatch. Report exposes explicit review, one-use consent,
Stop, saved-result recovery and accepted-only manual promotion. Reads and Optimize
never authorize holdout. The controller reloads saved custody after uncertain
replies; restart uses the same grant and reserved report identities. Quick tests
never show final controls, and launch authority is loaded before desktop dispatch.

The same CLI coordinator now journals root execution attempts before input
preparation, recording failure, interruption and exact adaptive completion.
Normal run show/list returns a separate `agentExecution` view and Agent-specific
root states without rewriting fixed-recipe history. Completion references the
verified last iteration and never implies promotion or final-holdout success.
A successor can mark an abandoned attempt interrupted only after acquiring the
exclusive PID/start-time-verified execution lease. Completed retries revalidate
scientific evidence without adding attempts or repeating completed work.
The process fixtures exercise failure before iteration completion, failure while
recording that error, and interruption before parent completion. This recorded
attempt status is not independently a live-process probe. The CLI now accepts
`stop-agent <RUN_ID>`, which persists Stop intent before interrupting work, and
`drive-agent <RUN_ID> --resume <EXECUTION_HEAD>`, which resumes only the exact
stopped head the caller observed. `stop-agent` accepts `--request-id <UUID>`;
clients must reuse it when retrying the same command. A new Stop while paused
still changes the head, invalidating an older Resume. Retrying an old Stop after
Resume never stops the new attempt. Compare-and-append checks the original
observed head inside the write transaction; it must not reload and adopt a newer
intent between admission and startup. Stop before dispatch is supported. `reconcile-agent`
records a dead coordinator only after obtaining its exact exclusive execution
lease, without dispatching any work. A live lease leaves the attempt untouched.
Permanent Cancel is unchanged.

Recorded-child recovery rechecks exact process identities before termination.
On Windows, transient termination-access failures while a child exits are
rechecked within the same finite cleanup deadline, not treated as either an
immediate failed recovery or proof of death. A still-live or unqueryable child
continues to block lease takeover. An exited child needs no new termination
handle.

The existing Agent/generator dispatch fences consume Stop intent. A scoped
observer propagates it to local/native file reads and native subprocess waits;
normal Stop confirms the owned child's termination before acknowledging pause.
Scientific training/evaluation interruption does not append a failed-candidate
verdict, and completed scientific outputs remain reusable. Unfinished training
may restart rather than continue an optimizer checkpoint. The coordinator lease
records exact PID/start-time identities for observed descendant processes. A
successor that finds a dead coordinator stops and waits for those exact children
before it replaces the lease or records the root attempt as interrupted; a live
descendant is projected as `orphaned`, not as a stopped worker. Root execution
records a native time-limit or exhausted allowance
as a distinct terminal `budget_exhausted` event; normal run projection exposes
`agent_budget_exhausted`, and another `drive-agent` call cannot create a retry
attempt or dispatch more work.
Continuation prompts retain the latest completed Agent turn in full, along with
the newest content-bearing result for each inspection capability. Older
duplicate results are projected as immutable item identities and fingerprints,
with their content explicitly marked compacted and available through a fresh
inspection. The durable trace remains complete. This prevents quadratic replay
of already-persisted row and development-evidence payloads without weakening
request reservation, cumulative token accounting, or proposal validation.
Inspection is a finite phase rather than an open-ended search loop. After one
successful call to each owned inspection capability, every later model call for
that iteration exposes only `propose_dataset_edits`; the OpenAI-compatible
transport also requires that single tool. The last permitted call is
proposal-only even when inspection did not complete, so the Agent must submit a
validated evidence-linked edit or an explicit no-change stop. Invalid proposal
attempts remain in the append-only trace and may be corrected within the
remaining turn budget. Pi advertises the strict tool schemas to the model but
forwards submitted arguments unchanged to the host validator. It must not drop
schema-invalid proposals or silently coerce their values before the host can
record the rejection; a rejected attempt still consumes its reserved call and
reported usage. The host remains the sole authority for accepting edits.
Reservation failures identify the exact exhausted input,
output, request, or spend dimension and its projected and configured limits.
The desktop uses the same `drive-agent`, `stop-agent`, and `reconcile-agent`
commands. New launches preview `--agentic`, selecting core-owned standard
defaults rather than the historical fixed recipe. Executor selection reads the
immutable launch even when a manually prepared run has no Agent attempt yet.
Stop retains its command UUID after an uncertain reply and uses a distinct UUID
for a later confirmed action. Resume carries the paused head displayed in the
renderer through IPC; main-process reads cannot adopt a newer Stop intent.
Concurrent stale-lease recovery is serialized with a separate per-run SQLite
lock. Windows CLI startup additionally establishes an anonymous kill-on-close
job before any dispatch. Descendants inherit membership at spawn, so abrupt
coordinator death stops even unobserved children; the job handle is not inherited.
The finite Agent coordinator drains remaining members before a terminal attempt
or Stop acknowledgement. Process-handle cleanup checks job membership (or the
exact legacy creation time), and lease child identities publish atomically.
Other platforms retain polling-based recovery. Windows process tests cover
death before a child's first instruction, reparented descendants and actual
CLI training interruption with conservative unknown charges. Another process test
kills the production coordinator at publication, qualification, training completion,
both development reports, model registration and the result/completion/root writes;
retry reuses completed provider and scientific work. Scientific stages drain their
owned job before lease release, and terminal Pi sessions are reaped immediately.
An empty initialized job takes precedence over stale recorded child PIDs.

Native training now reserves remaining time immediately before fresh process
dispatch, through an experiment-core accounting port backed by the project
database. Append-only attempts pin the iteration and exact training binding;
completion records retain measured milliseconds for success, Stop, failure and
deadline expiry. Iteration and whole-run limits include previous attempts and
unsettled reservations. A lost settlement keeps the full unknown charge, even
when the completed artifact is reused. Previously started unmetered iterations
also retain a conservative reservation rather than receiving a fresh allowance.
Artifact reuse does not dispatch another trainer or alter candidate identities.
Accounting failures and exhausted time leave scientific work resumable rather
than writing a failed-candidate verdict. The CLI exposes these facts with
`workspace optimization-run <PROJECT> training-time <RUN_ID>`. Root execution
records exhausted time as a dedicated terminal budget-stop view. Accounting-
integrity failures remain retryable execution failures because verified
completed artifacts may still be adopted after the persistence issue is fixed.

Agent and Generation activities are now projected from their own persisted
actions into the existing activity stream, rather than requiring desktop
re-emission under `optimization.run`. Generation has an explicit origin and
per-call action identity. In addition to renderer and CLI regressions, the connected
Electron journey opens these actual Agent/Generation activities from the same run.

The Agent runner reserves one call before each one-turn Pi session, retains
completed tool results for continuation, and rejects edits referencing
uninspected rows or development evidence. The project adapter accounts for all
pending and interrupted reservations and recovers only after checking the exact
worker PID/start time. Native diagnostic inspection reads existing retrieval
reports without running an evaluator; the historical evaluator's fifty-failure
sample is explicitly labeled as a sample. These component tests do not prove a
completed application/CLI cycle or a real Nomos improvement.

The first Agent iteration now has an append-only project input record before
call dispatch. It binds the root preparation, baseline revision, starting model
and dataset, benchmark, provider revision and every declared development report.
The CLI derives it from the benchmark's exact recorded scientific source;
only development references enter the record, not the source protocol's sealed
scores. Retries retain the original timestamp and identity. Agent-store opening
requires this binding and rejects substituted scope evidence. This first-input
handoff is consumed by the CLI paths above. Later iterations require the
verified completion/selection lineage described above; they cannot supply
arbitrary replacement evidence.

Generation uses the generation slice's separate structured-output port, without
recasting a native retrieval task as classification. The Nomos adapter currently
generates questions for inspected templates, preserving their registry, label,
router state and training partition. It rejects authority-field injection,
invalid native rows and duplicate questions; publication deterministically
excludes cross-batch duplicates and records each surviving row's call/slot source.
This is not semantic qualification or a substitute for the complete derived
population's training-to-benchmark firewall. Generation reservations enforce
separate cumulative request/token/spend allowances and in-flight concurrency.
Unknown outcomes retain their reservations, and reported overruns cannot authorize
edits or native rows. Custom-endpoint cost remains unknown without pinned pricing.

Native registration and full-population qualification use the same committed
manifest fingerprint after verifying a clean isolated checkout. Git's CRLF/LF
checkout conversion does not change runtime authority; an actual revision,
manifest, baseline or training-binding change still invalidates the pinned
identity. CLI fixtures exercise qualification with a clean CRLF manifest.

Native clearance v2 identifies within-training duplicates by normalized native
query plus the sorted multiset of eligible candidates' native rendered views.
Distinct retrieval contexts remain distinct; registry IDs, registry order and
changed labels cannot disguise duplicate model inputs. This is exact rendered
input matching, not semantic or embedding deduplication. Benchmark isolation
remains query/source/group/lineage based: different candidates never excuse a
protected query match. The versioned program identity prevents reuse of a v1
receipt under v2 rules; historical receipts and datasets remain immutable.

The read-only CLI supports `workspace optimization-launch <PROJECT> preview
--setup <ID> --agentic`, `--quick-test`, or `--settings-file <STRICT_JSON>`.
These options are mutually exclusive. Omitting all three preserves the legacy
preview byte shape. Authorization persists the exact
settings and their derived bounds. `optimization-run start` only reserves the
authority; the legacy fixed-recipe `materialize` rejects Agent settings so they
cannot silently enter an executor that ignores them.

This specification extends the fixed-recipe execution in
`optimization-launch-spec.md`. It is explicitly authorized by the September 14,
2026 product request. It permits a bounded agent and concurrent generation
requests inside one local run, not multiple trainers or distributed workers.

## Run settings

Setup retains the five ordinary input selectors. An Advanced disclosure owns
an optional user objective, maximum optimization iterations, maximum agent
turns per iteration, maximum total row additions/removals, generation concurrency,
separate Agent/generator request, input/output token and spend ceilings, and
training device, epoch ceiling, batch size, learning rate and time ceiling.
Settings are immutable launch inputs, validated before authorization. The user
objective cannot alter budgets, acceptance, credential access or evidence roles.

One iteration means analyze development evidence, propose explicit training-row
edits, generate and validate additions, publish a new dataset version, train one
candidate, and compare it on the unchanged development benchmark. An agent turn
is one model call inside that iteration. Both have independent finite limits.
Defaults are three iterations, eight agent turns per iteration, 192 total row
changes and one generation request in flight. Concurrency is configurable from
one to sixteen. It does not bypass cumulative provider request/token/cost limits.

`optimization-launch <PROJECT> presets` supplies the core-owned Standard and
Quick-test defaults to the desktop. Optimize sends the exact selected settings
through the existing settings-file preview and start contracts. An uncertain
reservation retries its original UUID and settings, not a fresh authorization.
Historical settings are read-only and never float to current project defaults.

Optional `providerLimits` in settings contains complete `advisor` and
`generation` limits. Every value must be no greater than its pinned project
provider limit. Omission preserves historical serialization and fingerprints.
These are whole-run ceilings; the existing durable provider journals enforce
them across iterations, retries and unknown outcomes. Provider exhaustion and
Agent turn exhaustion use the same non-retrying budget-stop state as training
time exhaustion. Budget ceilings/reservations are not actual usage or spend;
unpriced costs remain unknown.

Desktop model assignments now persist the exact project connection UUID in the
non-secret provider revision. `workspace optimization-run <PROJECT> providers
<RUN_ID>` returns the run's original verified revision, not current defaults.
Connection-backed secrets use connection-specific child environment slots; they
never fall back to a mutable role assignment or its environment variable. Missing
connections/keys stop dispatch with an actionable error. Legacy role references
remain readable and may use their explicit legacy key/environment, but are not
aliases for newly assigned connections. Existing saved assignments without a
connection pin must be saved again before they can use connection-backed keys.

Quick test uses one iteration, four agent turns, eight edits, a deterministic
sample of at most 64 training rows and at most two minutes of training. It still
uses the configured agent and generator. Evaluation remains on the pinned
development benchmark; this may still take time. Quick tests never use final
holdout or qualify a model for promotion. A plain one-iteration normal run is
not called a quick test.

## Execution and recovery

The root pins baseline, starting dataset, benchmark and both provider revisions.
Each iteration pins its source dataset, development evidence, model-produced
public rationale, explicit edit proposal, generation attempts, validated dataset
version, candidate and evaluation. Source datasets and models never change.
Training schema admission belongs to the native task adapter; optimization
cannot publish arbitrary model output as valid training data. Development
observations are suitable for adaptation; protected-test rows and scores are
never agent inputs. Evaluation thresholds remain deterministic.

Every external call is reserved durably before dispatch. Completed work reuses
the same identity after restart. An interrupted request has unknown remote
outcome, consumes its reserved budget and is not described as exactly-once.
Concurrency includes outstanding reservations. Stop aborts local calls where
possible and prevents the next step from dispatching; immutable completed work
is retained. Rejected edits and failed attempts still consume their budgets.

After each development result the next iteration gets the previous result and
best eligible dataset so far. Finite limits and an explicit no-change decision
can end the loop early. Final holdout is used at most once, after all adaptive
work, for the deterministically selected candidate, only under matching launch
authority. It never feeds another iteration. The baseline is not auto-promoted.

## Presentation

The read-only `optimization-run history` command projects existing iteration
custody and the shared development-only benchmark reports. Overview consumes
that closed projection for its iteration selector, original-baseline gate
comparisons and ordinary model/dataset/benchmark navigation. Each iteration
retains its selected stage and scroll during live updates; returning to live
work is explicit. Native telemetry carries the iteration ordinal, and unscoped
run setup remains separate. Development gate acceptance does not imply the
candidate was selected as best, received final approval, or was promoted.
Rendered fixtures verify stage/iteration interaction and narrow layouts. The
connected production-path journey additionally opens actual candidate, dataset
diff and benchmark viewers after evidence-dependent iterations.

Run observations retain the newest durable root/Agent control heads when polls,
Stop replies and drive responses arrive out of order. Stored running state after
reopening is labeled unverified instead of paused; Stop can target that existing
run without starting a coordinator. Pending Stop remains distinct from an
acknowledged pause, and terminal/paused states outrank an unsettled desktop
promise for both stage and sidebar spinner presentation. Mutation locks remain
held until owned IPC settles; a completed or paused run is not labeled running
just because that response is pending. Stored state is not live worker discovery. The
connected restart journey requires explicit Resume against the displayed saved
head and verifies that completed inspection is not repeated.

One expandable run keeps Setup → Status → Report. Status displays an iteration
selector above that iteration's stage strip and activity. Finished iterations
remain inspectable; selecting history never snaps back to live updates.
Activities carry iteration, stage, timestamp and origin (Agent, Generation,
Training, Evaluation or System). Agent entries are actual model-produced public
explanations and tool decisions, not generated system captions or private
chain-of-thought. All stage activity remains paginatable without truncation.
The active iteration advances; the same strip never appears to go backwards.

Report compares iterations against the same baseline and benchmark, shows the
selected candidate and deterministic result, and labels quick tests clearly.
Provider usage is cumulative across the whole run with separate role totals;
cost is unknown unless reliably reported or calculated from pinned rates.

## Acceptance

Use deterministic agents, generation providers and trainers in ordinary tests.
Prove an evidence-driven removal and addition, a second iteration consuming the
first result, bounded concurrent dispatch, provider-budget exhaustion, invalid
native-row rejection, immutable dataset/model ancestry, no sealed leakage,
stopping and restart at each boundary, and no duplicated completed work.
Exercise the production CLI and desktop composition, not only isolated mocks.
Do not present settings in the production GUI until execution consumes them.
