# Slice 3 product specification — Encoder Training and Checkpoints

> Slice 3.1 extension: the deterministic hashing-linear backend remains the
> baseline, and `training-transformer` adds exact local BERT/F32 CPU fine-tuning,
> bounded batch progress, registered base-model identities, self-contained
> immutable checkpoints, prediction, and explicit provenance-linked
> continuation through the same project-owned ports. See
> `docs/transformer-training.md` and ADR 001. No API or graphical UI was added.

## Objective

Train a reproducible text-classification model from an immutable dataset
snapshot through a project-owned, replaceable training backend.

The application must let a user:

1. Configure a training run from one dataset snapshot.
2. Choose a registered training backend and hyperparameters.
3. Start and cancel a local training run.
4. Observe durable epoch progress and loss.
5. Persist immutable checkpoint metadata and artifacts.
6. Select the final or an earlier checkpoint for later evaluation.
7. Reproduce a run from its snapshot ID, seed, backend, and configuration.

## Training backend port

The platform owns a `TrainingBackend` contract. It receives normalized snapshot
examples and training configuration and emits progress plus immutable model
artifacts. It does not select dataset members, persist runs, evaluate models,
or update UI state.

The first working backend is a deterministic CPU baseline: a fixed-dimensional
hashing text encoder with a trainable multiclass linear softmax head. Its model
artifact is portable JSON and implements the project-owned predictor port. It
is intentionally simple but genuinely trains parameters and exercises the
complete platform contract without model downloads or GPUs.

## Checkpoints

- At least one checkpoint is written per configured interval and a final
  checkpoint always exists after successful training.
- Artifact paths, checksums, epoch, metrics, backend, and model format are
  persisted.
- Checkpoints are immutable and stored beneath a configured local artifact
  root, never inside SQLite blobs.
- A failed or cancelled run retains completed checkpoints and progress facts.

## Test priorities

- deterministic training for the same snapshot/configuration/seed
- measurable loss reduction on a separable fixture
- checkpoint serialization and predictor round trip
- cancellation between epochs
- invalid configuration handling
- no provider types in core contracts

## Completion criterion

Slice 3 is complete when the CLI can train the working CPU backend from a
snapshot, show durable progress, cancel it, and expose loadable immutable
checkpoints through replaceable ports. API and graphical UI work is deferred.
