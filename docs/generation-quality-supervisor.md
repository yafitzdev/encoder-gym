# Generation quality supervisor

The generation supervisor is an optional CLI-only control loop around ordinary
Slice 1 generation. It breaks a plan into finite segments, audits every new
row with a blind evaluator, pauses weak or drifting scopes deterministically,
and permits a new prompt-guidance version only after bounded Pi diagnosis,
authorization, and a passing canary.

It does not train an encoder, modify labels or dimensions, expose sealed
evidence, or let Pi decide whether quality passed.

## Prepare a contract

Start with [`examples/supervisor/contract.json`](../examples/supervisor/contract.json)
and replace `plan_id`. The strict authoring document selects both independent
plugin boundaries:

- `generator`: `fake` or `openai_compatible`;
- `evaluator`: `fake` or `openai_compatible`.

The fake pair is deterministic and offline. The repairable fake generator is
intentionally weak before the example revision, so it exercises the complete
pause-and-repair path. The fake evaluator is a blind lexical test adapter, not
a production semantic judge. Select `openai_compatible` for semantic review of
real data. The evaluator receives candidate text and allowed vocabularies, but
not the assigned label, assigned dimensions, provenance, or sealed evidence.

Configure the saved OpenAI-compatible endpoint/model before previewing a
contract that selects either real adapter:

```powershell
synth backend configure --base-url https://provider.example/v1 --model provider-model
```

Preview resolves the dataset, plan, current accepted coverage, semantics,
approved authenticity context, construction plan, approved strategy context,
backend identities, evaluator egress, thresholds, and every finite budget. It
writes nothing. Create persists those exact values and their fingerprint:

```powershell
synth supervisor contract-preview supervisor-contract.json
synth supervisor contract-create supervisor-contract.json
synth supervisor contract-show <CONTRACT_ID>
```

Changing a bound dataset, plan, context, endpoint, model, construction plan, or
starting coverage makes later startup fail closed. Create a new contract rather
than trying to update the old one.

## Run finite generation

`start` queues state but performs no provider I/O. One or more `--guidance`
values form immutable prompt version 0:

```powershell
synth supervisor start <CONTRACT_ID> --guidance "Use natural customer language."
synth supervisor run <RUN_ID>
```

`run` continues only until a durable boundary: paused diagnosis, review,
canary, completion, failure, or cancellation. With real adapters, credentials
are read only from environment variables:

```powershell
$env:SYNTH_GENERATOR_KEY = "..."
$env:SYNTH_EVALUATOR_KEY = "..."
synth supervisor run <RUN_ID> --api-key-env SYNTH_GENERATOR_KEY --evaluator-api-key-env SYNTH_EVALUATOR_KEY
```

Omit `--evaluator-api-key-env` to reuse the generation key. Environment-variable
names and secret values are not persisted in the quality contract. Status and
watch are read-only; watch never starts a segment:

```powershell
synth supervisor status <RUN_ID>
synth supervisor watch <RUN_ID>
synth supervisor issues <RUN_ID>
synth supervisor strategy-coverage <RUN_ID>
```

`issues` includes the append-only state history, normalized windows, criterion
failures, issue counts, threshold observations, and deterministic decisions.
Rows that fail the supervisor contract remain immutable evidence but do not
contribute to qualified coverage or reduce the remaining generation target.

## Diagnose and authorize a repair

Build the pinned Pi sidecar once:

```powershell
cd adapters/research-agent-pi
npm install
npm run build
cd ../..
```

The checked-in offline script uses exactly the seven supervisor tools and no
network access:

```powershell
synth supervisor diagnose <RUN_ID> --script examples/supervisor/diagnosis.json
synth supervisor revision-show <SESSION_ID>
synth supervisor revision-review <RUN_ID> <SESSION_ID> --approve --reviewer operator --reason "Guidance-only repair reviewed"
```

For a real Pi model, omit `--script` and select its provider/model plus the
environment-variable name containing its key:

```powershell
synth supervisor diagnose <RUN_ID> --provider openai --model <MODEL> --api-key-env PI_API_KEY
```

Pi can inspect only aggregate contract/window/failure/guidance facts, preview
one bounded guidance patch, submit it, and finish. The host rejects attempts to
alter protected prompts, schema, label/dimension targets, semantic authority,
construction, thresholds, budgets, approval, or safety policy.

An `explicit_review` contract uses `revision-review`. A contract with a valid
`finite_preauthorization` envelope instead uses:

```powershell
synth supervisor revision-authorize <RUN_ID> <SESSION_ID>
```

The exact proposal must fit the persisted envelope; this command cannot infer
or broaden permission.

## Canary, completion, and recovery

Approval creates an inactive immutable prompt candidate. Only `canary` performs
its new generation/audit child and only a deterministic pass activates it:

```powershell
synth supervisor canary <RUN_ID>
synth supervisor issues <RUN_ID>
synth supervisor run <RUN_ID>
```

A failed canary remains evidence, cannot become active, and does not count
toward qualified coverage. A passing canary resumes from persisted absolute
coverage; accepted rows from an earlier prompt are not regenerated.

Cancellation is persisted before forwarding to exact linked children.
Recovery interrupts uncertain external calls and never replays them or infers
approval:

```powershell
synth supervisor cancel <RUN_ID>
synth supervisor recover <RUN_ID>
```

## Integrity and provenance

Trace a supervised row and verify the complete local fact graph with:

```powershell
synth supervisor trace-row <ROW_ID>
synth supervisor integrity
synth doctor
```

The trace binds the row to its job and attempt evidence, prompt version,
optional exact strategy assignment, quality observations/windows/decisions,
supervisor run, and immutable contract. `integrity` and `doctor` fail closed on
fingerprint, append-only chain, child ownership, usage, strategy conservation,
prompt protection, authorization, canary activation, or provenance tampering.

The generic `rows` and `export` commands reflect structural generation status.
For training-data admission, continue to use the independent dataset-quality
curation and approved snapshot flow. Supervisor-qualified coverage prevents
weak rows from satisfying generation targets; it is not a hidden mutation of
historical row status.
