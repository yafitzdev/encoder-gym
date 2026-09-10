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
