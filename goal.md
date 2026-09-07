# Long-Range Goal — Prove a Native, Non-Adopted Optimization Cycle

Run Encoder Gym's first genuinely new production-encoder optimization through
the complete `synth encoder optimize` operator path, without adopting a
previous experiment, and test one small conservative Nomos training hypothesis
designed to avoid the regressions observed in the first genuine fine-tune.

The product question is:

> Can the reviewed Nomos repair signal be introduced without damaging either
> independent development suite, while preserving sealed evidence until a
> candidate is fully eligible?

This is the highest-impact continuation because the platform has already proved
that it can reject an unsafe candidate and productize the resulting evidence.
The next proof must exercise fresh training and evaluation directly through the
new durable parent. Do not add another agent, dashboard, data source, or broad
optimization subsystem before this path is real.

Promotion is not required. A verified `retain_baseline` is a successful result.

## Required starting context

Read `AGENTS.md`, `docs/current-status.md`, `docs/platform-spec.md`,
`docs/architecture.md`, `docs/development.md`,
`docs/production-encoder-experiment-spec.md`, and `docs/encoder-optimize.md`
before implementation.

Preserve these facts:

- Encoder Gym begins from the documentation-handoff commit that follows
  `118b3d7a279db5b5dd74faafc84db562539aa37d`.
- The isolated Nomos experiment is
  `C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment` at
  `4450ab3f1de8a1fc64bcbe5d77c67d0fb0f99af9`, clean and with no remote.
- The source Nomos repository
  `C:\Users\yanfi\PycharmProjects\fitz-tool` is read-only. Its HEAD is
  `14e0a1667431982ee00ee07108e7d82351fa28eb` and it starts with these existing
  changes:
  - modified `fitz_tool/coding_beta_v4_controls.py`;
  - modified `tools/assemble_beta4_control_dataset.py`;
  - modified `tools/audit_beta4_dataset.py`;
  - modified `tools/diagnose_beta4_control_head.py`;
  - untracked `fitz_tool/coding_beta_v4_live_contrasts.py`;
  - untracked `tests/test_coding_beta_v4_live_contrasts.py`;
  - untracked `tools/generate_beta4_live_contrasts.py`.
- The unrelated untracked Encoder Gym `ui/` directory belongs to the deferred
  GUI attempt. Do not modify, delete, or commit it.
- Active successor benchmark generation
  `10cba5de-e501-4169-a603-27f75c2abd37` is fresh, unused, and has zero
  candidate exposures. Preserve it unless a new candidate passes every
  development gate and the operator explicitly authorizes its exact sealed use.
- The failed genuine candidate
  `44b98240-6b63-49ca-a0c3-21bddba1d151` and optimization run
  `2317e08b-5848-4773-9a9e-42499ee09815` are immutable historical evidence.
  Never overwrite, reopen, or present them as successful.

Record all three repositories' exact HEAD and status before and after work.
Never place credentials, API keys, or remotes in the isolated copy.

## Why the previous candidate failed

The approved 192-row deterministic delta was task-valid and contamination-free,
but one full-model CPU triplet fine-tune over the combined 6,992-row population
regressed both suites:

- `generic_holdout`: MRR `-0.000026190476`, Recall@2 `-0.005`;
- `retired_post_scaling`: MRR `-0.08103442021`, with Recall@1/2/3 regressions.

Treat this as evidence about the complete data-plus-training mechanism. Do not
assume the data is good merely because its structural audit passed, and do not
assume the training recipe is solely responsible without testing that claim.
Do not weaken the metric contract or repeatedly tune against these development
numbers without predeclared bounded hypotheses.

## Milestone 1 — Audit and harden the fresh optimize path

Before spending another real training run, test the path used when the manifest
does not contain `[existing_experiment]`.

Add focused deterministic coverage for:

- fresh protocol and run creation under the pre-reserved IDs;
- duplicate `start` and `resume` without duplicate artifacts or budget spend;
- process interruption before an adapter call, after native output appears, and
  after output is returned but before the next parent event;
