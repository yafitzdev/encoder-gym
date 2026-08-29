# Slice 6 product specification — Optimization

## Objective

Convert persisted evaluation and error-analysis evidence into explicit,
reviewable recommendations for the next data-generation or training run.

The application must let a user:

1. Create an optimization proposal from one analysis report.
2. Configure a finite additional-example budget and minimum support.
3. Allocate that budget deterministically toward weak generation cells.
4. Inspect every recommendation with score, evidence, and proposed count.
5. Persist and export the immutable proposal before applying it.
6. Apply approved data recommendations by creating a normal unequal Slice 1
   generation plan; never bypass the existing planner or mutate old plans.
7. Optionally recommend bounded training-configuration changes using explicit
   deterministic rules.

## Safety and boundaries

Optimization is advisory and finite. It does not call an LLM, spend money,
start generation or training automatically, rewrite historical artifacts, or
run an autonomous loop. Applying a proposal is an explicit user action through
the same ports used by ordinary plan creation.

The controlled-workflow phase preserves this boundary. A separate workflow
orchestrator may create Optimization inputs, request a normal proposal, verify a
compatible explicit approval or finite pre-authorization envelope, apply the
proposal through the existing idempotent contract, and then invoke ordinary
slice runners. Optimization itself remains deterministic and provider-free.
Only development/diagnostic evidence is eligible; sealed-acceptance evidence is
rejected before Optimization receives a diagnostic contract.

## Decision evidence and scoring

Slice 6.1 consumes only the analysis-owned diagnostic contract. Cell evidence
includes support, errors, rate, baseline lift, error share, high-confidence
severity, unique marginal coverage, optional paired-comparison counts, latest
review disposition, and reproducible fingerprints.

The resolved optimization protocol selects one explicit deterministic scoring
policy: error count, error rate, positive error-rate lift, high-confidence
severity, marginal error coverage, comparison regression, or a conservative
composite. The conservative composite combines Wilson- or shrinkage-adjusted
error rate, positive lift, severity, unique coverage, error share, and a
support factor. It is a prioritization heuristic, not a root-cause or expected
improvement claim. Every scored cell retains the raw evidence, adjustment,
penalties, boosts, eligibility reasons, and final rounded score.

The pure integer allocator applies canonical cell and label lower/upper bounds,
per-label share bounds, a global per-cell concentration cap, and the minimum
useful allocation increment. It uses deterministic proportional base shares
and stable largest remainders, redistributing around saturated cells without
violating upper bounds. A preview always accounts for the entire finite budget
as allocated or explicitly unallocated and returns structured constraint
issues when exact conservation is impossible.

Default sensitivity analysis compares four bounded alternatives over the same
immutable evidence: conservative support-adjusted scoring, raw error volume,
high-confidence or paired-regression priority, and a balanced per-label cap.
Each scenario has its own protocol and proposal fingerprints. Pairwise output
reports allocated-cell overlap and every cell whose integer allocation changes,
while concentration summaries expose totals by label and dimension value.

Proposal reviews are append-only children of immutable proposals. Full and
partial plan approvals validate the exact recommendation IDs; rejection,
supersession, training-candidate acceptance, and completion awaiting assessment
remain separate states. Decision-grade application requires a compatible
approval and reproduces proposal, protocol, dataset, and accepted-coverage
identity before creating an ordinary unequal plan. Changed coverage is reported
as stale rather than silently changing absolute targets. The legacy apply path
remains explicit for historical proposals.

An explicit rebase creates a new immutable proposal from the same analysis,
dataset, snapshot, protocol, and comparison identities using freshly observed
accepted-row coverage. The new proposal fingerprints its predecessor and both
coverage identities; neither proposal is rewritten.

Optional training-configuration recommendations enumerate a user-supplied
finite typed space from one completed baseline run, its final checkpoint,
immutable snapshot, and supported backend identity. Candidates retain exact
field differences, rationale, cautions, and fingerprints. They are experiment
suggestions only and never start a run. Review-only recommendations surface
existing diagnostic cells for label/schema attention without changing data.

Human-governed campaigns bind a chosen proposal and compatible approval to
append-only links for an applied plan, generation jobs, snapshot, training
runs/checkpoints, candidate evaluation, paired comparison, and follow-up
analysis. Every link validates the expected artifact lineage. A deterministic
outcome assessment summarizes overall/per-label deltas, persisted uncertainty,
fixed/regressed/persistent examples, and target coverage realization under a
fingerprinted policy. Its classification is observational and makes no causal
claim.

## Test priorities

- exact budget conservation
- deterministic ranking and tie handling
- no allocation to unknown cells
- correct conversion into unequal absolute generation targets
- immutable evidence and proposal persistence
- training candidate backend/configuration validation
- append-only reviews, stale rejection, rebase, and idempotent application
- scenario sensitivity and campaign lineage compatibility
- deterministic, reproducible outcome assessment

## Completion criterion

Slice 6 is complete when the CLI can preview and persist a protocol-bound
proposal, compare deterministic scenarios, inspect every normalized
recommendation, record an append-only human decision, and create a valid
unequal Slice 1 plan only from a compatible non-stale approval. It must also
expose bounded training and review-only candidates without acting on them,
record a compatible manual experiment campaign, assess observed outcomes,
export rows, reproduce provenance, and pass doctor. API and graphical UI work
remain deferred.
