# Goal: Evidence-driven dataset repair V3

Status: backend/CLI release accepted; V3 is the default for new Standard and
Quick-test launches. Existing Overview presentation remains deferred pending
explicit UI authorization.
Owner: managed Nomos optimization loop.
Last updated: 19 September 2026.

## 1. Product outcome

Build one finite, resumable optimization loop that can improve a training
dataset from development-evaluation evidence without exposing sealed benchmark
data or inventing unsupported labels.

For every iteration, the system must make this chain inspectable:

1. Which development weakness was observed?
2. What relevant training data exists, is missing, duplicated, or suspicious?
3. What exact dataset repair is proposed, and why that repair and row count?
4. Which generated or removed rows passed deterministic and semantic checks?
5. What immutable dataset was trained?
6. What changed in the target metrics and the global development gates?
7. What should the next iteration avoid repeating?

The intended workflow is:

`evaluate -> inspect -> plan -> canary -> generate -> admit -> publish -> train
-> evaluate -> record outcome -> repeat or stop`

This release improves decision quality, traceability, and recovery. It does not
promise that every intervention improves the model. A justified no-change,
unsupported repair, rejected canary, zero surviving edits, failed execution,
and scientifically rejected candidate are valid and distinct results.

## 2. Release boundary

### In scope

- The existing managed local optimization run and production CLI.
- Nomos-specific evidence, inventory, context identity, generation, and native
  semantic assessment behind task-neutral core contracts.
- Analysis protocol V3 for new launches only.
- Immutable repair plans, review evidence, publication facts, and outcomes.
- Bounded outcome memory within one root run.
- Deterministic tests, loopback-provider tests, persistence/recovery tests, and
  legacy replay tests.
- Activating V3 in the Standard and Quick-test presets only after all connected
  acceptance tests pass.

### Separate final phase

After the CLI/backend release is accepted and UI work is explicitly authorized,
extend the existing Overview to display the persisted V3 facts. The UI must be a
passive view and control surface, not a second policy engine.

### Out of scope

- Evaluation redesign or collecting full new prediction corpora.
- New registries, candidate pools, task state, or label taxonomies.
- Automatic relabeling or deletion based only on an LLM judgment.
- Embedding or semantic deduplication.
- Warm-starting from a scientifically rejected model.
- Cross-run memory, learned allocation, bandits, reinforcement learning, or
  arbitrary autonomous agents.
- New training backends, Unsloth integration, cloud/distributed execution,
  multiple workers, authentication, or a general app redesign.
- Paid provider calls, production-dataset mutation, final evaluation, or model
  promotion as part of development acceptance.

## 3. Existing system to preserve

Do not replace or duplicate these foundations:

- Finite local runs with Stop, Resume, cumulative budgets, and frozen settings.
- Immutable dataset versions, model candidates, evaluations, and provenance.
- Independent Agent and generator connection/model pins.
- Native qualification, training, development evaluation, and deterministic
  candidate selection.
- V1/V2 launches, saved tool results, historical fingerprints, and replay.
- The training-to-benchmark firewall and sealed-evidence isolation.

No resumed or historical run may silently upgrade to V3.

## 4. Required V3 contracts

### 4.1 Development evidence

Create a versioned diagnostic projection from already-saved native development
reports. It must pin the model, suite and suite fingerprint, report, native
artifact fingerprint, and projection version.

The projection must keep these populations separate:

- Retrieval metrics use evaluated retrieval-state count.
- Agent metrics use Agent-session or metric-specific denominators when saved.
- Saved disagreements are a bounded diagnostic sample, not a complete error
  population.

Unavailable denominators remain unknown. Missing, empty, corrupt, and
incompatible diagnostics are distinct states with actionable reasons. Do not
rerun inference to fill a missing projection.

Comparisons are valid only for compatible population identities. Existing
scientific reports, gates, support values, and fingerprints remain unchanged;
V3 corrects only the passive/adaptive diagnostic projection.

### 4.2 Dataset inventory and bounded inspection

Build one immutable inventory for the selected dataset fingerprint and native
inventory version. For each declared native dimension/value cluster record:

- A stable semantic cluster key.
- Training count and share.
- Development support and comparable metrics.
- Original-baseline and current-candidate deltas.
- Missing-evidence flags and deterministic priority.
- Distinct native context count and generation capacity.
- Exact duplicate groups and contradictory-label groups under the native
  model-input identity.

Native context includes the trainer-visible candidate pool and state/history.
Question text or registry ID alone is not sufficient identity.

