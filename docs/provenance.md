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
```

A changed local base-model file fails encoder verification; it does not change
the historical identity. Continuing a transformer checkpoint creates a new run
with `parent_checkpoint_id` rather than rewriting the original run or artifact.
