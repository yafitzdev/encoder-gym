# Production readiness

Encoder Gym is an actively developed local platform, not a declared production
release. This page separates implemented behavior from the evidence and release
work around it.

## Current capability

| Area | Current evidence |
|---|---|
| Offline workflow | The checked-in pilot runs deterministically without credentials, downloads, Python, or GPU |
| Core lifecycle | Dataset preparation, immutable snapshots, training, evaluation, error analysis, review, and finite optimization are implemented as persisted Rust components |
| Recovery | Generation, training, workflows, experiments, and optimization runs use durable identities and explicit recovery paths |
| Governance | Development and sealed evaluation roles, finite budgets, reviews, and promotion decisions are represented explicitly |
| Production adapters | Provider-neutral contracts accept compiled task adapters; Nomos is the first isolated adapter |
| Desktop | Managed project custody, inspection, comparisons, settings, and activity history are available locally |
| Real-run evidence | One dated Nomos candidate completed the negative-result path and correctly retained its baseline |

## Not yet claimed

- A stable public API or compatibility window.
- Hosted, authenticated, collaborative, or distributed operation.
- Universal support for encoder tasks, architectures, or model formats.
- Broad evidence that optimization improves production encoders.
- A public binary/package release or open-source license.
- Complete desktop parity with the CLI and every compiled adapter.

## Verification entry points

- [`QUICKSTART.md`](QUICKSTART.md) for the deterministic product journey.
- [`DEVELOPMENT.md`](DEVELOPMENT.md) for formatting, linting, and tests.
- [`evaluation/current.md`](evaluation/current.md) for checked-in measured evidence.
- [`LIMITATIONS.md`](LIMITATIONS.md) for explicit product boundaries.
