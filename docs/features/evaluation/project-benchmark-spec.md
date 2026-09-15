# Project evaluation benchmark

Implements the Evaluation page contract in
`../workspaces/encoder-workspace-product.md`.
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
For a connected task runtime with no recorded protocol, the adapter may compile
its checked-in suite manifest and metric defaults into version 1. That explicit
action evaluates the current baseline once; it is not inferred from a training
run. Importing a new evaluator/test collection remains a separate onboarding
boundary.

## CLI

```text
synth workspace benchmark <PROJECT> list
synth workspace benchmark <PROJECT> initialize [--expected-parent <VERSION_ID>]
synth workspace benchmark <PROJECT> preview-run <EXPERIMENT_RUN_ID>
synth workspace benchmark <PROJECT> adopt-run <EXPERIMENT_RUN_ID> [--expected-parent <VERSION_ID>] [--expected-definition <FINGERPRINT>]
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
Previewing or adopting a recorded benchmark never creates a fresh generation or
consumes a holdout. `initialize` is the explicit exception: it uses the bound
adapter's named development suites and one final suite, evaluates the current
baseline, persists the resulting protocol, and then records version 1. It never
trains or contacts a provider. Its action UUID and safe protocol/version
references enter project Activity; final scores and test contents do not.
Stable protocol and version IDs make retry idempotent. Once the protocol exists,
retry recovers it without rerunning native evaluation.

The desktop's Choose recorded benchmark dialog previews the selected run before
saving. It always supplies the reviewed definition fingerprint and exact parent;
a changed definition is rejected before a version is recorded. Choosing an
already-cataloged definition opens that version without another mutation. A lost
save response can retry the exact request without adding a duplicate version.
The recorded-run dialog is onboarding from historical authority. When a project
has a connected runtime but no benchmark history, its first Optimize action
initializes the project benchmark and then continues the same click. Importing
an entirely new test-data/protocol definition is not implemented by either path.

Results starts from every registered model and resolves development reports from
all compatible project-owned scientific bindings. Reused report identities have
one result with multiple run contexts; distinct evaluations remain distinct even
if their metrics coincide. Empty report lists mean Not evaluated. Each original
development assessment retains its exact baseline revision and producing run.
The projection validates registered outputs against their training receipt (or
the terminal head used by older accepted imports), not just matching file hashes.

Input-first optimization may materialize a new scientific project for the selected
dataset. Results also follow each project run's exact preparation, materialization
and attached experiment receipts into that derived project. They verify the pinned
setup, launch, benchmark, original runtime/baseline, protocol, candidate and journal
head before projecting its development reports. They never discover unrelated
projects merely because they share a store, model name or benchmark definition.
The result context identifies the derived scientific project while retaining the
original runtime binding and baseline revision. This is a read-only association:
historical reports and journals are not rewritten or reevaluated.
Materialization may assign a new scientific UUID to the same baseline bytes;
use the owning model artifact's content-equivalence contract (format, byte count
and fingerprint), not UUID equality. The managed baseline revision stays pinned.

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
- Exercise preview/adoption through the real production CLI in an isolated
  offline fixture; reject a definition changed since preview.
- Exercise initialization through the real production CLI and native adapter
  boundary; prove one baseline evaluation, no run/training/provider operation,
  safe activity output, final-value isolation, and evaluation-free retry.
- Keep existing Nomos evidence intact; real inspection is read-only.
