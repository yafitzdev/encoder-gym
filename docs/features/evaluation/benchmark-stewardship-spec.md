# Benchmark stewardship specification

## Purpose

Benchmark stewardship owns the evidence boundaries that make development and
acceptance claims trustworthy. It does not train encoders, generate rows,
choose dataset membership, or optimize against sealed evidence.

An immutable benchmark bundle proves that development and sealed cohorts are
mutually clean. Stewardship extends that guarantee to the data consumed by a
trainer and, in later milestones, to the fitness of the benchmark itself.

## Milestone 1 — training-to-benchmark leakage firewall

Every workflow training stage must possess one immutable
`TrainingBenchmarkCheck` for the exact training snapshot and exact benchmark
bundle. This applies independently to the initial snapshot and every iteration
snapshot.

The trainer-input protocol is versioned as `train_and_validation_v1`:

- every member in the snapshot `train` split is included;
- every member in a non-empty `validation` split is included;
- the snapshot `test` split is not included because Slice 3 training backends
  cannot consume it;
- changing these rules requires a new protocol value and therefore a new
  fingerprinted check.

Each consumed split is registered as an internal evaluation cohort with an
active `Training` role. The cohort pins the immutable snapshot fingerprint and
split. The check pins the exact cohort and role-decision identities.

### Check inputs

The contamination population is exactly:

```text
training snapshot train cohort
+ optional non-empty training snapshot validation cohort
+ every development cohort in the BenchmarkBundle
+ every optional sealed cohort in the BenchmarkBundle
```

No participant may be missing, duplicated, or added. Benchmark participants,
their current roles, snapshots, splits, and fingerprints must still match the
bundle authority.

Use the bundle's global-report group dimension and the default zero-tolerance
`ContaminationPolicy`. Detect:

- shared source-row identity;
- exact text overlap;
- normalized text overlap;
- shared configured group identity.

Missing required group values fail closed. The comparison implementation may
use indexes, but its normalized findings and fingerprint must remain identical
to the deterministic contamination contract.

### Immutable check

`TrainingBenchmarkCheck` records:

- its identity and fingerprint;
- training snapshot ID and fingerprint;
- trainer-input protocol and selected-population fingerprint;
- canonical training cohort bindings (split, cohort, role decision, and
  fingerprints);
- benchmark bundle ID and fingerprint;
- the canonical benchmark cohort ID union;
- contamination report ID and fingerprint;
- clean or blocked status;
- creation time.

The artifact fingerprint covers all authority fields except its random ID and
creation time. A clean check requires a reproducible clean report with zero
findings, zero counts, no reasons, a strict policy, and no override. A blocked
check is durable evidence, not an approvable exception.

### Persistence and integrity

Persist any new training cohorts, their initial role decisions, the
contamination report, and the check in one SQLite transaction. Enforce one
check per snapshot, benchmark bundle, and trainer-input protocol.

Loading a check must fail closed unless persistence can:

1. reproduce the check fingerprint and normalized columns;
2. load and verify the complete snapshot membership;
3. reproduce the selected-population fingerprint;
4. load the exact executable benchmark bundle and its suites;
5. verify every pinned role is still current and active;
6. recompute the contamination report from persisted cohort members;
7. prove that the report participant set and all bindings are exact;
8. prove that status and fingerprints agree across the full chain.

Historical workflow definitions without benchmark-bundle authority remain
inspectable and non-executable under the existing compatibility policy.

### Runtime placement

For `Training` and `IterationTraining`:

1. validate any configured curation manifest/application;
2. create or deeply load the exact training-benchmark check;
3. stop with a non-retryable blocked reason unless it is clean;
4. build one ephemeral spool from the same verified member buffer and reproduce
   a versioned digest over the exact ordered labels and examples visible to the
   backend;
5. seal the source as ordered example fingerprints and reject any later backend
   read whose value differs from the verified sequence;
6. bind the training request and run to that input digest, population
   fingerprint/count, and exact check identity/fingerprint;
7. only then query exactly bound, configuration-compatible reusable runs or
   call a training backend;
8. link the clean check from the completed stage attempt.

The runner must reproduce the backend-facing digest before startup. Legacy or
standalone runs without the binding cannot be retroactively treated as governed
training evidence.

New training runs enter persistence only in their canonical queued state.
Persistence accepts only legal lifecycle transitions, requires each running
progress update to represent exactly one completed backend batch, and refuses a
completed run unless its exact final checkpoint already exists and agrees with
the final epoch, model identity, and run configuration.

Development evaluation, sealed assessment, finalization, and promotion must
reject checkpoint evidence whose source training snapshot is not linked to the
matching clean check for the workflow definition's benchmark bundle.
Promotion must additionally prove that both assessments evaluate the same
selected checkpoint and the workflow's exact development and sealed suites.

The check proves only the platform-visible fine-tuning population. It cannot
prove that an upstream base model's unknown pretraining corpus did not contain
benchmark material; surfaces and documentation must state that limitation.

