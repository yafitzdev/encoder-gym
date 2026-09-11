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

## One-click launch authority

`project-workspace-core` also owns the immutable authority created by one
Optimize click. It pins the exact current setup, the exact current provider
catalog revision, separate generation and advisor ceilings, and a finite
execution envelope. The fixed envelope permits at most three iterations, three
models, 5,000 dataset-row changes, six hours of training, one development
evaluation per model and development suite, and one final evaluation of the
development-selected candidate. It is deliberately not another user form.

The authority contains provider-catalog identity and non-secret ceilings only.
It contains no credential, environment value, model bytes, dataset row, or
evaluation payload. A new authority is rejected if the baseline, selected setup,
provider revision, dataset, or benchmark changed after preview. Exact retry
reuses its authorization; every deliberate later Optimize click may use a new
UUID. Immutable history remains verifiable after provider settings change.

The local adapter keeps preview and history read-only, upgrades only during the
explicit authorization mutation, and records a safe `optimization.launch`
activity action. The CLI contract is:

```text
synth workspace optimization-launch <PROJECT> preview --setup <SETUP_ID>
synth workspace optimization-launch <PROJECT> authorize --file <REVIEWED_REQUEST_JSON>
synth workspace optimization-launch <PROJECT> list
```

The request contains a stable retry `id` and the exact `scope` returned by
preview. Users do not author the scope or its limits.

## Desktop and execution integration

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

Saving inputs and the one-click authorization boundary are available now;
input-first execution is not connected yet and the desktop must not imply a run
has started or fall back to a previously prepared recipe. Existing runs are
opened from Runs and retain their current supervision, recovery and approval
controls.

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
- Actual CLI launch preview/authorization/retry, version-9 read-only behavior,
  stale provider revision, immutable storage, historical verification, exact
  limits, and activity redaction.
- Follow-on desktop one-click composition and automatic bounded execution must
  be tested end to end; setup and authorization catalogs alone do not complete
  the goal.
