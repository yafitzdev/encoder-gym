# Goal: evidence-driven dataset repair in the existing optimization loop

Status: scoped specification, not implemented or accepted.
Date: 19 September 2026.

## 1. Outcome and scope

Make each bounded optimization iteration answer five inspectable questions:

1. What does the development evaluation show is wrong?
2. What does the training dataset contain in those areas?
3. What specific data intervention will test the Agent's hypothesis, and why
   this many rows?
4. Which proposed changes actually passed admission and reached training?
5. What changed in the targeted development metrics and the global gates?

Deliver this through the existing managed Nomos optimization loop and its CLI.
Expose the same persisted facts in the existing Overview as a separate final
UI phase. Do not build another coordinator, another training pipeline, or a
general-purpose dataset agent.

The release improves the quality and auditability of repair decisions. It
does not promise that every iteration improves the model. A justified no-change,
an admission rejection, and a scientifically rejected candidate are valid,
distinct outcomes.

This file replaces the completed one-click-loop goal and its historical
checkpoints. Git retains that history. It extends the existing
[agentic optimization contract](docs/features/optimization/agentic-optimization-spec.md);
historical launches keep their original behavior. This editing task authorizes
planning/documentation only, not implementation or external execution.

## 2. Existing foundations: preserve, do not rebuild

The following already exist:

- A finite, resumable CLI/desktop Agent loop, independent Agent/generator pins,
  cumulative budgets, immutable dataset versions, native qualification,
  training, development evaluation, and deterministic candidate selection.
- Protocol V2's full local training scan and aggregate landscape, joined on
  expected capability, task kind, scenario family, and eligible candidate-pool
  size; bounded cluster-row inspection.
- Evidence-linked removals and generation requests. Generation currently changes
  only a question while preserving its template's native context and labels.
- Saved repair-plan history, a first-batch native-admission canary, and an
  on-demand viewer of saved development disagreement samples.

The gaps are not simply missing clustering or missing orchestration:

- Retrieval support is currently projected from the combined scientific report.
  In `evaluation.rs` that report's support can be the minimum of retrieval
  states and agent sessions. `dataset_landscape.rs` uses it as a retrieval-share
  denominator; `development_comparison.rs` uses it to bound retrieval samples.
  These are different populations and must not be conflated.
- V2 closes inspection after one successful landscape and cluster inspection.
  Samples are content-hash selected, not selected for distinct native contexts
  or suspected defects.
- A generation target is an inspected template, instruction, count, and evidence
  IDs. It does not encode a repair hypothesis, percentage basis, anchor diversity,
  measurable expected result, or allocation shortfall.
- Native admission checks structure and duplicates, not whether a generated
  question still supports the inherited label. The current canary covers one
  target/context only.
- Saved failure samples do not establish complete confusion rates or prove that
  an absent case was fixed. Existing iteration history is not structured
  hypothesis/outcome memory.

Do not base implementation on a display ordinal such as "Run 46". Tests and
diagnostics must identify the root run, iteration, model, dataset, reports, and
pinned protocol explicitly.

## 3. Release boundaries

The first implementation is Nomos-specific at the adapter boundary and
task-neutral in the core. Use deterministic native dimensions, not embeddings.

Supported interventions:

- Add label-preserving question variants across multiple inspected training
  contexts for a diagnosed cluster.
- Add contrasting examples for a recorded capability confusion, using existing
  inspected anchors for both sides. A true contrast pair requires compatible
  native context and independently supported labels.
- Remove individually inspected exact redundant model inputs when the native
  adapter proves equivalent context and labels and identifies the retained row.
- Record suspected faulty labels, unsupported contexts, and ambiguous examples
  as review findings. Do not silently relabel or delete them.

Deferred: new registries, new candidate pools or state construction, automatic
label correction, automatic removal based only on an LLM verdict, semantic or
embedding deduplication, arbitrary topic discovery, and evaluation redesign.
A zero-training-coverage cluster with no valid anchor is an explicit unsupported
repair, not permission to use a benchmark row as a template.

"Politics" is a usable cluster only if the task actually declares that dimension.
Do not invent a generic topic taxonomy for Nomos or infer domain labels from
question keywords.

## 4. Trustworthy development evidence

Create a versioned diagnostic projection over the exact already-saved native
development reports. It must pin model, suite, suite fingerprint, report,
native artifact fingerprint, and projection version.

Required facts:

