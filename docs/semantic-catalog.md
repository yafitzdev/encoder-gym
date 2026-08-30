# Semantic catalog

Dataset schema names remain sufficient for simple generation. The semantic
catalog is an optional, local source of reusable meaning for labels and
categorical dimensions. It uses the same SQLite database as the rest of the
application; it is a separate domain component and persistence contract, not a
second service the operator must run.

## Why it exists

Names such as `hard`, `technical`, or `ambiguous` are underspecified. A semantic
profile can define what a target and its values mean, with examples,
counterexamples, inclusion rules, and exclusion rules. The generation prompt
then receives only the guidance relevant to its current cell. The bounded
workflow advisor receives the exact same version-pinned context used by the
generation job, so later profile edits cannot reinterpret historical evidence.

No profile is attached by name alone. `semantic suggest` may show compatible
reusable profiles, but `semantic bind` is the explicit decision that makes one
active for a dataset.

## Artifact model

- A profile is immutable and has a stable logical `key`, an integer `version`,
  a unique ID, a predecessor ID, and a content fingerprint.
- A reusable profile may be bound to compatible datasets. A dataset-scoped
  profile is an override and can only be bound to its declared dataset.
- Binding and unbinding are append-only decisions. Historical bindings are
  never rewritten or deleted.
- Resolution applies the reusable layer first and the dataset override second.
  A nonempty override field replaces that field; unspecified fields continue
  to come from the reusable profile.
- Every generation job stores one immutable semantic assignment before its
  first backend call. Generated-row metadata and provenance retain the exact
  profile IDs, versions, binding decisions, and fingerprints.
- With no bindings, generation behaves as before and asks the model to infer
  meaning from the task and schema names.

Profiles may be partial: they can document only the values that need additional
precision. Unknown labels, dimensions, or values are rejected before a binding
is persisted.

## Profile document

Start from [`examples/semantic-profile.toml`](../examples/semantic-profile.toml).
The strict JSON/TOML schema is:

```toml
schema_version = 1
key = "support-difficulty"
description = "Difficulty is inference burden, not vocabulary rarity."

[scope]
kind = "reusable"

[target]
kind = "dimension"
name = "difficulty"

[entries.easy]
description = "The intent is explicit."
examples = ["Why was I charged twice?"]
inclusion_rules = ["One direct clue is sufficient."]

[entries.hard]
description = "Multiple indirect clues are required."
counterexamples = ["Rare words alone do not make an example hard."]
exclusion_rules = ["Do not include a direct category keyword."]
```

For label semantics, use `[target] kind = "labels"` and keys under `entries`
that match dataset labels. For a dataset override, use:

```toml
[scope]
kind = "dataset"
dataset_id = "<DATASET_ID>"
```

## CLI workflow

```text
synth semantic profile-create examples/semantic-profile.toml
synth semantic suggest <DATASET_ID>
synth semantic bind <DATASET_ID> <PROFILE_ID>
synth semantic bindings <DATASET_ID>
synth semantic resolve <DATASET_ID>
synth generate <PLAN_ID> --backend fake
synth semantic job-context <JOB_ID>
synth provenance generation-semantic-context <JOB_ID>
```

Create a new immutable version and move only future jobs to it:

```text
synth semantic profile-revise <OLD_PROFILE_ID> revised-profile.toml
synth semantic bind <DATASET_ID> <NEW_PROFILE_ID>
```

Detach one layer with an append-only decision:

```text
synth semantic unbind <DATASET_ID> dimension --dimension difficulty --layer reusable
```

`doctor` verifies profile, binding, resolved-context, and job-assignment
fingerprints as well as dataset/job relationships.
