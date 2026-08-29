# Trustworthy optimization

Slice 6.1 is a local advisory workflow. It turns one immutable Slice 5 analysis
report into finite, reproducible recommendations. It never starts generation,
snapshot creation, training, evaluation, or analysis. Those actions remain
separate normal CLI commands chosen by a human.

When composed by the governed encoder workflow, proposal creation records an
`optimization` evidence exposure and still crosses an explicit approval or
finite preauthorization boundary before generating a dataset diff. The
optional LLM advisor may interpret development evidence but cannot allocate
rows, apply the proposal, start training, or consume sealed evidence.

## Decision protocol

Start from `examples/optimization.toml`. The file is strict: unknown fields,
contradictory filters, invalid bounds, unsupported recommendation requests, and
non-finite values are rejected. `optimize preview` resolves and validates the
same artifact that `optimize propose` persists:

```text
synth optimize preview <ANALYSIS_REPORT_ID> --protocol examples/optimization.toml
synth optimize propose <ANALYSIS_REPORT_ID> --protocol examples/optimization.toml
```

The immutable proposal embeds the complete normalized protocol, its
fingerprint, the bounded decision evidence, source analysis/evaluation/
comparison identity, dataset definition, snapshot identity, and accepted-row
coverage. Reproduction does not depend on later config defaults.

The current protocol always includes `data_generation` because it owns a
finite additional-example budget. It may additionally include
`training_configuration` with a bounded candidate request and `review_only`.

## Evidence and scores

Raw evidence remains distinct from its prioritization score. Each cell retains
support, errors, error rate, baseline lift, error share, high-confidence error
severity, unique marginal coverage, optional paired-comparison evidence, and
the latest review disposition. Supported scoring policies are error count,
error rate, positive error-rate lift, high-confidence error severity, marginal
error coverage, paired-comparison regression, and a conservative composite.

Risk adjustment can be disabled, use a Wilson lower bound, or shrink rates
toward the baseline. This reduces the tendency to rank tiny, noisy cells above
well-supported problems. It does not prove root cause or predict improvement.
`optimize explain` shows raw evidence, transformations, eligibility, score,
constraints, deterministic rationale, and cautions for one recommendation.

## Allocation and alternatives

The pure integer allocator honors cell and label bounds, label budget shares,
a global per-cell concentration cap, exclusions, and the minimum useful
increment. A feasible proposal conserves the exact budget. An infeasible
proposal either fails or records an explicit unallocated remainder and
structured reasons, according to the protocol.

`additional_count` is the proposed increment. `proposed_target` is the
absolute accepted-row target consumed by Slice 1. Application never treats the
increment as an absolute target.

Compare four deterministic interpretations of the same evidence without
running an experiment:

```text
synth optimize scenarios <ANALYSIS_REPORT_ID> --protocol examples/optimization.toml
synth optimize scenario-show <SCENARIO_GROUP_ID>
synth optimize scenario-materialize <SCENARIO_GROUP_ID> <SCENARIO_ID>
```

The group reports allocation overlap, policy-sensitive cells, and
concentration by label and dimension. Materialization persists a chosen
scenario as an ordinary immutable proposal so it can enter the same review
workflow.

## Recommendations and bounded training experiments

Recommendations are normalized into independent data-generation, training,
and review-only sections. They can be filtered and paged without changing the
proposal:

```text
synth optimize recommendations <PROPOSAL_ID> --kind data-generation \
  --label billing --dimension difficulty=hard --eligible --limit 50
synth optimize explain <PROPOSAL_ID> <RECOMMENDATION_ID>
```

Training candidates start from an exact completed run, its final checkpoint,
the immutable source snapshot, and the registered backend identity. Create a
finite configuration-space document from a JSON or TOML file containing a
top-level `choices` array:

```text
synth optimize training-space <TRAINING_RUN_ID> <FINAL_CHECKPOINT_ID> \
  --choices training-choices.toml --file training-space.json
synth optimize propose <ANALYSIS_REPORT_ID> --protocol optimization.toml \
  --training-space training-space.json
synth optimize training-candidates <PROPOSAL_ID>
```

