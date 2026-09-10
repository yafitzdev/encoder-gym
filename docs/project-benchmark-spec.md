# Project evaluation benchmark

Implements the Evaluation page contract in `encoder-workspace-product.md`.
A project has one benchmark with immutable versions, not a separate benchmark
for each baseline or candidate. This catalog is not execution, qualification,
freshness, exposure or promotion authority; the existing owning contracts remain
mandatory for those operations.

## Identity and boundaries

`encoder-experiment-core` owns the benchmark definition for external encoder
experiments. It pins task, backend identity, evaluator source revision, the
adapter-normalized evaluation-configuration fingerprint, the existing metric
contract, and each suite's role, complete evaluation fingerprint and support.
It contains no model identity, baseline scores, training settings or run budget.
The suite fingerprint must bind both test membership and evaluation procedure.
The native adapter owns normalization, including agent model/configuration; the
renderer must not infer equivalence from matching labels, top-k or filenames.

Changing data, roles, metric rules, evaluator code/configuration or support
changes the definition. Changing the baseline, candidate, training recipe or
run ID does not. Development results are comparable only against their exact
definition. Sealed reports, scores, rows and diagnostics never enter the
comparison projection. Final-test exposure stays in the existing scientific
ledger and cannot be reset by cataloging a version.

`project-workspace-core` owns the project-local numbered version and its exact
predecessor, plus the scientific binding/project/protocol that established it.
It references the experiment-owned definition rather than implementing another
metric or acceptance policy. The workspace adapter persists append-only versions
and verifies their complete chain and source bindings. Identical definitions
reuse an existing version; stale-parent writes fail, and retries reuse the same
IDs. Moving a managed folder must preserve these identities.

The CLI composes native normalization and verified scientific-store reads with
the project catalog. Adopting a recorded definition does not run evaluation,
import arbitrary historical models, or grant approval to use protected data.
Every mutation records a UUID activity action with safe version/protocol refs.

## Interface

Evaluation has one version selector and one results table for that version.
The table includes the baseline and all project models, including rejected
outputs. Missing matching results say Not evaluated; missing values are not zero.
Repeated evaluations retain their report/run references rather than silently
selecting the best score. Protocol and version history are secondary tabs.
Model links open the same model viewer used by Models. No setup cards, candidate
subcatalog, repair headings, or benchmark count badges are required.

Results must resolve all project-owned scientific bindings, including historical
bindings after a baseline change, and verify each report against the selected
definition. A native model maps to a managed model through its exact source
identity and producing run; the recorded baseline uses its explicit scientific
binding's baseline revision. Names and directory similarity are not evidence.
Historical checkpoints that were never registered remain run evidence, not new
rows in the managed model inventory. Original acceptance verdicts retain their
original baseline/run context; a comparison against today's baseline must not
silently relabel an old verdict as a new acceptance decision.

New benchmark versions must come from explicit test-data/protocol configuration
or verified existing authority, never inferred from successful training. The
Optimize request pins the selected benchmark version alongside model and dataset.
Initial test-data onboarding and automatic execution are required follow-on
integration, not implied by a historical-results catalog.

## CLI

```text
synth workspace benchmark <PROJECT> list
synth workspace benchmark <PROJECT> preview-run <EXPERIMENT_RUN_ID>
synth workspace benchmark <PROJECT> adopt-run <EXPERIMENT_RUN_ID> [--expected-parent <VERSION_ID>]
synth workspace benchmark <PROJECT> inspect <VERSION_ID>
synth workspace benchmark <PROJECT> results <VERSION_ID>
```

Preview is read-only and returns a stable proposed version ID, current parent,
and any existing matching version. Adoption pins a verified recorded experiment
from the project's current scientific binding. A changed definition requires
the exact current parent; an identical definition reuses its original version
and original provenance even if another run uses it. Inspection reopens the
version's original binding and reproduces its definition from the verified
scientific protocol; it neither needs Python nor opens native test files.
Cataloging a benchmark never creates a fresh generation or consumes a holdout.

Results starts from every registered model and resolves development reports from
all compatible project-owned scientific bindings. Reused report identities have
one result with multiple run contexts; distinct evaluations remain distinct even
if their metrics coincide. Empty report lists mean Not evaluated. Each original
development assessment retains its exact baseline revision and producing run.
The projection validates registered outputs against their training receipt (or
the terminal head used by older accepted imports), not just matching file hashes.

Suite support is the normalized evaluation report's support, not necessarily
the number of source rows or the support of every individual metric. Surfaces
must not label this value as a test-set row count.

## Verification

- Prove model/training/run changes preserve benchmark identity.
- Prove changed membership, metric gates, roles, evaluator settings/code and
  support cannot compare as the same version.
- Reject sealed or mismatched reports and altered definition fingerprints.
- Verify immutable persistence, exact source/project binding, moved folders,
  stale-parent rejection and idempotent retry.
- Exercise CLI adoption and history through real process boundaries with
  deterministic fixtures; verify safe activity references and no execution.
- Exercise the actual desktop version selector/results/model links, including
  missing and incompatible results and restart.
- Keep existing Nomos evidence intact; real inspection is read-only.
