# Managed encoder workspaces

## Goal and scope

Encoder Gym owns a portable project folder, starting from a local checkpoint.
New project and Open project are distinct operations. A source repository is
an import source, not a project. Hugging Face, downloads, automatic training,
automatic evaluation, and historical-run migration are deferred.

The existing slice contracts remain authoritative for dataset admission,
snapshots, training, evaluation, and controlled execution. Workspace imports
provide artifact custody and provenance; they do not manufacture slice runs,
evaluation evidence, class labels, or approved training membership.

## Ownership and layout (version 1)

- `encoder-gym.json`: immutable project identity, name, creation time, task
  description, layout version, and baseline inventory with source identity.
- `project.sqlite`: project-bound imported dataset metadata.
- `models/baseline/`: independent byte-for-byte local checkpoint copy.
- `datasets/imports/<content-sha256>/data.jsonl`: immutable source data copies.
- `datasets/snapshots/`, `models/candidates/`, `runs/`, `evaluations/`: owned
  artifact locations for the respective existing slice contracts.

All working paths are relative to the managed root. Original source paths are
provenance only and are never required to reopen or verify a project. Project
identity must survive moving the root and reopening in another app profile.

## Core and local adapter

Pure manifest, inventory, and dataset provenance contracts belong to
`project-workspace-core`. Filesystem copying, source inspection, hashing, and
SQLite persistence belong to `project-workspace-local`. Expose these through
`synth workspace` before connecting Electron to the same implementation.

Creation requires an absent destination, validates a local safetensors encoder
bundle, and publishes only after the baseline copy and database are complete.
No pickle, custom model code, symlink/reparse traversal, network, or shell
execution is permitted. Preserve sentence-transformer pooling/normalization
configuration and native model metadata. Custody validation does not imply
compatibility with every existing trainer. Unknown model execution remains
unavailable rather than silently selecting a trainer.

Dataset import validates newline-delimited JSON objects without converting the
native schema. Record bytes, row count, checksum, declared purpose, source,
timestamp, and optional baseline training-manifest reference. Reimporting the
same content is idempotent; conflicting purpose/provenance is not silently
overwritten. Explicitly reject held-out/sealed rows in training imports.
Import staging and SQLite transactions must leave interrupted or duplicate
operations recoverable without changing existing artifacts.

Opening and verification validate the version, project/database binding,
contained paths, and persisted inventories. Verification rehashes artifacts.
Neither operation may initialize an arbitrary folder or mutate a source.

## Desktop experience

New project collects name, checkpoint, destination, and optional task. Show
the inspected model and copy size before confirmation. Report ongoing work
and actionable validation failures. Open project accepts a managed workspace;
existing external journal connections remain explicitly legacy/read-only.

Project home shows the real baseline immediately, with no fabricated runs,
candidates, or scores. Datasets has working local import and provenance views.
Runs and evaluations explain their empty state. Setup distinguishes imported
data from admitted snapshots and an evaluation contract. Preserve persistent
project folders, project isolation, missing/moved recovery, and forget-without-
delete behavior. No Hugging Face placeholder controls.

## Nomos onboarding

Create a separate managed Nomos workspace from the retained local
`nomos_bge_contrast_replay_ablation` checkpoint in the user-designated
`fitz-tool` source. Backfill its two final-stage inputs named by the checkpoint's
`nomos_training_manifest.json`, preserving native rows and original bytes.
Verify manifest-declared counts and training partition. Record this as the
final-stage training data, not a reconstruction of all ancestor pretraining.
Do not attach old experiment runs to the new identity or consume sealed data.
Replace the existing Nomos source-folder connection with the managed project
without deleting or modifying anything in `fitz-tool`.

## Completion evidence

1. Deterministic core/local/CLI tests cover creation, safe copy, corruption,
   unsupported inputs, portable reopen, duplicate imports, provenance, and
   training/held-out isolation. All required Rust gates pass.
2. UI unit/build checks and actual Electron acceptance cover New/Open, import,
   two-project isolation, restart, moved-folder recovery, and invalid inputs.
3. The real managed Nomos baseline and dataset copies verify against source
   hashes and counts. Its actual app entry opens the managed project. Source
   Git status and source artifact hashes remain unchanged.
4. Updated documentation explains local onboarding and imported-data limits.
   Changes are reviewed and committed in coherent working stages.
