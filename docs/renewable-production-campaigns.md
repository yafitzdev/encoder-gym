# Renewable production campaigns

Encoder Gym can now run a bounded encoder experiment against multiple named
development suites while reserving one independently qualified sealed cohort
for explicit acceptance use.

The campaign layer coordinates existing benchmark-generation and experiment
contracts. It does not train models, evaluate models, construct benchmark
rows, or decide metric gates itself.

## Safety model

- A benchmark generation pins exact development-suite authorities, one sealed
  suite, contamination evidence, qualification, independent approval, and a
  finite freshness window.
- A previously consumed sealed cohort can be imported as a historical anchor.
  The anchor is created directly in `exhausted` state and can never become
  adaptive authority.
- Successor activation atomically supersedes the exhausted predecessor and
  activates the ready successor.
- Every candidate is evaluated independently on every named development
  suite. Missing evidence or one failed suite makes the candidate ineligible.
- Candidate selection maximizes the worst-suite primary improvement, then the
  mean improvement, with a stable identity tie-break.
- `advance` stops before sealed use. Only `authorize-sealed` can permit the one
  selected candidate assessment.
- A sealed assessment atomically exhausts the generation and finalizes the
  campaign iteration.
- If no candidate passes every development suite, the campaign retains the
  baseline without a sealed call. The campaign can finish while the unused
  sealed generation remains active.

`production-campaign doctor` deeply replays the project, predecessor and active
generation journals, protocol, experiment journal, per-suite reports, campaign
journal, and sealed-exposure count. `production-campaign provenance` emits the
same row-free chain with immutable identities and fingerprints.

## Real Nomos successor campaign

The authoritative local journal is:

`C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment\encoder-gym-final.sqlite`

It records:

- historical generation `3663775e-1367-4c1d-87d7-fae615fbcf97`, superseded;
- successor generation `10cba5de-e501-4169-a603-27f75c2abd37`, active and
  unused;
- campaign `8a6e1004-a9df-4b3c-a127-42b121ed337c`, completed;
- protocol `c141a389-7c00-4d3a-b1a3-65fa0666b41a`;
- run `9c7aef1b-173e-4245-b301-26691edfcc0c`;
- final decision `retain_baseline`;
- zero successor-sealed candidate exposures.

The 2.5% MNRL interpolation candidate passed the generic suite but failed the
retired post-scaling MRR improvement gate. The 5% candidate passed the retired
post-scaling suite but failed generic Recall@2. Neither candidate was eligible
for sealed assessment. This is an honest negative result and demonstrates why
the two cohorts must remain independent.

The verified provenance fingerprint for this run is
`sha256:152797c4cdba1045baaf1a0adba74c535a1eb58964ec1fb7c12d206d2110029d`.

## Remaining evidence boundary

The successor sealed cohort remains available for a future bounded campaign,
but a future candidate must first pass both current development suites. Its
rows and candidate metrics remain unavailable to adaptive selection. If it is
eventually consumed, another independently acquired and approved generation is
required before further adaptive work.