- Retrieval population: evaluated state count; existing aggregate and
  dimension metrics; per-cluster support and metric direction.
- Agent evaluation: session count and metric-specific denominators when saved,
  kept separate from retrieval states. Unavailable denominators remain unknown.
- Original-baseline and current-candidate values with comparable population
  identities. Never compare rates as though their denominators were identical
  merely because they share a report.
- Diagnostic availability: available sample, available empty sample, missing,
  corrupt, or incompatible, with a bounded actionable reason.

Use native retrieval `metrics.states` for retrieval shares and sample bounds.
Validate each cluster's support against that population. Multi-label capability
memberships may sum above the population; each individual membership share may
not exceed 100%. Keep counts exact and mark errors reconstructed from rounded
rates as estimates.

Do not change historical scientific support, acceptance gates, report IDs,
fingerprints, or saved Agent tool results. Correct passive projections from
verified source artifacts; new adaptive inputs use the new protocol. If required
native evidence is unavailable, report that limitation before Agent spending
instead of substituting another population or rerunning inference.

Retain the existing failure-sample boundary. Aggregate clusters are complete
only where the saved native report says so; confusion examples derived from its
at-most-50 disagreements are sampled evidence. A case missing on one side is
not a fix or regression. Full prediction collection and new evaluation cases
remain outside this release.

## 5. Dataset inventory and bounded investigation

Extend the existing full local scan, not the prompt size. Produce one immutable
inventory for the selected dataset fingerprint and native inventory version.

For each existing dimension/value cluster retain:

- Stable semantic cluster key, separate from the evidence fingerprint that
  changes across reports or dataset versions.
- Training count/share, development support/metrics, baseline deltas, missing
  evidence flags, and deterministic priority.
- Counts of distinct native context fingerprints and generation capacity.
- Exact duplicate groups and contradictory-label groups under the same
  native model-input identity; conflicts are findings, not automatic repairs.

Retain the existing priority order: descending estimated top-1 error count,
recall-at-1 regression, coverage gap, then stable cluster key. This is a browsing
order, not an additive error total or an automatic row-allocation formula.

Reuse the native renderer/validator for input and context identity. Context
includes the trainer-visible candidate pool and state/history, not registry ID
or question text alone. Distinct contexts must not be discarded as duplicates.

Extend bounded inspection to return representative, contrasting, and suspect
training examples. Selection is deterministic: distinct context strata first,
then content fingerprints. Return selection method, inspected count, total
eligible count, and continuation cursor. Only actually returned rows become
eligible anchors/removals.

The new protocol permits additional pages within the frozen Agent-turn/token
limits; one successful page must not force an immediate decision. A final
proposal-only turn remains reserved. If evidence is insufficient, submit
no-change with the specific limitation.

V3 requires at least four planning turns: landscape, examples, plan preview,
submission. Standard keeps eight and Quick test keeps four. Custom settings
below four are rejected at readiness, not discovered after provider calls.

Frozen first-release limits:

- At most four repair targets per iteration.
- At most eight inspected generation anchors per target.
- At most 32 distinct training rows inspected per iteration, including removals.
- At most eight requested additions per anchor across the entire iteration.
- Existing per-item/page byte limits remain enforced; continuation never clips
  native row content or grants access to an unreturned row.

These are safety/capacity limits, not reasons to spend the whole allowance.
Show when a limit prevents a larger intervention. Pagination and context
selection must not grow prompts with the full dataset or full trace history.

## 6. Structured repair plan and exact allocation

Introduce a versioned `RepairPlan` with explicit targets; do not parse intent
back out of free-form instructions. Each target must contain:

- Target ID and stable cluster key(s), plus exact inspected evidence references.
- A concise public hypothesis, evidence limitations, intended failure pattern,
  and the alternative explanation the Agent considered.
- Operation: label-preserving variants, existing-anchor contrast, or proven
  redundant-row removal.
- Exact inspected anchor/member IDs and fingerprints; contrast relationships
  where applicable; individual removal reasons and retained-row identities.
- Desired addition count, count basis, allocation rationale, and exact
  per-anchor counts. Removal counts are always explicit row IDs.
- One existing target metric and desired direction, plus the unchanged global
  development gates to watch. This is a hypothesis, not a new acceptance rule.

Support only two addition-count bases in this release:

1. `absolute_rows`: an integer desired count.
2. `relative_cluster_growth`: integer basis points of the selected source
   cluster's current row count, rounded up once.