Inspection returns bounded representative, contrasting, and suspect training
rows. It selects distinct context strata first and content fingerprints second,
and returns the selection method, inspected and eligible counts, and a cursor.
Only rows actually returned to the Agent are eligible as anchors or removals.
Development or benchmark rows can never become generation anchors.

Priority remains deterministic: estimated top-1 error count, recall-at-1
regression, coverage gap, then stable cluster key. Priority is a browsing order,
not an edit-allocation formula.

### 4.3 Repair plan and exact compiler

The Agent submits a versioned structured `RepairPlan`; the host never parses
dataset actions from prose. A plan contains at most four targets. Each target
must include:

- Target ID and stable cluster key(s).
- Exact inspected evidence references.
- Public hypothesis, intended failure pattern, evidence limitations, and one
  alternative explanation.
- One supported operation.
- Exact anchor, contrast-pair, or removal identities and fingerprints.
- Desired edit count, count basis, rationale, and exact per-anchor allocation.
- One existing target metric and desired direction.

A V3 plan with no edit targets is valid only when it carries one explicit stop
reason: `no_change` when the evidence supports leaving the dataset unchanged, or
`unsupported_repair` when the observed weakness cannot be repaired with the
inspected legal operations. Historical protocols keep their legacy stop shape.

Supported operations are limited to:

1. Label-preserving question variants from inspected training anchors.
2. Existing-anchor contrasts with compatible native contexts and independently
   supported labels on both sides.
3. Removal of an individually inspected redundant model input when the adapter
   proves equivalent context and labels and identifies the retained row.

Suspected bad labels, unsupported contexts, and ambiguous rows are findings.
They are not automatically relabeled or removed.

Supported count bases are:

- `absolute_rows`: an integer number of additions.
- `relative_cluster_growth`: integer basis points of the pinned source-cluster
  row count, rounded up once.

Thus 500 basis points on a pinned 400-row cluster means exactly 20 additions.
Changing the denominator requires a new plan.

A pure compiler returns the exact executable preview:

- Desired and executable additions/removals.
- Per-anchor or per-pair allocation.
- Projected dataset size and descriptive cluster shares.
- Unused allowance and typed limiting constraints.
- Stable preview and intervention fingerprints.

Allocation is even by quotient/remainder with stable identity tie-breaking.
Contrast totals must be even and both pair sides receive equal allocations.
Overlapping descriptive memberships never double-charge an edit. Every removal
and requested generation slot consumes the existing row-change ledger; rejected
generation is not refunded. Infeasible plans are rejected, never silently
trimmed and never granted a larger budget.

Submission must reproduce the accepted preview fingerprint exactly.

### 4.4 Targeted generation and native semantic admission

Each generation request pins target, anchor, strategy, slot, instruction,
schema, and prompt version. The host continues to own labels, registry, state,
identities, and training partition.

The generator receives the inspected training anchor plus a host-compiled
abstract repair brief. It does not receive raw benchmark questions or persuasive
Agent rationale. Ordinary paraphrases must not be presented as contrast data.

Every structurally surviving generated row then receives two durable checks:

1. Blind native-answer assessment. The reviewer sees the generated question,
   relevant native context, and legal candidate semantics, but not inherited
   labels, desired answer, Agent rationale, or evaluation rows.
2. Target-fit assessment. After the blind answer is recorded, the reviewer sees
   only the abstract strategy and judges whether the row fits the target. It
   cannot revise the blind answer.

Admission requires an unambiguous compatible answer set matching native label
policy, consistent context, supported target fit, exact request identity, legal
candidate vocabulary, and valid output shape. Unsupported, ambiguous,
contradictory, malformed, or unassessed rows fail closed. Contrast pairs are
admitted together or not at all.

The existing Agent connection/model performs semantic assessment through the
existing provider transport. There is no third provider selector. Each review
call is charged to the cumulative advisor request/token/spend limits with a
visible review operation category and a durable reservation.

Review batches contain at most eight rows and never split a contrast pair.
Malformed or negative assessments are terminal evidence. Only an interrupted or
unknown attempt may receive one fresh attempt on explicit Resume, budget
permitting; its unknown charge remains charged.

### 4.5 Per-combination canary and publication

For new V3 launches, the first planned batch of at most two rows for every
distinct target/anchor/strategy combination is its canary. A contrast pair is
one coupled canary unit. Canary rows count toward the requested edit total.

All canary units run and pass structural/native/semantic admission before any
bulk generation starts. One rejected canary:

- Prevents all bulk generation for that iteration.
- Publishes no candidate dataset for that iteration.
- Persists `repair_not_executed: canary_rejected`.
- Ends normally without masquerading as no-change.
- Cannot dispatch another provider call on Resume.

