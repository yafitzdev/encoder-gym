# Active goal — Local-model onboarding and Gym-owned workspaces

Implementation delivered 2026-09-08: local New/Open, managed checkpoint copies,
dataset import/provenance, persistence/recovery, and real Nomos final-stage
backfill. Verification and exact onboarding records are in
`docs/nomos-managed-onboarding.md`; Hugging Face and automatic execution remain
outside this goal.

Implement the managed project workflow end to end, with Hugging Face deferred.
Encoder Gym creates a standardized project folder from a local checkpoint;
users reopen Gym projects rather than register arbitrary source repositories.
Preserve the architecture and persistent multi-project navigation. Provide
real baseline ownership, local dataset import and provenance, and honest
empty/setup states through the CLI and desktop UI.

Then onboard Nomos from `C:\Users\yanfi\PycharmProjects\fitz-tool` into a
separate Gym-managed folder and backfill the final-stage training inputs named
by its retained checkpoint. Preserve the source, native dataset structure,
immutable identities, and held-out evidence boundaries. Do not start training,
evaluation, downloads, or external calls. See
`docs/managed-workspaces-spec.md` for contracts and completion evidence.

Checkpoint before implementation, verify and commit coherent stages. The goal
is not complete until the actual Nomos workspace opens in the app and copied
artifacts/data have been verified against their sources.

## Previous presentation goal (retained context)

# Build Encoder Gym as a Clear, Multi-Project Desktop App

Make Encoder Gym easy to use, easy to understand, and sophisticated enough for
serious encoder development. Preserve the existing architecture while rebuilding
the presentation and navigation around the user's actual work.

This is an encoder-development application, not a Nomos dashboard. Nomos is one
encoder we optimized as a test experiment. There will be many different encoder
projects, and the application needs durable project "folders" for organizing them.

The main page **inside each project** must own that project's baseline encoder
and compare it with all of that project's candidates.

## Start from the current state

Read AGENTS.md, docs/platform-spec.md, docs/architecture.md,
docs/development.md, the specifications relevant to the surfaces being changed,
and the existing UI code before implementation. Inspect Git status and the
running application; do not assume earlier screenshots or summaries represent
the current product.

There is already partially implemented GUI work. Assess it honestly: preserve
useful behavior, improve what is unclear, and replace assumptions that conflict
with this goal. Do not reset, delete, or indiscriminately commit existing changes.

The previous goal in this file concerned running another native Nomos
optimization experiment. That is not this task. This goal does not authorize
training a model, starting an optimization run, consuming sealed evidence,
downloading models, or making paid/external calls.

## 1. Establish the product hierarchy

Separate application-level project organization from work inside one project.

- **Project collection:** create or register project folders, open them, switch
  between them, and find previously opened projects.
- **Project home:** the selected project's baseline and candidate comparisons.
- **Model detail:** one baseline or candidate, its artifact identity, how it was
  produced, and its evaluation evidence.
- **Run detail:** one experiment's configuration, execution, candidates,
  recorded outcome, budgets, and provenance.
- **Supporting project pages:** datasets, benchmarks/evaluations, and project
  configuration, where existing capabilities support useful real workflows.

Do not mechanically reproduce backend slice names as navigation. Give each
page a clear purpose and only expose controls with implemented behavior.
Determine whether a project tree, a project library, or a compact project
switcher best supports this hierarchy; do not build redundant navigation.

Before the next broad redesign, show the proposed page responsibilities and
project-navigation structure to the user and get their direction. The user is
actively steering this design. Respect corrections and requests to pause;
an earlier instruction to work autonomously does not override them.

## 2. Make projects real, independent containers

Each project needs a durable identity, a user-facing name, a local folder
association, and its own baseline, candidates, runs, datasets, and evaluation
context. Keep this organizational identity separate from immutable experiment
snapshots and run IDs.

- Persist the project collection and selected project across app restarts.
- Support multiple projects without replacing or forgetting the previous one
  whenever a different folder is opened.
- Support a genuinely empty project: no baseline yet, no candidates, and no
  runs is a valid state, not a connection error.
- Explain the next available setup action using existing platform contracts.
  Adding a project folder must not silently start training or evaluation.
- Handle missing or moved folders, duplicate registrations, unreadable data,
  and unsupported evidence with understandable recovery paths.
- Keep project switching isolated: no scores, selections, run links, filters,
  or asynchronous responses may leak from one project into another.
- If removing a project from the collection is implemented, distinguish that
  clearly from deleting files. Do not delete project contents implicitly.

Nomos may appear as an explicitly identified example or an opened local
project. Do not hard-code it into startup behavior, product identity, project
names, baseline labels, filesystem paths, or generic metric explanations.
Do not fabricate additional real projects to make the interface look populated.

## 3. Make the project home a useful comparison workspace

A user should immediately understand:

1. Which project am I in?
2. What is its baseline encoder?
3. Which candidates exist, and how do they compare with that baseline?
4. Which results are meaningful improvements, regressions, incomplete
   evaluations, or failures to meet the recorded requirements?
5. What should I open to understand a result?

Make the baseline a stable reference and the candidate comparison the dominant
content. Show all candidates through a usable collection, not only the most
recent run or one selected candidate. Candidate identity and run identity are
different; recovered attempts must retain their history without duplicating
models or borrowing evidence from another attempt.

