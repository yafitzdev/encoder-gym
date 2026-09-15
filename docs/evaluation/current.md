# Current evaluation evidence

This page maps the evidence currently checked into the repository. It does not
claim that historical runs were repeated against the latest revision.

## Deterministic product path

The offline pilot exercises project preparation, immutable inputs, workflow
execution, hashing-linear training, evaluation, provenance, and recovery without
network access or external credentials. Run it through
[`../QUICKSTART.md`](../QUICKSTART.md).

## Real encoder experiment

The recorded Nomos repair-training experiment from 7 September 2026 trained one
CPU triplet candidate. The candidate regressed on its unchanged development
contracts, so the persisted decision retained the baseline. The measured
outcome, artifact fingerprints, and safety boundary are recorded in
[`encoder-optimize.md`](../features/optimization/encoder-optimize.md).

That run is evidence of a complete negative-result path, not evidence that the
current checkout improves Nomos or other encoders.

## Desktop evidence

The dated [`Run 14 review`](run-14-review.md) and
[`managed onboarding record`](nomos-managed-onboarding-2026-09-08.md) document
the inspected desktop behavior. Current launch and smoke commands live in
[`../../ui/README.md`](../../ui/README.md).

## Release claims

See [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md) for the distinction
between implemented behavior, checked-in evidence, and work still required for
a public release.
