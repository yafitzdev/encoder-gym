# Finite encoder optimization

`synth encoder optimize` is the high-level CLI for one bounded, post-review
encoder improvement cycle. It composes the existing repair, training,
evaluation, renewable-benchmark, and campaign contracts. It does not copy or
replace their business logic.

The V1 manifest deliberately starts after two human review boundaries. Create
and approve a repair proposal and native delta with `production-repair`, then
freeze a `training-snapshot`. The optimize manifest may consume that immutable
snapshot; it cannot revise either approval.

## Manifest

The strict TOML schema pins:

- the clean project revision and compiled `nomos` adapter;
- exact training-snapshot identity, content fingerprint, and specification;
- one active, fresh, unused renewable benchmark generation;
- explicit sealed-use approval;
- worst-suite-first development selection and the unchanged metric contract;
- optionally, an exact previously proven protocol and run to adopt.

Unknown fields and unsupported policy values fail closed. A normal new run
omits `[existing_experiment]`. Adoption exists only to link an already-finished
experiment into the productized workflow without spending training or
evaluation budget again; all candidate parameters, suites, metrics, gates,
project identity, protocol budget, and journal head must match exactly.

The checked-in [proven Nomos manifest](examples/nomos-proven-optimize.toml)
documents the real negative experiment. Its IDs are meaningful only with the
isolated experiment database and checkout used for that proof.

## Operator flow

Every mutating command stops at the next durable stage boundary:

```powershell
synth encoder optimize preview --manifest optimize.toml --workspace <ISOLATED_PROJECT>
synth encoder optimize start --manifest optimize.toml --workspace <ISOLATED_PROJECT>
synth encoder optimize status <RUN_ID> --workspace <ISOLATED_PROJECT>
synth encoder optimize resume <RUN_ID> --workspace <ISOLATED_PROJECT>
```

Repeat `resume` only when `status.next_command` says `resume`. The workflow
reserves campaign, protocol, and experiment-run identities when it starts.
Retries can therefore adopt only the exact intended child artifact.

If development selects a fully eligible candidate, status stops with
`human_authorization_required: true` and `next_command: authorize-sealed`:

```powershell
synth encoder optimize authorize-sealed <RUN_ID> --authorized-by <ACTOR> --workspace <ISOLATED_PROJECT>
synth encoder optimize resume <RUN_ID> --workspace <ISOLATED_PROJECT>
```

Authorization binds the exact selected candidate and journal. It never obtains
a replacement sealed cohort. When no candidate passes every development suite,
the run retains the baseline and leaves the sealed generation active and
unused.

Other commands are:

- `inspect`: immutable definition, reservations, and linked state;
- `review-repair` / `review-delta`: verify the frozen pre-optimize approvals;
- `authorize-external`: report the exact external-call boundary (the proven
  Nomos proposal authorizes zero external calls);
- `cancel`: append cancellation before another stage starts;
- `doctor`: expensive native replay plus complete provenance verification;
- `provenance`: row-free machine-verifiable evidence bundle;
- `report`: deterministic management report with hypotheses, data changes,
  checkpoints, suite results, gates, approvals, usage, and limitations.

`status` is intentionally cheap. It verifies immutable storage envelopes and
hash-chained journals without reconstructing the full native delta. `start`
performs a full graph check, every native side-effect re-inspects the trusted
project, and `doctor` reconstructs the native evidence again.

Cancellation is observed between synchronous local stages. V1 does not claim
to preempt a Python trainer already running inside one `resume`; its exact
reserved attempt completes or fails before the next stage is allowed.

## Real Nomos outcome

The proven run genuinely fine-tuned one candidate from 6,800 base rows plus a
192-row reviewed repair delta. It failed both unchanged development contracts:
generic MRR changed by `-0.000026190476`, and retired-post-scaling MRR changed
by `-0.08103442021`, with additional recall regressions. Encoder Gym correctly
retained the baseline, selected no candidate, and did not expose the candidate
to sealed evidence.

The productized replay is optimization run
`2317e08b-5848-4773-9a9e-42499ee09815`. It completed with journal head
`sha256:37d6e0a16516da7a7edf79773b4e3a68dab84338eabbb4097523a9817e6ed691`.
Repeated `start` and terminal `resume` calls returned that same run without new
artifacts or budget spend. Repeated provenance reconstruction produced the
same row-free bundle fingerprint
`sha256:19d30e96ebe62da73836cd285364208fa57d20f9662f34bd95d5409e70850eb4`,
and `doctor` passed native delta replay plus campaign and journal verification.
The successor benchmark generation remained active with zero sealed
exposures.

This is a successful safety outcome, not a promotion. A future experiment must
use a new reviewed hypothesis and immutable training snapshot; it must not
weaken the gates or reuse this run as evidence of improvement.
