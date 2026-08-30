# Reproducible project configuration

Project files use strict, versioned TOML. Unknown fields are rejected so a typo
cannot silently change a run. `examples/project.toml` is the complete reference.

```text
cargo run -p synthetic-data-cli -- config validate examples/project.toml
cargo run -p synthetic-data-cli -- --output json config show examples/project.toml
cargo run -p synthetic-data-cli -- config init examples/project.toml
```

`config init` atomically creates the dataset definition, initial equal-target
generation plan, optional non-secret OpenAI-compatible backend settings, and a
copy of the fully resolved configuration. Failure rolls the whole transaction
back. The persisted record includes a deterministic SHA-256 fingerprint.

## Sections

- `dataset`: name, task, labels, and zero or more arbitrary categorical
  `[[dataset.dimensions]]` entries.
- `generation`: initial equal target, batching/retry policy, backend, model,
  normalized generation parameters, generic provider `extra` parameters, and
  `api_key_env`.
- `snapshot`: name, description, ratios, and deterministic split seed.
- `training`: local backend, optional registered `base_model_id`, feature
  dimension, epochs, learning rate, L2, checkpoint cadence, seed, and artifact
  directory.
- `training.transformer`: maximum sequence length, batch size, weight decay,
  warmup ratio, gradient clipping, and `fine-tune` or `frozen` encoder mode.
- `evaluation`: snapshot split to evaluate.

Explicit command flags take precedence over TOML values; TOML values take
precedence over built-in defaults. Config-aware commands include `generate`,
`snapshot create`, `training run`, and `evaluation run`. `config show` applies
the supported overrides and prints the exact resolved value and fingerprint.

`backend = "hashing-linear"` remains the default and ignores the transformer
section. `backend = "bert-cpu"` requires a `base_model_id` previously returned
by `synth encoder register`. See `examples/transformer-project.toml` and
`docs/transformer-training.md`. Unknown fields are rejected at every level,
including inside `training.transformer`.

`generation.extra` is a strict map of backend-neutral JSON values copied into
the normalized generation request. It is intended for replaceable-provider
controls that have no portable first-class field. It cannot override reserved
request fields such as `model`, `messages`, `response_format`, `temperature`,
`max_tokens`, or `seed`. For example, current DeepSeek V4 models can avoid
spending a bounded JSON-generation budget on reasoning with:

```toml
[generation.extra.thinking]
type = "disabled"
```

Analysis protocols are deliberately resolved on `analysis create` rather than
read from mutable project defaults. The command persists every selected finding
kind, support threshold, dimension intersection, confidence threshold, ranking
policy, evidence limit, contrast setting, deterministic seed, and comparison
ID with a SHA-256 protocol fingerprint. Reproducing a report therefore does not
depend on a later edit to `project.toml`.

Optimization protocols are likewise separate strict TOML/JSON decision
artifacts rather than mutable project defaults. Start with
`examples/optimization.toml`. The normalized protocol and fingerprint are
embedded in every decision-grade proposal. Optional training candidates use a
separate finite, fingerprinted configuration space built by `optimize
training-space`; arbitrary provider setting maps are not accepted.

Governed workflow definitions are separate strict TOML/JSON documents. At
`workflow define`, the platform resolves and fingerprints the project
configuration, development and optional sealed suites, analysis protocol,
optimization protocol, optional advisor policy, fresh-iteration training
policy, governance mode, and finite budgets. Editing project TOML or suite
defaults later cannot change an existing workflow definition or run.
Start with `examples/workflow.toml` and replace its persisted identity and
fingerprint placeholders after creating the project and benchmark suites.

The optional OpenAI-compatible advisor stores only its endpoint, model,
versioned prompt policy, parameters, egress policy, and `api_key_env` name.
`aggregate_only` is the default-safe evidence boundary. Raw development text
requires the explicit `development_text` policy; sealed text is rejected under
every policy.

## Secrets

TOML accepts an environment-variable name such as
`api_key_env = "SYNTH_OPENAI_API_KEY"`; it never accepts a raw API key. Backend
configuration in SQLite also contains no credentials. The named environment
variable is read only when a request is made.