- trainer and evaluator failure evidence;
- cancellation from every externally side-effecting child state;
- one candidate failing only one development suite;
- missing suite evidence remaining ineligible;
- full development success pausing for explicit sealed authorization;
- mismatched and repeated sealed authorization;
- atomic one-time sealed consumption and final decision persistence;
- deterministic report and provenance output;
- Doctor rejection of changed files, revisions, database envelopes, journals,
  and foreign child artifacts;
- append-only migration from the current `0007` database.

Prefer extracting a small application-level orchestration unit from the CLI
module if that makes deterministic fake end-to-end testing possible. Keep CLI
parsing and presentation out of the lifecycle logic. Do not create a broad
framework or generic service module.

The ordinary test suite must require no Nomos checkout, model, Python, network,
GPU, or credential. Retain one explicit opt-in isolated-Nomos integrity test.

## Milestone 2 — Define one conservative causal hypothesis

Use persisted development-only evidence and training receipts to specify the
smallest credible alternative to the failed full-model fine-tune. Inspect the
native trainer and model architecture before choosing it.

Good candidate mechanisms may include one or a very small combination of:

- freezing most encoder layers and training only a narrow upper portion;
- a smaller learning rate or shorter schedule;
- an explicit base-replay versus repair-row sampling ratio;
- a regularization or anchoring term against the baseline representation;
- deterministic checkpoint selection from training-internal evidence that is
  separate from both named development suites.

These are possibilities, not instructions to implement all of them. Choose one
mechanism only after determining which can be expressed cleanly by the existing
Nomos trainer and provider-neutral candidate parameter contract. State the
causal hypothesis, expected benefit, main failure mode, and exact finite budget
before viewing new candidate metrics.

Use at most two genuine candidates, and only if the second isolates a specific
mechanism. No search sweep, optimizer agent, reinforcement learning, bandit,
adaptive gate change, or arbitrary hyperparameter exploration.

## Milestone 3 — Create new immutable authority

Historical proposal, delta, snapshot, protocol, run, and candidate artifacts
must remain immutable. Create only the new authority required by the selected
hypothesis:

- a new reviewed repair/training proposal or explicit successor proposal;
- a newly approved native delta selection if row membership or construction
  changes;
- a new logical training snapshot when inputs, membership, or training-visible
  weighting facts change;
- a strict optimize manifest with no `[existing_experiment]` block;
- new content-addressed candidate identities and bounded budgets.

If the same approved row membership is reused, record that honestly and ensure
the new training hypothesis—not accidental UUID churn—is what changes identity.
Do not duplicate large base datasets. Keep native row contents out of Encoder
Gym's database, generic domain objects, reports, and provenance bundle.

No network or paid generation call is authorized. If new data truly requires
one, persist the exact bounded request and stop for explicit authorization.

## Milestone 4 — Run the new experiment through `encoder optimize`

Use only the high-level operator family for the final proof:

```text
synth encoder optimize preview --manifest <NEW_MANIFEST> --workspace <ISOLATED_NOMOS>
synth encoder optimize start --manifest <NEW_MANIFEST> --workspace <ISOLATED_NOMOS>
synth encoder optimize status <RUN_ID> --workspace <ISOLATED_NOMOS>
synth encoder optimize resume <RUN_ID> --workspace <ISOLATED_NOMOS>
```

Advance one persisted stage at a time and inspect status after each boundary.
The fresh run itself must create and use the reserved protocol and experiment
run. It may discover and adopt only its own exact content-addressed output after
an interruption; it may not adopt the prior completed experiment.

Evaluate every candidate independently on both `generic_holdout` and
`retired_post_scaling`. Any failed or missing suite makes a candidate
ineligible. Do not average away a suite-specific regression.

If no candidate passes all development gates, complete with `retain_baseline`
and prove that the successor sealed generation remains active and unused.

