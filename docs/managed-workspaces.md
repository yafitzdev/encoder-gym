# Local-model projects

Gym workspaces are distinct from source repositories and from `synth project`
scientific preparation. Creating a workspace copies a baseline, not a run.

## CLI

Inspect a checkpoint and use its returned fingerprint to confirm the copy:

```powershell
synth --output json workspace inspect-model C:\models\my-encoder
synth workspace create C:\EncoderGym\Projects\my-encoder --name "My encoder" --model C:\models\my-encoder --expected-fingerprint "sha256:…" --task "Tool routing"
synth workspace open C:\EncoderGym\Projects\my-encoder
synth workspace verify C:\EncoderGym\Projects\my-encoder
```

The destination must not exist; its parent must exist. Supported local bundles
contain `config.json`, `tokenizer.json`, and unsharded `model.safetensors` from
the BERT encoder family. Sentence-transformer Transformer, Pooling and Normalize
modules are preserved. This is safe-format custody, not a promise of trainer
compatibility. Custom code, pickle, links/junctions and source repositories are
rejected. No model code is executed and there is no network access.

Import JSONL without translating its native schema:

```powershell
synth --output json workspace inspect-dataset C:\data\examples.jsonl --purpose training
synth workspace import-dataset C:\EncoderGym\Projects\my-encoder --source C:\data\examples.jsonl --name "Training source" --purpose training --expected-fingerprint "sha256:…"
```

Purpose is explicit: `unassigned`, `training`, `development`, or `sealed`.
Training import rejects rows declaring another partition. Without partition
metadata, purpose is the user's declaration, not inferred approval. Imports
are immutable custody records, **not admitted dataset snapshots**. Use normal
slice admission, snapshot, and evaluation contracts before training. Native
tool-routing rows are never flattened into classification labels.

The content hash owns replay identity. Reimporting identical content with the
same purpose/provenance keeps the original name and source. Conflicts fail.
Copies are staged and verified. A copy published before an interrupted DB
commit is reused only when its bytes match. Interrupted project creation does
not publish an openable manifest; its incomplete destination is retained for
inspection rather than overwritten on retry.

## Nomos final-stage backfill

For a copied baseline containing `nomos_training_manifest.json`:

```powershell
synth workspace backfill-nomos C:\EncoderGym\Projects\Nomos --source-root C:\source\fitz-tool
```

Only relative inputs named in that copied manifest are imported. The adapter
checks declared counts and accepted native training partitions, preserves
bytes, and links baseline and manifest fingerprints. It does not import all
ancestor data, historical runs, or sealed evidence. The historical manifest's
path references do not attest the bytes that existed during training; the new
inventory attests the exact files available and copied at import time.

## Portability

Move the whole folder, including manifest, SQLite and artifacts. Original source
paths are provenance only. `open` checks project/database binding, required
files and sizes; `verify` rehashes all model/data files and revalidates counts,
formats, and training provenance. Do not hand-edit or mix project manifests
and databases. Library nicknames are separate from immutable creation metadata.
Workspace commands never initialize the global synthetic-data database.

`project.sqlite` belongs to the workspace custody registry. Existing slices
retain their own stores and migration histories; do not pass this registry as
the legacy CLI's `--database-url`. Slice-owned run stores and artifacts can
live beneath the project's `runs/`, `models/candidates/`, and evaluation/data
directories when explicitly configured through those slice contracts.
