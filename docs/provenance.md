# Immutable provenance

Provenance is derived from persisted foreign keys and cryptographic artifact
identities. It is not reconstructed from CLI output or current filesystem
timestamps.

For transformer evaluation, the dependency path is:

```text
evaluation run
  -> transformer checkpoint (size and SHA-256)
    -> training run (resolved common and transformer configuration)
      -> registered base model (bundle SHA-256 identity)
      -> immutable source snapshot
      -> optional parent checkpoint (explicit continuation only)
```

The checkpoint manifest additionally records the tokenizer SHA-256 identity,
exact label order, BERT configuration, snapshot ID, epoch, optimizer step, and
normalized training settings. Evaluation records retain the selected split and
resolved configuration. Analysis consumes persisted evaluation facts without
rerunning the predictor. A comparison-aware analysis has both its analyzed
evaluation and paired comparison as parents; the comparison points to both
evaluation runs. Finding evidence records prediction, snapshot-member, and
source-row IDs, so full text and probabilities resolve from immutable facts.
Append-only reviews reference the immutable report/finding pair and may name a
later evaluation or comparison that resolved it.

Decision-grade optimization extends the chain without changing upstream
artifacts:

```text
outcome assessment
  -> campaign
    -> chosen proposal
      -> approval review
      -> analysis report + optional comparison
      -> dataset + snapshot-bound coverage/evidence
    -> append-only links to applied plan/job
    -> candidate snapshot/training/checkpoint/evaluation/comparison/analysis
```

Proposals embed normalized recommendations and the complete bounded evidence
needed to reproduce scores and allocation. Recommendation IDs are SHA-256
content identities, so inspect them with `optimize explain` and trace the UUID
of their containing proposal. A plan created by approved application links
back to that proposal. Scenario proposals become independently traceable after
explicit `scenario-materialize`.

The governed workflow adds a fingerprint-linked orchestration chain without
copying slice artifacts:

```text
model promotion or rejection
  -> sealed acceptance assessment -> sealed evaluation -> sealed suite/cohort
  -> selected checkpoint -> training run -> immutable candidate snapshot
  -> development acceptance and stop decision
  -> paired comparison and follow-up analysis
  -> approved proposal/application -> generation plan/job -> added rows
  -> workflow run -> immutable resolved workflow definition
```

A declarative preparation is itself immutable and points to the ordinary
workflow definition it assembled. The definition retains the project,
dataset, suite, cohort, and snapshot identities; the manifest fingerprint is
the idempotency key.

Workflow attempts retain iteration, predecessor, usage-after, approval,
advisor, dataset-diff, and slice-artifact links. Exposure records independently
show which development evidence was consumed for diagnosis, advising,
optimization, and comparison, and which sealed aggregate was disclosed for
acceptance.

Inspect a chain with:

```text
synth provenance checkpoint <CHECKPOINT_ID>
synth --output json provenance checkpoint <CHECKPOINT_ID>
synth provenance analysis-report <ANALYSIS_REPORT_ID>
synth provenance analysis-finding-review <REVIEW_ID>
synth provenance optimization-proposal <PROPOSAL_ID>
synth provenance optimization-proposal-review <REVIEW_ID>
synth provenance optimization-campaign <CAMPAIGN_ID>
synth provenance optimization-campaign-link <LINK_ID>
synth provenance optimization-outcome <OUTCOME_ID>
synth provenance workflow-definition <DEFINITION_ID>
synth provenance project-preparation <PREPARATION_ID>
synth provenance workflow-run <RUN_ID>
synth provenance acceptance-assessment <ASSESSMENT_ID>
synth provenance advisory-assessment <ASSESSMENT_ID>
synth provenance workflow-approval <APPROVAL_ID>
synth provenance stop-decision <DECISION_ID>
synth provenance model-promotion <PROMOTION_ID>
```

A changed local base-model file fails encoder verification; it does not change
the historical identity. Continuing a transformer checkpoint creates a new run
with `parent_checkpoint_id` rather than rewriting the original run or artifact.