If a candidate passes all gates, stop. Report its exact identity, every
development result, journal head, and authorization fingerprint. Do not use
sealed evidence until the user explicitly authorizes that exact candidate in
the active session. After authorization, permit exactly one sealed evaluation
and atomically persist `promote_candidate` or `retain_baseline` under the
unchanged contract.

Never acquire another sealed cohort automatically and never expose a second
candidate to the current one.

## Milestone 5 — Finish the operator proof

For the terminal run:

- repeated `start` returns the same optimization run;
- repeated terminal `resume` changes nothing;
- `status` is fast and explains the outcome and next command;
- `report` states the hypothesis, actual data/training change, checkpoints,
  every suite metric and failed gate, budget use, retries, sealed use, decision,
  evidence limits, and next safe action;
- `provenance` reproduces the same row-free bundle fingerprint twice;
- `doctor` replays native files, the training snapshot, experiment/campaign/
  optimization journals, and sealed-exposure facts;
- a deliberate temporary tamper in a disposable copy is detected, then the
  clean authoritative evidence is reverified;
- the original Nomos repository is byte/status unchanged and the isolated copy
  remains clean, remote-free, and credential-free.

Update `docs/current-status.md`, `docs/encoder-optimize.md`, the production
experiment spec, migration/recovery/provenance docs if contracts changed, and
the checked-in example manifest. Do not leave completed work described as
future work.

## Engineering and execution rules

- Keep domain contracts independent from SQLite, CLI, Python, paths, Nomos,
  SDKs, and presentation.
- Keep native training/evaluation and row parsing inside the compiled Nomos
  adapter.
- Keep orchestration thin: reserve/link/advance; never copy slice business
  logic into the CLI.
- Derive status, decisions, budgets, and reports from persisted facts.
- Preserve append-only immutable history and compare-and-append journals.
- Use deterministic fakes for ordinary tests.
- Implement and commit coherent stages.
- After each stage run `cargo fmt-check`, `cargo check-all`, `cargo lint`, and
  `cargo test-all` with `CARGO_INCREMENTAL=0` where appropriate.
- Run relevant isolated Nomos Python tests and the opt-in real adapter integrity
  test before completion.
- Never reset or overwrite unrelated user changes.

## Explicit non-goals

Do not add GUI/TUI, HTTP endpoints, cloud execution, distributed workers,
authentication, multi-user support, arbitrary plugin loading, automatic web
research, a research/prompt-repair agent, semantic deduplication, ModernBERT
token relief, reinforcement learning, bandits, an endless loop, automatic gate
changes, automatic approval, automatic sealed acquisition, or broad data
generation.

Do not modify the original Nomos repository. Do not use an LLM as metric,
quality, candidate-selection, sealed-use, or promotion authority.

## Completion criteria

This goal is complete only when:

1. Fresh non-adopted optimize execution and recovery have deterministic
   application-level coverage for both terminal paths.
2. One conservative causal training hypothesis is documented and frozen before
   its results are observed.
3. All new inputs, parameters, artifacts, budgets, and approvals are immutable
   and provenance-linked.
4. At least one genuinely new candidate is trained by the high-level optimize
   run rather than linked from historical evidence.
5. Every new candidate has independent evidence for both development suites,
   and strict eligibility is enforced.
6. Sealed evidence is either demonstrably unused or used once only after exact
   explicit authorization.
7. A deterministic final decision, report, provenance bundle, and passing
   Doctor exist for the new run.
8. Idempotency, crash recovery, cancellation, staleness, migration, tamper, and
   budget behavior are verified in proportion to the changed contracts.
9. All Rust gates, relevant Python tests, isolated adapter verification, secret
   scan, and three-repository isolation audit pass.
10. Documentation and coherent commits leave a new Codex session with an exact,
    truthful handoff.

Finish with a management summary separating platform hardening, the causal
hypothesis, data changes, training changes, per-suite results, sealed result,
production-baseline decision, evidence limitations, exact commits, and the next
safe action.
