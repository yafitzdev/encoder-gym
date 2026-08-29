# Project instructions

## Scope

Build the local encoder-development platform incrementally through the slices
defined in `docs/platform-spec.md` and the individual slice specifications.

The platform slices are:

1. Synthetic Data Generation
2. Dataset Management
3. Encoder Training and Checkpoints
4. Evaluation
5. Error Analysis
6. Optimization

After the independent slices, the controlled-workflow phase may compose their
existing contracts into one finite local run. It may automate explicitly
authorized stages and bounded iterations, but it must preserve slice ownership,
sealed-evidence isolation, external-call budgets, and deterministic acceptance.

Each slice must remain independently useful and replaceable. Do not add cloud
deployment, authentication, multi-user collaboration, distributed execution,
multiple workers, autonomous agents, bandits, reinforcement learning, or
semantic/embedding deduplication unless a later specification explicitly adds
one of those capabilities.

## Required reading

Before implementation work, read:

1. `docs/platform-spec.md`
2. The specification for the slice being changed
3. `docs/architecture.md`
4. `docs/development.md`

## Engineering rules

- Implement one coherent component at a time.
- Begin each slice with domain and pure logic; expose it through the CLI before
  building HTTP API or graphical UI surfaces.
- For Slices 2–6 and controlled workflow orchestration, implement core logic,
  persistence, tests, and CLI only. Do not
  extend the HTTP API or graphical UI until the user explicitly requests that
  separate phase.
- Keep domain objects independent from SQLite, HTTP, SDKs, CLI parsing, and
  presentation concerns.
- Own replaceability contracts inside the relevant core crate. Provider or
  framework types must not leak through ports.
- Keep orchestration, persistence, adapters, and presentation separate.
- Derive metrics, progress, dataset contents, and recommendations from
  persisted facts rather than presentation state.
- Preserve immutable provenance between slices: every derived artifact must
  identify its source snapshot, run, model, evaluation, or analysis.
- Avoid god modules, generic `services.rs`, generic `utils.rs`, speculative
  abstractions, and scaffolding not required by an active slice.
- Prefer deterministic algorithms and fakes in ordinary tests.
- Never commit API keys, credentials, private model tokens, or raw secrets.

## Slice boundaries

- Data generation must not know how datasets are split or models are trained.
- Dataset management must not know which trainer or model implementation runs.
- Training backends must not decide dataset membership or evaluation policy.
- Evaluation must consume a model/predictor port and immutable snapshot data.
- Error analysis must consume persisted predictions and metrics, not rerun
  training or generation.
- Optimization may propose explicit generation targets or training settings,
  but must not mutate source datasets, model artifacts, or historical runs.
- Workflow orchestration may invoke the normal slice contracts and link their
  immutable artifacts. It must not copy slice business logic, expose sealed
  evidence to adaptive components, infer approval, or exceed persisted finite
  execution budgets.

## Validation and commits

After each coherent component:

1. Run `cargo fmt-check`.
2. Run `cargo check-all`.
3. Run `cargo lint`.
4. Run `cargo test-all`.
5. Review dependency directions and public interfaces.
6. Commit the coherent, working change when Git identity is configured.

Most tests must use deterministic fakes and must not require an external API,
model download, GPU, or paid service.