Every candidate states exactly which typed settings differ from the baseline.
Only hashing-linear and registered `bert-cpu` configurations accepted by the
selected backend are supported. Candidate enumeration is capped and never
creates a training run. Review-only recommendations likewise never mutate a
label, schema, dimension, row, or snapshot.

## Review, stale evidence, and safe application

Reviews are append-only children of a proposal. Approval and partial approval
carry exact recommendation IDs; rejection and supersession do not change the
proposal:

```text
synth optimize review <PROPOSAL_ID> --state approved-for-plan-creation
synth optimize review <PROPOSAL_ID> --state partially-accepted \
  --select <RECOMMENDATION_ID> --select <RECOMMENDATION_ID>
synth optimize apply <PROPOSAL_ID> --approval-id <REVIEW_ID>
```

Application reproduces the protocol, evidence, allocation, dataset cells,
approval, and current coverage before atomically creating a normal unequal
generation plan. It is idempotent and does not start a job. If accepted
coverage changed, application rejects the stale proposal. Refresh explicitly:

```text
synth optimize rebase <STALE_PROPOSAL_ID>
```

Rebase creates a new proposal with predecessor and old/new coverage
fingerprints. The stale proposal remains unchanged. `legacy-apply` exists only
for historical pre-protocol proposals and is intentionally named as a
compatibility path.

## Human-governed campaigns

A campaign is an immutable decision header plus append-only links to artifacts
created through their normal commands:

```text
synth campaign create <PROPOSAL_ID> --approval-id <REVIEW_ID>
synth campaign link <CAMPAIGN_ID> --generation-plan-id <PLAN_ID>
synth campaign link <CAMPAIGN_ID> --generation-job-id <JOB_ID>
synth campaign link <CAMPAIGN_ID> --snapshot-id <SNAPSHOT_ID>
synth campaign link <CAMPAIGN_ID> --training-run-id <RUN_ID>
synth campaign link <CAMPAIGN_ID> --checkpoint-id <CHECKPOINT_ID>
synth campaign link <CAMPAIGN_ID> --evaluation-run-id <EVALUATION_ID>
synth campaign link <CAMPAIGN_ID> --comparison-id <COMPARISON_ID>
synth campaign link <CAMPAIGN_ID> --analysis-report-id <REPORT_ID>
synth campaign assess <CAMPAIGN_ID> --comparison-id <COMPARISON_ID>
synth campaign show <CAMPAIGN_ID>
```

Each link validates dataset, snapshot, run, checkpoint, cohort, protocol, and
comparison compatibility as applicable. Assessment records overall and
per-label deltas, persisted confidence/significance evidence, fixed/regressed/
persistent errors, target-cell coverage realization, and an explicit
improved/regressed/mixed/inconclusive classification under a fingerprinted
policy. This is an observed experiment lineage, not a causal attribution.

## Export, provenance, and recovery

```text
synth optimize export-summary <PROPOSAL_ID> --format csv --file summary.csv
synth optimize export-recommendations <PROPOSAL_ID> --format jsonl --file recommendations.jsonl
synth optimize training-candidates <PROPOSAL_ID> --format csv --file candidates.csv
synth optimize export <PROPOSAL_ID> --file proposal.json
synth provenance optimization-proposal <PROPOSAL_ID>
synth provenance optimization-proposal-review <REVIEW_ID>
synth provenance optimization-campaign <CAMPAIGN_ID>
synth provenance optimization-outcome <OUTCOME_ID>
synth doctor
```

Recommendation IDs are content fingerprints rather than UUID provenance root
IDs; use `optimize explain` and trace their containing proposal. Applied plans
trace back to the proposal. Optimization writes are transactional or
append-only. Recovery never auto-applies, resumes, or schedules an optimization
decision; safely rerun the read/creation command after an interrupted process.