After canaries pass, bulk generation uses the same checks. There is no automatic
refill or hidden retry. Publication records requested, generated, structurally
admitted, semantically admitted, duplicate-excluded, and published counts per
target and anchor. A shortfall remains visible.

Publish surviving additions and validated removals only when at least one real
edit survives. Before training, run the existing complete-population native
qualification and benchmark-isolation checks over the exact candidate dataset.
Generated duplicates are checked against the full selected population and one
another using native model-input identity.

If the canary passes but later admission or qualification leaves no publishable
edit, persist `repair_not_executed: zero_surviving_edits`. It is terminal for the
iteration, publishes no dataset, starts no training, and remains distinct from
`canary_rejected`, `no_change`, and `unsupported_repair`.

### 4.6 Training, evaluation, and target outcomes

Training and scientific selection remain owned by the existing pipeline and
gates. V3 does not add an acceptance rule.

After development evaluation, persist one immutable `RepairOutcome` per repair
target containing:

- Root run, iteration, plan, target, and intervention identity.
- Requested, generated, admitted, published, and removed counts.
- Input/output dataset versions and candidate identity.
- Exact evaluation report references.
- Comparable target metric values/support and direction.
- Global KEEP/REJECT verdict from the unchanged development gates.
- Explicit limitations, including that a measured delta is not causal proof.

Target direction is `improved`, `regressed`, `unchanged`, or `unavailable`.
Compare against the same original baseline and, when population-compatible, the
preceding candidate. Never manufacture comparability.

The next iteration receives a bounded summary of earlier plans and outcomes in
the same root run, including scientific rejection and admission failure. An
identical cluster/strategy/anchor intervention with unchanged input evidence is
a typed plan constraint, not a new hypothesis. A materially different inspected
anchor, strategy, or new evidence may support a new plan.

## 5. Frozen limits

V3 must operate inside the existing launch budgets. It may not expand authority
after launch.

- Maximum repair targets per iteration: 4.
- Maximum inspected generation anchors per target: 8.
- Maximum distinct training rows inspected per iteration: 32, including
  removal candidates.
- Maximum requested additions per anchor per iteration: 8.
- Maximum rows per semantic-review batch: 8.
- Standard preset Agent turns per iteration: 8.
- Quick-test Agent turns per iteration: 4.
- V3 readiness minimum: 4 turns.
- One final proposal-only turn remains reserved.

Existing iteration, row-change, provider, concurrency, training-time, and
training-row ceilings remain unchanged. Limits are safety bounds, not quotas.
When a limit blocks a larger intervention, the plan records that constraint.

## 6. Persistence, recovery, and isolation

Persist every externally consequential boundary before the next stage:

- Repair plan and accepted preview.
- Generation slots and provider reservations.
- Blind and target-fit semantic evidence.
- Canary decision and non-execution receipt.
- Publication membership and per-target counts.
- Training/evaluation artifact references.
- Per-target outcomes.

All records are append-only or immutable under their natural run/iteration
identity. Reads must return typed failures; missing/corrupt data must never be
interpreted as an empty successful result. History reads never dispatch work.

Stop, crash, restart, and Resume must reuse completed work, preserve unknown
charges, and avoid duplicate provider calls, publication, training, and outcome
records.

Adaptive components may consume only permitted development projections and
training data. Sealed rows, sealed diagnostics, protected overlap identities,
final-evaluation metrics, and promotion evidence must never appear in Agent,
generator, reviewer, or outcome payloads. The isolation gate exposes aggregate
clearance only.

## 7. Compatibility and ownership

Pin the new behavior as analysis protocol V3 plus an explicit semantic canary
policy. Missing historical fields retain their legacy defaults. V1/V2 tools,
serialization, canaries, fingerprints, and replay remain unchanged.

Ownership remains:

- `encoder-optimization-core`: repair-plan contracts/compiler, bounded
  investigation policy, intervention identity, and outcome summaries.
- `dataset-quality-core`: provider-neutral native-assessment contracts and
  deterministic policy.
- `encoder-experiment-nomos`: native evidence projection, inventory/context
  identity, repair metric projection, generation construction, and assessment.
- `encoder-optimization-runner`: bounded Agent/tool sequencing and orchestration
  through ports.
- `project-workspace-core/local`: frozen launch authority, custody, append-only
  persistence, migrations, recovery, and cumulative accounting.
- CLI: composition and typed history/status output.
- Desktop, in its later phase: passive display and existing controls only.