For example, +5% of a 400-row cluster means 20 additions, not five percentage
points of the whole dataset. Pin the 400-row base and 500 basis points. Changing
the denominator requires another plan. Percentage-point balancing is deferred.
An absolute request may describe an unsupported empty-cluster gap but cannot
execute without valid inspected anchors.

A pure compiler validates a proposed plan and returns an exact preview:

- Desired and executable additions/removals, per-anchor allocation, projected
  dataset size and cluster shares, unused allowance, and limiting constraints.
- Every removal plus every requested addition consumes the existing row-change
  ledger. Rejected generation is not refunded.
- Overlapping clusters never duplicate edit charges. A generated slot has one
  owner target; its descriptive memberships can overlap.
- Allocate an accepted target total evenly across its selected anchors using
  quotient/remainder and stable anchor-ID tie-breaking. Reject cap violations.
  For contrast targets, require an even total and declared anchor pairs;
  allocate pairs first, with equal counts on both sides and stable pair-ID ties.
- Never silently trim an infeasible plan or raise a ceiling. Return a typed
  constraint result; the Agent may revise within remaining turns or stop.
- Do not force a minimum edit volume or treat benchmark/training share matching
  as an optimization objective.

Expose preview through an application-owned tool and CLI contract. Submission
must reproduce the exact preview fingerprint before generation. Preserve legacy
proposal contracts rather than retroactively adding required fields.

## 7. Targeted generation and semantic admission

Keep the independently pinned generator and existing structured transport.
Each request pins target, anchor, strategy, slot, instruction, schema, and
prompt version. The host still owns labels, registry, state, identities, and
training partition.

Give the generator training anchors and a host-compiled abstract repair brief
from validated strategy/cluster fields, not free-form Agent rationale or raw
benchmark questions as generation examples. A contrast target uses inspected
training anchors on both sides; it cannot fabricate a competing label or claim
to create a new hard-negative candidate pool. Ordinary paraphrases remain an
honest strategy, not a claimed contrast intervention.

Add a bounded, blind semantic assessment for every structurally surviving
generated row:

- The reviewer sees the generated question, relevant native context, and legal
  candidate semantics. It does not see inherited labels, the desired answer,
  the Agent's persuasive rationale, or evaluation rows.
- It returns a strict native assessment: supported candidate IDs, ambiguity,
  context consistency, and concise issue codes/rationale. A second target-fit
  assessment sees the abstract strategy only after the blind answer is
  recorded; it cannot revise that answer. Both passes are required.
- The host validates exact row/request identity, permitted candidate vocabulary,
  and output shape. Admit only an unambiguous compatible answer set matching
  the native label policy, consistent context, and supported target fit.
  Abstention/no-tool semantics remain adapter-owned; unsupported cases fail
  closed rather than being forced into classification labels.
- Contradictory, ambiguous, unsupported, malformed, or unassessed additions do
  not enter the candidate dataset. An LLM assessment is evidence, not truth.
- A contrast pair is admitted together only if both sides pass their individual
  checks and the adapter verifies the declared contrast relationship.

Own the provider-neutral native-assessment extension in a feature-owned module
of `dataset-quality-core`, with a separate request shape from classification
`SourceRow`. Reuse quality-policy and assessment primitives where applicable;
do not force Nomos into the existing fixed-label audit. The Nomos adapter owns
native semantics; optimization consumes verdicts through a narrow port.

This is addition admission under a newly pinned finite launch policy. It does
not create an approved curation manifest, auto-approve existing human-review
workflows, or declare the unchanged source dataset semantically qualified.

Use the run's pinned Agent connection/model for review through the existing
provider transport; add no third provider selector or provider stack. Charge
reviews to the same cumulative advisor request/token/spend ceilings, with a
separate visible operation category and durable reservations. Review calls are
not planning turns, but have finite per-plan batch/attempt ceilings. Record
shared generator/reviewer models honestly; a separate call is not proof of
independent judgment. Preview must disclose review work before launch.

Each review pass batches at most eight rows, subject to the existing payload
bound; contrast pairs stay together. Reserve one attempt before dispatch.
Malformed or negative assessments are terminal evidence, not retry requests.
An interrupted/unknown review may have at most one fresh attempt on explicit
Resume, within remaining provider ceilings; the unknown charge remains.
The plan preview itemizes both passes, canary/bulk batch boundaries, planned
requests and token reservations. Feasibility is not a promise that retries
will fit. Missing review budget stops work; it never disables the gate.

