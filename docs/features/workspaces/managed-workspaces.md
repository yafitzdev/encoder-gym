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
synth workspace upgrade C:\EncoderGym\Projects\my-encoder
synth workspace readiness C:\EncoderGym\Projects\my-encoder
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

Workspaces created before the managed model catalog was introduced continue to
open without mutation and report no catalog. `workspace upgrade` applies the
versioned registry migrations and records the already-verified imported model
as the first immutable model artifact and active baseline revision. It is
idempotent and does not edit `encoder-gym.json`, copy artifacts, or initialize a
scientific run database.

## Project activity

`project.sqlite` contains a project-wide, append-only activity journal for
meaningful desktop commands and explicitly recorded CLI integrations. Each action has one UUID and a
hash-chained event sequence: `started`, optional bounded `progress`, then
`succeeded` or `failed`. Events have their own UUIDs and may carry only stable,
row-free artifact references. Provider keys, authorization headers, sealed
rows, prompts, dataset records, and raw model content are forbidden from this
journal. Provider failures are redacted before recording.

The desktop Activity page reads verified actions, exposes both action and event
UUIDs, and exports the complete verified stream as JSON Lines. Navigation,
automatic status polling, clipboard actions, and window controls are not
project commands and are deliberately excluded. Slice-owned scientific
journals remain authoritative for training, evaluation, gate, and decision
facts; the project activity action links to their run or artifact identities
instead of duplicating their evidence.

```powershell
synth workspace activity C:\EncoderGym\Projects\Nomos init
synth workspace activity C:\EncoderGym\Projects\Nomos list --limit 100
synth workspace activity C:\EncoderGym\Projects\Nomos show <ACTION_UUID>
synth workspace activity C:\EncoderGym\Projects\Nomos export --destination C:\exports\nomos-activity.jsonl
```

`init` verifies the manifest/database binding and applies missing registry
migrations without rehashing project artifacts. Desktop access runs it
idempotently so older managed projects gain the journal before the first newly
tracked action. It does not reconstruct past clicks; historical scientific
work remains available through its existing immutable journals.

The compiled Nomos adapter can be explicitly bound to a managed project after
the project is upgraded:

```powershell
synth workspace preview-nomos-binding C:\EncoderGym\Projects\Nomos --runtime C:\isolated\nomos-runtime --python C:\path\to\python.exe
synth workspace bind-nomos C:\EncoderGym\Projects\Nomos --runtime C:\isolated\nomos-runtime
```

To continue from a prior Encoder Gym scientific database, select it explicitly
during the same preview and bind. This is an import, not an inferred nearby
database:

```powershell
synth workspace preview-nomos-binding C:\EncoderGym\Projects\Nomos --runtime C:\isolated\nomos-runtime --python C:\path\to\python.exe --history-database C:\previous\encoder-gym-repair.sqlite
synth workspace bind-nomos C:\EncoderGym\Projects\Nomos --runtime C:\isolated\nomos-runtime --python C:\path\to\python.exe --history-database C:\previous\encoder-gym-repair.sqlite
```

The preview deeply verifies the managed checkpoint and the clean, no-remote
isolated runtime, proves that its baseline matches the active model, and runs a
15-second offline Python capability check. The check requires Python 3.11 or
3.12 plus the training, retrieval-evaluation, local-agent-evaluation, and
diagnostic modules used by the compiled Nomos adapter. It writes nothing and
makes no network or provider call.

If the selected interpreter is compatible but incomplete, the desktop can
explicitly install only the compiled adapter's fixed missing package mapping.
The equivalent CLI boundary requires an unmistakable authorization:

```powershell
synth workspace prepare-nomos-python C:\EncoderGym\Projects\Nomos --runtime C:\isolated\nomos-runtime --python C:\path\to\python.exe --allow-network-install
```

The command re-verifies the workspace, runtime, active baseline, Python version,
and missing modules before invoking that interpreter's pip with no input,
version-check disabled, and binary distributions only. If pip itself is absent
or unusable, the same intent first restores it through Python's bundled offline
`ensurepip` module and reports `pipBootstrapped: true`; it still accepts no
renderer-authored package. The renderer supplies no package name, index,
executable path, environment map, or shell text. Pip may use the configured
package index and install transitive dependencies into the selected environment;
the confirmation states that mutation and network boundary before it runs.
Installer output is not relayed into application errors where authenticated
index URLs could leak.

Without a history selection, the binding command repeats every check, rejects
an incomplete interpreter, and initializes a new production scientific store
at `runs/scientific.sqlite`. With a history selection it additionally requires
the current schema, a complete SQLite integrity check, and the exact current
runtime project snapshot. It then uses SQLite's transactionally consistent
snapshot operation so committed WAL state is included, hashes the standalone
copy, publishes it below `runs/` under its content identity, and records the
hash and byte count in the binding. The selected source database is opened
read-only and remains separate and unchanged. Neither path starts training,
evaluates a model, or calls a provider. The source `fitz-tool` repository is not
a valid runtime.

`workspace readiness` is passive: it reopens and rehashes the managed project,
reproduces the active runtime and scientific-store identities when configured,
and reports stable required checks with one actionable next intent. It never
migrates a database, runs training, evaluates sealed evidence, or contacts a
provider. Imported files are reported as custody and cannot satisfy scientific
snapshot or review requirements.

