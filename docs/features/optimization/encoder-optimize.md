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

The manifest may start a genuinely new training run or explicitly adopt an
existing experiment. Adopted IDs are meaningful only with the exact experiment
database and checkout that produced them.

## Operator flow

`start` reserves the run. `drive` then authorizes and advances its ordinary
stages automatically until completion or a separate approval boundary:

```powershell
synth encoder optimize preview --manifest optimize.toml --workspace <ISOLATED_PROJECT>
synth encoder optimize start --manifest optimize.toml --workspace <ISOLATED_PROJECT>
synth encoder optimize status <RUN_ID> --workspace <ISOLATED_PROJECT>
synth encoder optimize drive <RUN_ID> --authorized-by <ACTOR> --workspace <ISOLATED_PROJECT>
```

For deliberate single-stage inspection, `resume` remains available:

```powershell
synth encoder optimize resume <RUN_ID> --workspace <ISOLATED_PROJECT>
```

Repeat `resume` only when `status.next_command` says `resume`. The workflow
reserves campaign, protocol, and experiment-run identities when it starts.
Retries can therefore adopt only the exact intended child artifact.

If development selects a fully eligible candidate, status stops with
`human_authorization_required: true` and `next_command: authorize-sealed`:

```powershell
synth encoder optimize authorize-sealed <RUN_ID> --authorized-by <ACTOR> --workspace <ISOLATED_PROJECT>
synth encoder optimize drive <RUN_ID> --authorized-by <ACTOR> --workspace <ISOLATED_PROJECT>
```

Authorization binds the exact selected candidate and journal. It never obtains
a replacement sealed cohort. When no candidate passes every development suite,
the run retains the baseline and leaves the sealed generation active and
unused.

Automatic execution records one hash-chained authorization event for the exact
reserved run before work begins. It does not change the recipe, approved data,
spending limits, candidate identities or final-evaluation authority. The same
authorizer can retry without another authorization event. Another authorizer
cannot replace that record. Historical event bytes and fingerprints remain
unchanged; only the new authorization event uses schema version 2.

One process lease spans the whole drive, excluding concurrent drive/resume
workers. Between stages it reloads persisted cancellation and requires a strictly
decreasing finite lifecycle rank. Errors stop the invocation; explicit retries
recover the exact reserved children through existing slice contracts rather
than allocating new attempts or silently repeating uncertain external calls.
Status exposes the authorizer and live worker separately. Standard output is one
final status object; native progress remains on stderr and in durable journals.

This is the post-review execution primitive, not the complete input-first
Optimize product: selected dataset admission, bounded agent decisions and launch
budgets still need integration. `drive` never substitutes an old recipe for the
new desktop's saved inputs.

The experiment runner reuses a persisted sealed authorization only for the
same authorizer. If interrupted after saving the sealed report but before
finalizing its decision, it finishes from that report without another backend
evaluation. A completed sealed run is also safe to inspect through a repeated
`experiment run-sealed` command.

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

`status` verifies immutable storage envelopes and hash-chained journals.
Preview, launch validation, routine inspection, and reporting load persisted
facts without opening the native backend. Reports are available before protocol
creation and after cancellation, and their evidence limits reflect the run's
actual results. Terminal `resume` also works without native adapter files.
`start` performs a full graph check, every native side-effect re-inspects the trusted
project, and `doctor` reconstructs the native evidence again.

Cancellation is observed between synchronous local stages. V1 does not claim
to preempt a Python trainer already running inside `resume` or `drive`; its exact
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

Ordinary process coverage now exercises new non-adopted runs, child-to-parent
interruption recovery, approval retries, cancellation, development rejection,
sealed rejection, promotion, training failure, and reports. See
[`development.md`](../../DEVELOPMENT.md) for the deterministic test composition.
Further real experiments require their own reviewed manifest and immutable
training snapshot.