## 8. Per-target canaries and publication

Replace the single global canary only for new-protocol launches:

- Reserve the first planned batch of at most two rows for every distinct
  target/anchor/strategy combination. A contrast pair is one coupled canary
  unit. Canary rows are part of requested additions, not extra edits.
- Require native admission and semantic assessment of every canary row.
  Run all canary units before dispatching non-canary batches.
- Any rejected canary prevents bulk generation and publication for that
  iteration. Persist `repair_not_executed: canary_rejected` and end the run
  normally with no new candidate; retain any earlier eligible candidate.
  Resume cannot regenerate a saved rejection.
- After passed canaries, bulk rows use the same per-row checks and ordinary
  bounded generation concurrency. No automatic refill or hidden retry loop in
  this release.
- Persist requested, generated, structurally admitted, semantically admitted,
  duplicate-excluded, and published counts separately, per target and anchor.

A bulk shortfall is an observed result, not a rewritten requested allocation.
Publish surviving additions plus validated removals only when at least one
actual edit survives. Record partial target fulfillment explicitly. Zero
surviving changes ends without training. Interrupted/unknown provider work
retains its conservative charge and normal explicit recovery rules.

Reuse the complete-population native qualification and training-to-benchmark
firewall before training, including all retained source rows. Check generated
duplicates against the full selected population and other generated rows using
the native model-input identity, not question-only matching across distinct
candidate contexts. Schema, duplicate, and isolation failures remain blockers.

## 9. Outcome feedback, not causal claims

After normal development evaluation, persist a `RepairOutcome` for every
target: hypothesis/plan identity, requested and actual edits, input/output
dataset versions, candidate, report references, relevant cluster metric
deltas/support, global gate verdict, and limitations.

Compare to the same original baseline; also show the preceding candidate when
its population is comparable. Label target metric direction as improved,
regressed, unchanged, or unavailable. A positive delta does not establish
statistical significance or prove that a particular row caused it. Global
KEEP/REJECT and best-candidate selection remain unchanged.

Give the next iteration a bounded summary of earlier plans/outcomes within
this root run, including rejected candidates and admission failures. Use stable
cluster keys for continuity. An identical target/strategy/anchor intervention
with no new evidence must not be resubmitted as a new hypothesis.

Continue using the best eligible full dataset and the latest candidate's
development evidence, as today. Do not silently warm-start a rejected model.
Cross-run memory, learned allocation policies, bandits, and automatic tuning of
training settings are deferred.

## 10. Compatibility, ownership, and recovery

Pin this behavior as Agent analysis protocol V3 plus explicit repair/admission
policy versions. Keep V1/V2 tool contracts, optional-field serialization,
historical canaries, fingerprints, and replay behavior unchanged. No resumed
run silently upgrades. Activate V3 defaults only after connected acceptance;
unsupported adapters or insufficient evidence fail launch readiness clearly.

Keep existing iteration, row, provider, concurrency, and training ceilings.
Do not raise Standard or Quick-test defaults as part of this work. Account for
the extra review requests during plan feasibility and show budget-limited
capacity honestly. No policy expands authority after launch.

Ownership:

- `encoder-optimization-core`: repair-plan contracts, allocation, bounded
  investigation policy, and outcome summaries.
- `dataset-quality-core`: native row-assessment contracts and deterministic
  assessment policy, without provider or orchestration dependencies.
- `encoder-experiment-nomos`: native evidence projection, inventory/context
  identity, generation construction, and semantic assessment adapter.
- `encoder-optimization-runner`: bounded investigation, generation/canary/
  review orchestration through ports, not native business logic.
- `project-workspace-core/local`: frozen authority, append-only artifacts,
  custody, migrations, and cumulative accounting.
- CLI: compose existing owners. Desktop: typed passive projections and explicit
  existing execution controls, never a second policy engine.

Persist plan, slots, reservations, admission evidence, publication, and outcomes
before their dependent stages. Stop/restart/Resume must preserve completed
work, unknown charges, and exact artifact identities. Typed read failures must
not become empty successful results. Reading history never dispatches work.

Adaptive components may consume permitted development evidence, never sealed
rows, diagnostics, metrics, or protected overlap matches. The isolation gate
returns only aggregate clearance facts. Final holdout and promotion retain
their separate explicit authority.

## 11. Delivery order and staged commits

Implement one coherent component at a time, with its tests and contract docs.
Do not implement the whole specification in one unreviewed change.