After an exact reviewed optimize manifest exists, resolve the same immutable
definition used by start:

```powershell
synth workspace readiness C:\EncoderGym\Projects\Nomos --manifest C:\path\to\optimize.json
```

The result includes a launch preview only when the manifest, approved native
training snapshot, active benchmark generation, candidate set, finite budget,
bound runtime, and scientific project all agree. If that exact manifest already
owns a nonterminal run, readiness directs the operator to resume it instead of
offering a duplicate start.

The managed launch boundary exposes fixed intents and derives the database,
runtime, executable, and project scope from the active scientific binding:

```powershell
synth workspace optimize C:\EncoderGym\Projects\Nomos preview --manifest C:\path\to\optimize.toml
synth workspace optimize C:\EncoderGym\Projects\Nomos start --manifest C:\path\to\optimize.toml
synth workspace optimize C:\EncoderGym\Projects\Nomos status <run-id>
synth workspace optimize C:\EncoderGym\Projects\Nomos resume <run-id>
```

The boundary also provides fixed `inspect`, `review-repair`, `review-delta`,
`authorize-external`, `authorize-sealed`, `cancel`, `doctor`, `provenance`, and
`report` intents. It accepts no database URL, runtime path, Python selection,
environment map, SQL, or shell text from the caller. Every invocation reopens
the managed workspace, verifies the active baseline and binding, reproduces the
Nomos runtime project, confirms the store contains that exact project, and
rejects runs or manifests from another project. Starting remains idempotent;
resuming advances at most one persisted stage.

For the normal desktop path, a user does not create that manifest by hand.
When readiness finds exactly one unexpired approved repair selection bound to
the active, unused successor benchmark, **Prepare approved run** invokes the
fixed `workspace prepare-optimization` intent. It replays the approved native
delta through the production-repair owner, creates or adopts its immutable
logical training snapshot, and publishes a content-addressed strict definition
under the managed project's `runs/definitions/` directory. The step performs
no model training, evaluation, provider call, or sealed-evidence exposure.
Historical, expired, superseded, exposed, foreign-project, and ambiguous
approvals are never silently selected. Electron retains the definition path in
the main process and returns only an opaque project-scoped token to the
renderer.

On a later application launch, readiness recomputes the expected definition
from the current persisted authority and training snapshot. If and only if its
exact content-addressed file still exists inside this project, the main process
restores a fresh opaque token and the screen returns directly to the prepared
preview or existing run. Recovery is read-only and does not repeat native delta
replay. A missing, changed, stale, or ambiguous definition returns to an
actionable preparation state instead.

Once a run has been reserved, its append-only definition and event journal are
the recovery authority even after the proposal's pre-launch expiry. The newest
run for the exact bound scientific project is replayed through the optimization
owner and can be opened without recovering a manifest path. A current approved
successor still takes precedence over unrelated historical terminal runs.

## Provider settings and credentials

The desktop separates reusable connections from run authority. A project may
have up to 50 connections. Adding one requires only an API base URL and API key;
the main process queries the OpenAI-compatible `GET <base>/models` endpoint and
caches the returned model identities. DeepSeek and the operator's Yan gateway
are URL presets, not hard-coded model catalogs. API keys are encrypted in the
desktop profile and never enter project metadata, activity events, or renderer
state after submission.

The user independently assigns any discovered model to Agent and Data
generation. Both roles may use different models from one connection, the same
model, or models from different connections. Saving those assignments compiles
the existing strict role-based provider catalog below. Optimization launch then
pins that catalog revision, endpoint, model, authentication mode, and finite
limits for reproducibility. Model discovery does not authorize provider use;
only execution of a separately authorized run may resolve the selected keys.
An optional evaluator remains part of the lower-level contract.

```json
{
  "version": 1,
  "generation": {
    "kind": "openai-compatible",
    "endpoint": "https://api.openai.com/v1",
    "model": "generation-model",
    "authentication": "bearer",
    "environment_fallback": "SYNTH_OPENAI_API_KEY",
    "limits": {
      "maximumRequests": 100,
      "maximumInputTokens": 1000000,
      "maximumOutputTokens": 200000,
      "maximumCostMicrousd": 5000000
    }
  },
  "advisor": {
    "kind": "openai-compatible",
    "endpoint": "https://api.openai.com/v1",
    "model": "advisor-model",
    "authentication": "bearer",
    "environment_fallback": "SYNTH_ADVISOR_API_KEY",
    "limits": {
      "maximumRequests": 20,
      "maximumInputTokens": 200000,
      "maximumOutputTokens": 50000,
      "maximumCostMicrousd": 2000000
    }
  }
}
```

Configure and inspect it with:

```powershell
synth workspace providers C:\EncoderGym\Projects\Nomos configure --file C:\path\to\providers.json
synth workspace providers C:\EncoderGym\Projects\Nomos show
```

Updates require `--expected-revision-id` from the current status, preventing a
stale screen from overwriting newer settings. Provider revisions are immutable
and append-only. The project stores only a project-and-role scoped secret
reference and the optional environment-variable fallback—not the key. Status
returns only `available`, `missing`, or `unavailable`; it makes no network call
and never prints the environment value. Desktop-managed secret submission is
owned by the Electron main-process credential boundary, not this registry.