## 8. Delivery plan and current status

Each stage is one coherent, tested commit or small commit series. “Implemented”
below means code is committed; the release is not accepted until Stage 7 passes.

| Stage | Deliverable | Status |
| --- | --- | --- |
| 1 | Population-correct development evidence | Implemented and committed |
| 2 | Immutable native inventory and bounded diverse inspection | Implemented and committed |
| 3 | Structured V3 plan, exact allocation compiler, preview/submission | Implemented and committed |
| 4 | Blind native semantic assessment, transport, accounting, persistence | Implemented and committed |
| 5 | Per-combination canaries, contrast coupling, gated publication | Implemented and committed |
| 6 | Per-target outcomes and bounded within-run outcome memory | Implemented and committed |
| 7 | Recovery matrix, legacy/isolation regression, full gates, V3 preset activation | Accepted and committed |
| 8 | Existing Overview V3 presentation | Deferred; requires explicit UI authorization |

### Stage 6: completed outcome feedback

Immutable per-target outcomes are now required before a newly executed V3
iteration completes. History projects their measured target direction and
unchanged global KEEP/REJECT verdict, and a bounded summary is supplied to the
next iteration. Connected fixtures prove recovery without repeated work,
scientific REJECT remains distinct from execution failure, and an unchanged
intervention is rejected as `repeated_unchanged_intervention` while materially
revised evidence remains eligible.

### Stage 7: accepted release and activation

The release matrix is covered by unit, persistence, adapter, UI-decoder, and
managed production-CLI fixtures. New Standard and Quick-test presets now pin
analysis protocol V3 and `per_combination_semantic_v3`; historical missing
fields and explicitly pinned V1/V2 launches preserve their prior behavior and
fingerprints. Terminal history distinguishes no-change, unsupported repair,
canary rejection, zero surviving edits, budget exhaustion, execution failure,
KEEP, and REJECT. The full repository gates passed on the release tree; the
Windows CLI integration suite was also run serially to avoid unrelated SQLite
contention between parallel process fixtures.

## 9. Acceptance matrix

The backend/CLI release is complete only when deterministic fixtures prove:

- [x] 1,000 retrieval states, 16 Agent sessions, and 50 saved disagreements
      remain separate valid populations; a 100-state cluster reports 10%
      retrieval support.
- [x] Complete dataset counts and context-aware duplicate groups are exact while
      the Agent receives only bounded samples.
- [x] Only inspected training rows can be used; an unsupported empty cluster
      produces an explicit no-execution result.
- [x] `relative_cluster_growth` of 500 basis points on 400 rows compiles to 20
      additions with deterministic multi-anchor allocation.
- [x] A proven redundant row names its retained equivalent; a suspected label
      conflict is reported but not deleted or relabeled.
- [x] A schema-valid wrong-label or ambiguous generated row fails semantic
      admission, and an unassessed row cannot train.
- [x] Every target/anchor/strategy canary gates all bulk work and a contrast pair
      cannot be partially admitted.
- [x] Published counts reproduce exact immutable dataset membership and training
      input; complete native qualification still gates training.
- [x] A second iteration consumes the first plan/outcome, includes a REJECT case,
      blocks an unchanged intervention, and can submit a materially revised one.
- [x] No-change, unsupported repair, canary rejection, zero surviving edits,
      budget exhaustion, execution failure, KEEP, and REJECT remain distinct in
      CLI status and history.
- [x] Stop/crash/restart never duplicates an external call, publication,
      training run, or outcome and preserves conservative unknown charges.
- [x] V1/V2 fixtures replay unchanged and sealed sentinels never appear in an
      adaptive request, diagnostic projection, or outcome.
- [x] `cargo fmt-check`, `cargo check-all`, `cargo lint`, and `cargo test-all`
      pass from a clean worktree, together with the managed CLI acceptance test.

The later UI phase is complete only when the existing Overview shows the same
persisted chain—weakness, dataset evidence, plan/count basis, actual admission
and edits, target outcome, global verdict—and passes its production Electron,
IPC, renderer, and narrow-window checks.

## 10. Stop condition

The backend release stopped after the Stage 7 matrix passed and the V3 presets
were activated. Do not expand it to improve the evaluator, replace the trainer,
add a provider, add autonomous workers, or redesign the app. Stage 8 is a
separate product phase and starts only after explicit UI authorization.

Do not claim completion from a plan preview, isolated unit tests, a larger row
count, or one successful model run. Release completion requires the connected
two-iteration memory path, recovery/isolation coverage, legacy compatibility,
full repository gates, and a clean committed worktree.
