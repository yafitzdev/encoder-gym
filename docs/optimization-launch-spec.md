# Project optimization launch

Implements the input-first Optimize contract in `encoder-workspace-product.md`.
The user selects the active baseline model, an exact training dataset version,
and an exact shared benchmark version. A training recipe is an execution detail,
not the object the user must supply to set up optimization.

## Saved setup

`project-workspace-core` owns immutable setup revisions. Each revision pins the
managed project, baseline revision and model content identity, dataset-core
version reference, and shared benchmark version reference. Changing any selected
input creates a successor; creating another dataset or benchmark version does
not silently change an existing selection. The selected dataset is the starting
population, not a claim about the baseline's historical training data.

The local adapter resolves only project-owned artifacts and records the setup
with compare-and-append in the custody database. Preview and save reject a
non-baseline model, foreign references, empty or test-partition training input,
and substituted fingerprints. Save rechecks the active baseline in its database
transaction. Exact retry IDs reuse their original revision; an old retry never
changes which revision is current. History survives baseline changes and moved
folders without rewriting old selections. Saved setup is configuration, not
training qualification, provider authorization, holdout approval or a run.

The CLI exposes read-only preview/history and explicit audited save. Requests
contain identities only, never model bytes, dataset rows, benchmark content or
credentials. Each save command has a UUID activity action and safe references.

```text
synth workspace optimization-setup <PROJECT> preview --model <MODEL> --dataset-version <DATA_VERSION> --benchmark-version <BENCHMARK_VERSION>
synth workspace optimization-setup <PROJECT> save --file <REVIEWED_REQUEST_JSON>
synth workspace optimization-setup <PROJECT> list
```

The save request contains `id` (a stable retry UUID), `expectedParent` (the
previous setup UUID or null), and the exact `inputs` returned by preview.

## Execution integration

The project-header Optimize action opens the desktop input selector. It shows
the active baseline and named dataset/benchmark versions, with links to their
ordinary viewers. First setup prefers the model's recorded training version;
multiple unrelated datasets require a choice. Saved versions never follow a
newer dataset or benchmark automatically. Reopening a project reloads persisted
selection history and a changed baseline requires a new saved selection.

Save uses the same preview/save CLI, checks the preview against the displayed
identities and expected parent, and preserves its exact retry UUID after a lost
save or reload response. Changing inputs or refreshing abandons that retry.
Late responses from an old project location cannot replace current selection.
The renderer supplies no paths, credentials, native rows or execution settings.
The main process writes only a strict temporary request and removes it on both
success and failure. Each explicit save retains the ordinary CLI action UUID.

Saving inputs is available now; automatic execution is not connected yet and
the desktop states that limitation. It must not imply a run has started or
fall back to a previously prepared recipe. Existing runs are opened from Runs
and retain their current supervision, recovery and approval controls.

Optimize must consume this exact saved setup, resolve finite training/iteration
and separate provider budgets, and persist explicit execution authorization.
The bounded agent uses development evidence to choose generation/training work;
it cannot alter the pinned evaluation definition or baseline. Task adapters
own native row validation and materialization. Existing qualification,
contamination, experiment, acceptance and exposure contracts remain mandatory.

Routine authorized stages continue automatically with persisted child IDs,
recoverable progress and cancellation. A protected final evaluation remains a
pause unless its exact use is covered by launch authorization. Neither saving
setup nor storing API keys authorizes execution. The existing post-review
optimizer remains a compatibility path until the input-first executor is wired;
it must not silently run a different recipe or dataset in response to a setup.

## Verification

- Pure input binding and immutable revision/history checks.
- Actual CLI preview/save/retry, changed selections, stale parent/baseline,
  foreign artifacts, altered source bytes and request tampering.
- Setup persistence after folder movement; unchanged model, dataset, benchmark
  and scientific records; UUID activity without row content.
- Follow-on desktop selection and automatic bounded execution must be tested
  end to end; a working setup catalog alone does not complete the goal.