1. **Evidence correctness.** Add population-specific diagnostic projections and
   typed availability; fix landscape/sample consumers. Prove that 1,000
   retrieval states, 16 agent sessions, and 50 saved disagreements are valid,
   and that a 100-state cluster has 10% retrieval support.
2. **Inventory and investigation.** Add stable cluster/context identities,
   suspect groups, diverse bounded sampling, and V3 inspection. Prove complete
   aggregate counts without sending the full dataset to the Agent.
3. **Repair-plan compiler.** Add strict targets, percentage semantics, exact
   allocation preview/submission, immutable persistence, and CLI reads.
   Prove budget, overlap, cap, stale-preview, and no-change behavior.
4. **Native semantic assessment.** Add core contracts, deterministic fake,
   Nomos adapter, provider accounting, and durable row-level evidence.
   Prove a schema-valid wrong-label question is excluded.
5. **Generation integration.** Wire structured targets, per-combination
   canaries, contrast coupling, complete-population deduplication, immutable
   publication, and existing qualification/training. Prove saved rejection
   cannot cause another call and shortfalls remain visible.
6. **Outcome feedback and connected CLI acceptance.** Persist target outcomes,
   feed the next iteration, and exercise success/rejection/no-change and
   interruption paths through the production CLI.
7. **Separate desktop phase.** After CLI acceptance and explicit authorization
   to start UI work, extend the existing Overview repair plan/case/report
   views. Preserve Setup -> Status -> Report, navigation, and Stop/Resume.
   Show the chain: weakness -> dataset evidence -> hypothesis/count basis ->
   actual edits/admission -> target outcome/global verdict. No redesign or
   new HTTP API. Then prove the production Electron/IPC/CLI journey.

After every executable component run `cargo fmt-check`, `cargo check-all`,
`cargo lint`, `cargo test-all`, and relevant Pi/native/UI checks. Review
dependency directions and commit the coherent verified stage when Git identity
is configured. Documentation-only edits require diff/content checks, not claims
that executable test gates were rerun. Push only when requested.

## 12. Acceptance and stopping criteria

The release is complete only when deterministic fixtures prove all of these:

- [ ] Population-correct evidence and explicit unavailable reasons; no false
      disappearance of valid retrieval samples.
- [ ] Exact complete dataset counts, overlap-safe accounting, reproducible
      diverse samples, and no eligibility for unseen rows.
- [ ] A weak cluster produces a concrete, justified, feasible multi-anchor plan;
      +5% of 400 rows reproduces exactly 20 requested additions.
- [ ] A missing-anchor gap becomes an honest unsupported/no-change finding;
      no development row enters generation as an anchor.
- [ ] A redundant row is removed with a retained equivalent; a suspected label
      conflict is exposed without unsupported deletion or relabeling.
- [ ] Wrong-label/ambiguous generated questions fail semantic admission even
      when their native schema is valid; unassessed rows cannot train.
- [ ] Each target/anchor canary gates bulk work; contrast pairs cannot be
      partially admitted; no hidden refill or new budget.
- [ ] Actual immutable dataset membership and training input reproduce all
      per-target published counts; full native qualification still gates training.
- [ ] A second iteration consumes the first plan and measured outcome, including
      rejection, without claiming causality or repeating an unchanged hypothesis.
- [ ] Budget exhaustion, no-change, canary rejection, execution failure, and
      scientific rejection are distinguishable through CLI and history.
- [ ] Stop/crash/restart around each new durable boundary reuses completed work
      and preserves unknown charges, without duplicate publication or training.
- [ ] V1/V2 persisted fixtures still replay unchanged, and sealed sentinels never
      appear in prompts, diagnostics, review requests, or adaptive outcomes.
- [ ] The final authorized UI phase displays the same recorded facts and passes
      `npm run verify:agent`, relevant renderer tests, and narrow/desktop checks.

Use temporary projects, loopback provider fixtures, and deterministic native
adapters. Do not use the user's production dataset as a development fixture.
A separately authorized live run may evaluate usefulness after acceptance; no
paid call, real training, replacement run, final evaluation, or promotion is
authorized by this specification alone.

Stop scope expansion here: no benchmark improvement, full-prediction evaluator
rewrite, training-backend replacement, Unsloth integration, cloud execution,
multiple workers, new autonomous agents, automated curation approval, or
general app redesign. Do not claim completion from a plan preview, isolated
unit tests, more generated rows, or one successful training run.
