# Immutable provenance

Provenance is derived from persisted foreign keys and cryptographic artifact
identities. It is not reconstructed from CLI output or current filesystem
timestamps.

Synthetic generation includes the semantic interpretation used at request
time:

```text
generation job
  -> immutable generation execution specification
    -> ordered provider request attempts and outcome fingerprints
  -> generation semantic context (job-pinned assignment)
    -> append-only binding decision
      -> exact semantic profile version -> predecessor profile version
      -> predecessor binding decision
      -> dataset definition
  -> generation plan
```

An initial plan created from an exact dataset-size request has an additional
decision path:

```text
generation plan
  -> immutable initial allocation (policy, coverage, compiled bounds, targets)
    -> dataset definition
```

The allocation and its result carry reproducible SHA-256 fingerprints. Compact
distribution explanations are derived from that persisted result rather than
stored as mutable presentation state.

Qualified snapshots add a separate, reviewable evidence chain:

```text
qualified snapshot
  -> curation application
    -> approved manifest + exact manifest approval
      -> complete curation proposal + append-only row reviews
        -> quality report
          -> normalized assessments + evaluator attempts
            -> audit run + complete source-set plan
              -> pinned semantic/authenticity guidance
              -> generated or imported source rows
```

The audit stores the exact bounded guidance payload resolved at creation.
Later semantic or authenticity binding changes cannot alter or prevent replay
of the historical audit. Provenance redacts candidate text, local import paths,
provider-controlled errors and metadata, rationales, and human review reasons
while retaining fingerprints, budgets, decisions, and parent identities.

Generated rows also embed the resolved context in generation metadata. A
workflow advisory assessment links to the same generation semantic context,
so its prompt cannot silently pick up a later catalog revision.

The execution specification fingerprint covers initial per-cell needs,
backend/model and non-secret endpoint identity, parameters, bounded execution
policy, prompt-template identity, and semantic-context fingerprint. Each row
links to its provider attempt in generation metadata. Usage and backend outcome
metadata therefore remain inspectable even when a successful response yields no
accepted rows.

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
workflow definition it assembled. New definitions and preparations both pin the
same immutable benchmark bundle:

```text
project preparation or workflow run
  -> workflow definition
    -> benchmark bundle
      -> development benchmark suite
      -> optional sealed benchmark suite
      -> strict global contamination report
```

The bundle records both the identity and fingerprint of each parent. Provenance
traversal fails closed if a suite or report is absent or its fingerprint has
changed. Suite and report nodes remain bounded leaves; their immutable payloads
carry the cohort, role-decision, protocol, and contamination evidence pins. The
definition also retains the project and dataset identities, while the manifest
fingerprint remains the preparation idempotency key. Legacy definitions and
preparations without a bundle link remain readable for historical inspection,
but a legacy definition or run cannot be used to start or advance workflow
execution.

Workflow attempts retain iteration, predecessor, usage-after, approval,
advisor, dataset-diff, and slice-artifact links. Exposure records independently
show which development evidence was consumed for diagnosis, advising,
optimization, and comparison, and which sealed aggregate was disclosed for
acceptance.

The production encoder path has an independent row-free chain:

```text
production optimization run
  -> immutable optimization definition
    -> approved repair proposal and native-delta selection
    -> logical combined-training snapshot
    -> renewable benchmark generation
    -> metric-source protocol and finite candidate set
  -> reserved production campaign
    -> exact experiment protocol and run
      -> checkpoint and per-development-suite reports/assessments
      -> optional one-candidate sealed authorization/report
    -> deterministic final decision
```

Inspect it with `synth encoder optimize provenance <RUN_ID>`. The command emits
the normalized bundle and its SHA-256 fingerprint without native rows. `doctor`
adds checkout, file, native-delta, snapshot, journal, and sealed-exposure replay.
The proven Nomos bundle fingerprint is recorded in
`features/optimization/encoder-optimize.md`.

Inspect a chain with:

```text
synth provenance checkpoint <CHECKPOINT_ID>
synth provenance semantic-profile <PROFILE_ID>
synth provenance semantic-binding <BINDING_ID>
synth provenance generation-semantic-context <JOB_ID>
synth provenance initial-allocation <ALLOCATION_ID>
synth --output json provenance checkpoint <CHECKPOINT_ID>
synth provenance analysis-report <ANALYSIS_REPORT_ID>
synth provenance analysis-finding-review <REVIEW_ID>
synth provenance optimization-proposal <PROPOSAL_ID>
synth provenance optimization-proposal-review <REVIEW_ID>
synth provenance optimization-campaign <CAMPAIGN_ID>
synth provenance optimization-campaign-link <LINK_ID>
synth provenance optimization-outcome <OUTCOME_ID>
synth provenance benchmark-suite <SUITE_ID>
synth provenance contamination-report <REPORT_ID>
synth provenance benchmark-bundle <BUNDLE_ID>
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