Choose a small, meaningful set of summary metrics from the project's actual
task and evaluation contract. Explain technical terms where needed. Nomos
retrieval metrics must not become universal assumptions for every encoder.

Show candidate names, key changes relative to baseline, evaluation coverage,
and concise recorded outcomes. Provide useful search/filtering and explicit
side-by-side comparison where they help. Opening a candidate and returning
should preserve the user's context.

Do not mix incomparable results. Match the baseline artifact, benchmark
identity, metric contract, and relevant evaluation policy. If setups differ,
group or separate them and explain why. Missing evidence is not a zero score.
A positive delta is not proof that a gate passed or that a model was accepted.

## 4. Give detail pages distinct jobs

Project home answers **which model is worth inspecting**. Detail pages explain
**why a result occurred and what produced it**.

- Candidate results compare with the exact recorded baseline and explain
  failed requirements without forcing users to interpret raw journals.
- Training and dataset details expose the configuration and immutable input
  references that actually produced the candidate.
- Run pages own stage history, execution failures, finite budgets, and
  provenance. Completion and model acceptance are separate facts.
- Benchmark pages explain what is measured and which comparisons are valid.
- Technical identifiers, full metric tables, artifact paths, and inspection
  commands remain accessible without dominating the overview.

Do not fill the project home with equal-weight cards for budgets, hashes,
workflow stages, generic recommendations, and duplicate decision summaries.
Avoid unsupported recommendations or vague "next safe action" prose.

## 5. Design for comprehension, not decoration

Use a deliberate visual hierarchy, readable text, restrained color, consistent
spacing, and clear interaction cues. A new user should know where to look
without needing an annotated screenshot to navigate the product.

Keep baseline identity compact enough that candidate comparisons are visible
at ordinary desktop window sizes. Use progressive disclosure for complexity,
not tiny text or endless dashboard panels. Status must be understandable
without relying on color alone.

Every visible control must work or clearly explain why it is unavailable.
Cover first use, empty collections, loading, partial evidence, errors, and
recovery—not only a populated Nomos success path.

Verify keyboard access, focus behavior, navigation history, scrolling, narrow
windows, and every supported theme in the actual renderer. Preserve useful
desktop behavior and preferences. Do not add decorative charts, onboarding
screens, themes, or navigation layers simply to make the app look elaborate.

## 6. Preserve architecture and evidence boundaries

Keep the Electron shell, typed bridge, presentation, persistence adapters, and
core domain responsibilities separate. The GUI must consume authoritative
contracts rather than reimplement training, evaluation, gate decisions, or
optimization policy.

Project organization may persist its own non-secret presentation metadata.
Reading or refreshing experiment evidence must remain read-only. Preserve
immutable historical artifacts, provenance, finite budgets, explicit approvals,
and sealed-evidence isolation.

Do not expose sealed metrics, rows, predictions, or diagnostics as development
feedback. Any permitted archival acceptance summary must remain clearly
separate from candidate-development comparisons.

Distinguish live local evidence, historical recordings, and test fixtures.
Show source and freshness honestly; do not claim integrity verification,
deployment, promotion, or current authority merely because a record loaded.

Do not add cloud services, authentication, multi-user support, distributed
workers, autonomous optimizer agents, or a new backend framework for this GUI
task. If a real workflow needs a missing backend contract, identify that
boundary and obtain direction rather than implementing a misleading frontend
simulation or silently broadening the task.

## 7. Implement and verify in coherent stages

After the user has directed the hierarchy:

1. Record the starting state and make a scoped checkpoint before substantial
   implementation, respecting existing changes.
2. Implement project identity, folder organization, persistence, and empty
   states before polishing more single-project screens.
3. Build the baseline-centered project home and its real comparison behavior.
4. Align detail pages and navigation with the same hierarchy.
5. Exercise complete user journeys, fix the rough edges, and update
   documentation to match the actual application.

Commit coherent, working stages with clear messages. Never bundle unrelated
changes. Follow AGENTS.md validation requirements, including cargo fmt-check,
cargo check-all, cargo lint, and cargo test-all for implementation stages,
alongside the UI's type checks, build, and focused tests.

Use deterministic local fixtures for ordinary testing. Tests must not require
a Nomos checkout, model downloads, GPU, credentials, or external services.
Keep any deliberate real-workspace integration checks separately opt-in.

## Completion criteria

Completion must be demonstrated, not inferred from an attractive screenshot:

- At least two different encoder projects can be registered, switched between,
  and reopened after restart without losing their identities or mixing data.
- An empty project has a coherent setup state before any run exists.
- Nomos is an ordinary example project, not a special case in the app shell.
- Each project home identifies its baseline and exposes every candidate with
  honest, appropriately matched comparisons.
- Candidate and run detail pages explain results and preserve exact provenance.
- Project switching, navigation, search, filters, comparison selection, and
  recovery paths work with realistic populated, empty, missing, and failed data.
- The key information is readable at normal desktop sizes; keyboard behavior
  and supported narrow layouts/themes have been exercised in the real app.
- Existing architecture, historical evidence, and governance remain intact.
- Relevant automated checks pass, screenshots have been visually reviewed,
  documentation is current, and coherent implementation stages are committed.

Finish with a short explanation of the product hierarchy, what changed,
how to launch it, what was verified, commit references, and any remaining
limitations. Do not claim the goal is finished while required workflows are
still placeholders or only work for the Nomos fixture.
