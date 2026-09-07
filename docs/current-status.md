# Current project status and session handoff

Updated 2026-09-07 after completing the first real repair-training optimization
cycle and productizing it as `synth encoder optimize`.

## What Encoder Gym can do now

The repository contains independently usable Rust slices for synthetic data
generation, dataset management, training/checkpoints, evaluation, error
analysis, and deterministic optimization. It also contains bounded optional
capabilities for authenticity research, dataset architecture, dataset quality,
generation supervision, benchmark architecture/stewardship, project
preparation, and governed cross-slice execution.

For production encoders whose task is not global text classification, compiled
task adapters can now plug into provider-neutral experiment, repair, campaign,
and optimization contracts. The first adapter is the isolated Nomos retrieval
ranker. Native rows, Python, model layout, and task metrics remain adapter-owned;
Encoder Gym owns immutable authority, finite budgets, evidence roles, strict
gates, journals, recovery identities, decisions, and row-free provenance.

The new `synth encoder optimize` family provides strict manifest preview,
idempotent launch, one-stage resume, approval pauses, cancellation, status,
inspection, Doctor, provenance, and deterministic reporting. Lower-level
repair, experiment, benchmark-generation, and campaign commands remain usable.

## Real Nomos evidence

Repositories:

- Encoder Gym: `C:\Users\yanfi\PycharmProjects\fitz-traylm`
- read-only source Nomos: `C:\Users\yanfi\PycharmProjects\fitz-tool`
- isolated experiment: `C:\Users\yanfi\PycharmProjects\nomos-encoder-gym-experiment`
- experiment database: `encoder-gym-repair.sqlite` inside the isolated copy

Pinned isolated revision:
`4450ab3f1de8a1fc64bcbe5d77c67d0fb0f99af9`.

The approved logical training population contains 6,800 immutable base rows and
192 deterministic repair rows. The repair delta was fully task-valid and had
zero exact, normalized, source, group, or lineage contamination. Its immutable
training snapshot is `2602ef88-db56-4591-8a4c-2581c5613726`.

One genuine CPU triplet candidate was trained:

- candidate: `44b98240-6b63-49ca-a0c3-21bddba1d151`
- model fingerprint:
  `sha256:36289478c89d50b3de5e4a6d0da3557e49fe0280988e89e01c6d9e0002799171`
- one epoch, batch size 16, learning rate `5e-6`, margin `0.2`
- 6,992 triplets; recorded training duration 1,045 seconds

It failed both unchanged development contracts:

| Suite | Baseline MRR | Candidate MRR | Delta | Other failure |
| --- | ---: | ---: | ---: | --- |
| `generic_holdout` | 0.8956535714 | 0.8956273810 | -0.000026190476 | Recall@2 -0.005 |
| `retired_post_scaling` | 0.9664993081 | 0.8854648878 | -0.08103442021 | Recall@1/2/3 regressed |

Optimization run `2317e08b-5848-4773-9a9e-42499ee09815` therefore completed
with `retain_baseline`. No candidate was selected, the production baseline did
not change, and successor benchmark generation
`10cba5de-e501-4169-a603-27f75c2abd37` remains active with zero candidate
exposures.

Doctor passed native replay and complete campaign/journal verification. The
optimization head is
`sha256:37d6e0a16516da7a7edf79773b4e3a68dab84338eabbb4097523a9817e6ed691`;
the reproducible row-free bundle fingerprint is
`sha256:19d30e96ebe62da73836cd285364208fa57d20f9662f34bd95d5409e70850eb4`.

## Repository state

The completed experiment implementation at this historical handoff was:

- `598d302db81d5bc3669f0b290e6dc96c5c4ddb11` — productize finite encoder
  optimization;
- `118b3d7a279db5b5dd74faafc84db562539aa37d` — record the proven outcome.

The isolated Nomos HEAD is
`4450ab3f1de8a1fc64bcbe5d77c67d0fb0f99af9`, its worktree is clean, and it has
no remote. The original Nomos repository remains at
`14e0a1667431982ee00ee07108e7d82351fa28eb` with the exact pre-existing modified
and untracked file set recorded in `goal.md` history; it was not changed by the
experiment.

The user subsequently reopened the GUI phase. The tracked `ui/` desktop now
provides persistent independent project folders, a baseline/candidate comparison
home, model and run details, benchmark context, and project settings. Nomos is an
opt-in recorded example rather than startup identity. App folder metadata is
mutable; experiment evidence remains read-only. See `../ui/README.md` for launch,
verification, and the boundary between generic presentation and the existing
adapter-specific experiment CLI. No new optimization experiment was started
as part of this GUI phase.

## Verification completed

- `cargo fmt-check`
- `cargo check-all`
- `cargo lint`
- `cargo test-all`
- opt-in real isolated Nomos adapter integrity test
- Nomos encoder-gym Python tests: 5 passed
- repeated optimize `start` and terminal `resume` idempotency
- repeated provenance fingerprint reproduction
- real `encoder optimize report`, `provenance`, and `doctor`
- tracked-file credential scans and repository isolation checks

Ordinary live-provider smoke tests remain intentionally ignored and were not
needed. No network, paid model call, or sealed evaluation occurred.

## Start the next Codex session

Read, in order:

1. `AGENTS.md`
2. `docs/current-status.md`
3. `docs/platform-spec.md`
4. `docs/architecture.md`
5. `docs/development.md`
6. `docs/production-encoder-experiment-spec.md`
7. `docs/encoder-optimize.md`
8. `goal.md`

Then inspect Git status in all three repositories before making changes. Use
the next `goal.md` as the long-range Codex goal. The highest-value continuation
is to prove a genuinely new, non-adopted optimize execution with a conservative
training hypothesis—not to add another smart subsystem or weaken evidence
gates.
