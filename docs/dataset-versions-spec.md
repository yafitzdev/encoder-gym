# Project dataset versions

This is the dataset implementation of `encoder-workspace-product.md`. It extends
Dataset Management for native encoder schemas without converting retrieval rows
into classification labels. It does not replace the existing split, curation,
contamination, or trainer-input contracts.

## Objects

A project has a base training dataset and named variants. Each dataset has an
immutable identity and a linear sequence of versions. A variant's first version
forks one exact version of another dataset in the same project. Later versions
retain that ancestry. Creating a variant does not edit the base dataset.

A version pins its dataset/project, parent version, ordered source-row references,
row content fingerprints, split assignments, and an explicit change set. Initial
source-row IDs derive from source content identity and source record position;
they do not derive from their changing position in a version. Editing a row
preserves its logical ID and records its new source/content identity. Duplicates
in an imported file remain distinct records rather than being silently deduped.

Changes are additions, removals, and replacements. Missing removal/replacement
targets, repeated operations, cross-project ancestry, duplicate row IDs, stale
parents, empty results, and changes that contradict their resulting membership
are rejected. Reconstruction must reproduce the exact version fingerprint.

## Boundaries

- `dataset-core` owns schema-neutral row references, dataset/version identity,
  change application, ancestry validation, and immutable fingerprints.
- `project-workspace-local` owns managed source copies, paged row inspection,
  project-bound dataset persistence, append-only guards, and atomic updates.
- CLI composes verified source inspection with the core. Source files remain
  immutable and native. Qualification and training compatibility are separate
  from creating a dataset version; no version alone grants training authority.
- Imported Nomos base sources are grouped only from their recorded model training
  manifest. A verified historical logical training snapshot can be linked to a
  dataset version after exact input/membership verification; a label or row count
  alone cannot establish that link.
- Electron invokes typed CLI operations. The collection receives metadata only;
  explicit row/diff requests return bounded pages from permitted training sources.
  Neither activity events nor scientific journal projections contain row payloads.
  Sealed and held-out sources cannot enter these training dataset operations.

## Interaction

Data lists the base dataset and variants. Every entry opens the same viewer:
Rows, Changes, Versions, and Models. Users can create a variant from a selected
version, add imported rows, remove or replace stable row IDs, inspect changes,
and open the model trained on an exact version. New versions never silently
update a model's recorded training input or a reserved optimization run.

## Verification

Pure tests cover deterministic reconstruction, independent branches, replacement
identity, invalid/stale parents, duplicate operations, and tamper detection.
Persistence/CLI tests cover atomicity, append-only history, project fencing,
bounded row reads, replay idempotency, and protected-source rejection. Electron
acceptance follows the full base -> variant -> edit -> diff -> history journey.

## CLI

```text
synth workspace dataset <PROJECT> list
synth workspace dataset <PROJECT> adopt-baseline
synth workspace dataset <PROJECT> adopt-run <OPTIMIZATION_RUN_ID>
synth workspace dataset <PROJECT> create --name "Base dataset" --source <IMPORT_ID>
synth workspace dataset <PROJECT> fork <VERSION_ID> --name "Variant 1"
synth workspace dataset <PROJECT> inspect <VERSION_ID>
synth workspace dataset <PROJECT> rows <VERSION_ID> --offset 0 --limit 25
synth workspace dataset <PROJECT> changes <VERSION_ID> --offset 0 --limit 25
synth workspace dataset <PROJECT> revise --file changes.json
```

`create` accepts several `--source` arguments. `create` and `fork` accept explicit
`--dataset-id` and `--version-id` for retryable callers. Revision requests contain
`versionId`, `datasetId`, `parentId`, `added`, `removed`, and `replaced`. Additions
select an existing training import's `importId` and one-based `record`;
replacements pair a stable row `id` with such a `source`. Removals contain stable
row IDs. Repeating the exact IDs and changes returns the same immutable version.
Saving against an old branch head fails; it never silently overwrites new work.

Every mutating CLI operation records an action UUID with started and terminal
events. Events contain dataset/version references, never native row values.

## Model training links

`project-workspace-core::ModelDatasetLink` owns a separate immutable edge between
an exact model artifact and dataset version. It consumes dataset-core's version
contract; neither core depends on an adapter. The edge records ordered native
input keys, import identities, fingerprints and complete input membership.
Input order is model-specific provenance and may differ from dataset display
order. Editing a dataset or promoting a model never changes the edge.

`ImportedManifest` means recorded final-stage training inputs, not a claim about
all ancestral pretraining. `CompletedTraining` additionally binds the model's
exact original run and native snapshot. A source name or row count alone never
establishes a link.

`MaterializedTraining` binds a trained model to an existing exact version,
including a Quick-test subset, without replacing its stable source-row IDs with
positions in a flattened native file. It pins that version as the native
snapshot and records a digest of ordered row-content fingerprints. The core
checks version membership, order and counts; the local adapter independently
reads the verified native inputs and original managed sources to reproduce
those contents. A digest or matching row count alone is not proof of a rendering.
`adopt_materialized` uses normal import custody for the rendered files but never
creates a replacement dataset/version. An imported baseline need not have known
training history for a candidate to link its independently selected version.
Legacy whole-input adoption and its separate native input order stay unchanged.

`workspace dataset <PROJECT> adopt-baseline` reconstructs the imported Nomos
model's version from its verified checkpoint manifest and already-recorded
training imports. It reuses an exact existing base version or creates one with
project/model-scoped retry identities. An incompatible base is left unchanged.
Repeated calls preserve the same version and link. Interrupted link persistence
reuses the created dataset on retry. The append-only edge lives in migration 7;
metadata reads reproduce its fingerprint, model/version reference, original input
order, manifest checksum and counts, and source custody references. Full
verification additionally reconstructs version ancestry and complete membership,
then rereads the exact native source rows. Row/diff reads and mutations also
reconstruct the version rather than trusting lightweight navigation metadata.
Navigation is not full verification; malformed or foreign metadata still fails
closed. CLI adoption records an action UUID.

`adopt-run` recovers dataset links for already-registered completed Nomos
fine-tunes. Normal `workspace register-run-models` registration performs the
same adoption automatically. The CLI verifies scientific completion; the native
adapter verifies the published checkpoint, candidate configuration, training
manifest, receipt, source fingerprints and ordered input counts without running
Python or reading holdout files. Interpolation outputs without a recorded
training snapshot cannot be given an invented training dataset.

The workspace adapter verifies custody again, reuses matching training imports,
and copies missing training inputs through normal import validation. An exact
existing population reuses its version even when native input order differs.
Otherwise it forks the starting model's exact training version and records the
added/removed rows as a new version. Newer user-edited dataset heads do not
replace that historical starting point. Model-specific input order remains in
the link independently of row display order.

Retry identities are deterministic per project/model. A partial import, fork,
revision or link write can be retried without duplicating the version history.
Completed models remain registered if later dataset adoption fails; adoption
does not retrain, promote, or modify the original scientific journal. The
recovery action's JSON events link the run, model and dataset version without
row payloads.

Paged reads validate the complete source's identity and training partitions but
only compute row-content fingerprints for requested records. Full construction
and verification still reconstruct every member. An unrequested changed row
invalidates the entire immutable source, not just the page containing that row.