### CLI

Add read-only benchmark-check inspection surfaces:

- list checks, filterable by snapshot, bundle, and status;
- show one check;
- validate one check through the deep persistence path.

Normal users should not need to create a check manually; governed workflow
training creates or reuses it automatically. Output must not disclose raw
benchmark text.

### Acceptance tests

At minimum prove:

- source-row, exact-text, normalized-text, group, and validation-split overlap
  each block before trainer work;
- a duplicate confined to the unused snapshot test split does not block;
- a missing configured group value fails closed;
- retries reuse one check and one authority chain;
- a clean initial snapshot does not authorize a contaminated iteration
  snapshot;
- report threshold changes and contamination overrides cannot authorize
  training;
- missing/extra participants, retired roles, and snapshot/member/bundle/report
  tampering are detected;
- blocked runs have no training runs, checkpoints, or provider work;
- provenance and Doctor verify the complete authority chain.

## Milestone 2 — deterministic benchmark qualification

Before expensive workflow execution, derive immutable readiness evidence from
the exact benchmark populations and acceptance contract:

- support for every overall, label, and canonical slice requirement;
- label and required-slice coverage gaps;
- distribution and source composition summaries;
- threshold feasibility and uncertainty/power evidence;
- semantic-quality and representativeness evidence;
- explicit blocking issues versus advisory warnings.

Qualification may expose aggregate facts about sealed evidence, never row
content or adaptive diagnostics. Deterministic policy owns readiness.

### V1 readiness contract

Qualification operates on the exact cohort union pinned by one executable
`BenchmarkBundle`. It evaluates each cohort independently because suite
acceptance requirements are applied independently to each cohort. The immutable
artifact pins the bundle and suite fingerprints, a versioned policy, a
fingerprint of each exact member population, aggregate cohort summaries,
normalized issues, and one derived `ready` or `blocked` state.

The V1 policy makes the following values explicit:

- minimum overall and per-label support;
- a fixed confidence level and maximum binomial proportion margin of error;
- maximum normalized duplicate rate;
- minimum producer diversity and maximum single-producer concentration;
- maximum label-support imbalance.

The uncertainty floor is the conservative worst-case normal-approximation
sample size `ceil(z^2 * 0.25 / margin^2)`. It applies to overall and explicitly
contracted label/slice proportion decisions. A future policy version may use
Wilson intervals or metric-specific bootstrap simulation, but may not silently
change an existing artifact's calculation.

Every suite label is required to have support even when it is not named by an
individual metric requirement. Every canonical slice requirement is parsed
through `evaluation-core::SliceIdentity` and counted using the same slice
semantics as evaluation. A contracted slice must meet its declared support,
the evaluation protocol's minimum slice support, and the uncertainty floor.

Aggregate evidence records label counts, required-slice counts, observed
dimension/value counts, generated/imported composition, producer
concentration, normalized duplicates, and text-length bounds. Normalization
reuses the generation/contamination rule. The artifact never contains raw
text, source paths, or sealed member identities.

Population summaries do not by themselves establish real-world
representativeness or human semantic correctness. V1 therefore emits explicit
warnings for both limitations. A McNemar requirement also emits a warning that
paired-test power remains conditional on the future baseline/candidate
disagreement rate. Later evidence bindings may close those warnings by pinning
an approved quality audit or an operator-reviewed reference distribution; the
system must not infer either claim from class balance alone.

Blocked qualifications remain inspectable. Only a ready qualification may be
approved and bound to a new workflow definition. Approval is a separate
append-only human decision; recalculating a qualification does not imply
approval.

### Milestone 2 implementation sequence

1. pure qualification policy, summaries, issues, and fingerprints;
2. immutable SQLite artifact and deep population recomputation;
3. scriptable create/show/list/validate CLI;
4. append-only review and a ready-plus-approved workflow binding;
5. preparation preview/create integration, provenance, Doctor, and offline E2E;
6. typed semantic-quality and reference-distribution evidence bindings.

## Milestone 3 — bounded Benchmark Architect

A replaceable advisory component may propose a benchmark blueprint from task
semantics, deployment risks, candidate-source summaries, and user objectives.
It may propose cohort roles, coverage targets, evaluation protocols, metrics,
thresholds, and evidence gaps. It cannot read sealed rows, select itself as an
authority, waive qualification failures, mutate snapshots, or approve a
contract. The exact blueprint becomes executable only after deterministic
validation and explicit human review.

The detailed trust boundary, immutable artifacts, freshness contract, and
CLI-first acceptance are defined in
[Benchmark Architect](benchmark-architect-spec.md). An approved architecture
produces an acquisition handoff, not a benchmark bundle. Ordinary import,
contamination checking, suite construction, deterministic qualification, and
human approval remain mandatory.

## Non-goals

This capability does not add graphical UI, HTTP endpoints, cloud execution,
multiple workers, benchmark generation, automatic relabeling, hidden test-set
reuse, threshold optimization against sealed results, or autonomous approval.
