# Goal — Make Encoder Gym clear, cohesive, and genuinely usable

Autonomously assess, redesign where necessary, and improve the existing desktop
GUI. Preserve the architecture and working capabilities, but take responsibility
for the presentation, information hierarchy, navigation, language, and interaction
quality. Deliver implemented, verified improvements—not just a design proposal.

The problem is not that the interface lacks styling. It can look good while still
feeling confusing: too many things compete for attention, backend terminology
obscures meaning, and users cannot tell where to look or what to do next. Optimize
for understanding and confident use, not a more elaborate dashboard.

## Product foundations already decided

Encoder Gym is a local, multi-project encoder-development application. Nomos is
one ordinary project, not the product's identity or a universal task template.

- Persistent project folders belong in the sidebar. Selecting one scopes its
  pages, models, datasets, runs, and settings to that project.
- New project creates a Gym-owned workspace from a local checkpoint. Open project
  reopens an existing Gym workspace. Arbitrary source repositories are not the
  primary onboarding model; legacy experiment folders remain explicitly distinct.
- Gym owns imported copies and records their provenance without altering sources.
- The project home identifies the baseline and supports inspecting candidates
  against it. Supporting pages and deeper model/run details have distinct jobs.
- Hugging Face onboarding remains deferred. This goal does not introduce model
  downloads, training/evaluation launch features, or new optimization capabilities.

Managed workspace creation, local dataset import, and Nomos onboarding have
already been implemented. Do not repeat them or replace them with mock flows.
Improve their usability and how the application explains their actual state.

## Work autonomously, within scope

Make ordinary design and implementation decisions yourself. Do not ask the user
to approve every layout, label, hierarchy refinement, or component. Investigate,
choose a coherent direction, implement it, and refine it after using the result.
Briefly explain material decisions and keep the user informed as work progresses.

You may substantially change existing screens where that improves comprehension.
Do not redesign for novelty, introduce redundant navigation, or add settings and
features simply to fill space. Preserve useful behavior and user preferences.

Ask only when a decision requires new authority, risks user data, or materially
changes the agreed product scope. Respect subsequent user steering. Autonomous
design work does not authorize autonomous agents or execution inside the product.

## 1. Establish the current reality

Read AGENTS.md, docs/platform-spec.md, docs/architecture.md, docs/development.md,
docs/managed-workspaces-spec.md, docs/managed-workspaces.md, ui/README.md, and the
specifications relevant to any affected slice before implementation. Read
docs/nomos-managed-onboarding.md for the real workspace's provenance and limits.

Inspect Git status, the current implementation, and the running application.
Earlier screenshots and summaries are context, not proof of today's behavior.
Record the starting commit and preserve unrelated work. Make a scoped checkpoint
before substantial changes where needed; do not indiscriminately commit a dirty
worktree or manufacture an empty checkpoint when the starting state is committed.

Walk through first use, project creation/opening, switching projects, dataset
import, model comparison, details, and recovery. Inspect both managed projects
and existing legacy evidence. Identify the highest-impact points of confusion.

Keep a concise audit and implementation plan in docs/gui-usability-pass.md:
observed problems, page responsibilities, priorities, and intended improvements.
Use it to drive implementation, not as a substitute for implementation. Separate
observed friction and design judgments from claims of user-tested usability.

## 2. Give every page a clear job

Make it easy to answer: Where am I? What am I looking at? What matters here?
What can I actually do next? What should I open for more detail?

- Application-level navigation owns creating, opening, finding, and switching
  projects. Project identity must remain obvious without dominating the screen.
- Project home owns the baseline and candidate overview. A project with no
  candidates should explain its state and offer an available next step without
  presenting a failed experiment or a mostly empty dashboard.
- Model details explain one model's identity, origin, and recorded evaluation
  evidence. Candidate comparisons use the exact applicable baseline and setup.
- Datasets own imported data, its purpose and preparation state, and import
  actions. Detailed lineage and verification information stay accessible nearby.
- Runs own execution history and recorded outcomes; run details own stages,
  configuration, failures, budgets, and provenance.
- Benchmark/evaluation pages explain what was measured and which results can be
  compared. Project settings own configuration and workspace management.

Refine labels and layouts based on the actual workflows. Do not mechanically map
backend modules onto navigation or display unsupported destinations as promises.
Preserve context when opening details, going back, or switching projects.

## 3. Build a strong, restrained information hierarchy

Give each screen an unmistakable starting point and a clear primary task or
question. Establish consistent typography, spacing, alignment, density, control
behavior, and status language across the application.

Replace equal-weight card piles, repeated summaries, oversized empty states,
decorative progress bars, and vague policy prose where they obscure the work.
Prefer useful collections and concise explanations over dashboard decoration.
Use plain language first, with precise technical detail available on demand.

Keep model identity, meaningful comparisons, dataset summaries, and actionable
status easy to scan. Move long paths, hashes, raw configuration, and extended
provenance into clearly labelled details with useful copy actions. Do not hide
important warnings or make required information discoverable only through hover.
Color must reinforce meaning, not carry it alone.

Explain domain distinctions at the moment they matter, without repeating a
compliance essay on every page. In particular:

- Importing a checkpoint does not prove trainer compatibility or readiness.
- Importing a dataset does not create an approved training snapshot.
- No evidence, a zero score, a failed evaluation, and a rejected candidate are
  different states. Completing a run does not imply accepting its candidate.
- A positive metric delta alone does not establish an accepted improvement.

Keep comparison groups faithful to model, benchmark, metric, and policy identity.
Do not impose Nomos retrieval metrics on every encoder project.

## 4. Make complete interactions feel reliable

Improve the full journey, not just each screen in isolation. Forms and dialogs
must make required inputs, validation, consequences, progress, and completion
clear. Users should always know whether an operation is still working, succeeded,
failed, or needs input.

Every control must work or explain its unavailability. Prevent duplicate
submissions and stale async results from affecting a different project. Preserve
safe user input on recoverable failures. Provide concise actionable errors, with
raw diagnostic details separate from the main message.

Exercise empty, populated, loading, long-running, partial, unsupported, missing,
and failed states. Verify focus order and restoration, keyboard activation,
dialog scrolling, visible actions, readable contrast, long names and paths,
supported themes, and practical desktop window sizes including narrow layouts.
Do not require a maximized window to complete onboarding or import a dataset.

Removing a project from the library must remain distinct from deleting files.
Opening, refreshing, browsing, and verifying existing evidence must not silently
modify scientific artifacts or start execution.

## 5. Preserve architecture, facts, and custody

Keep Electron main, typed IPC/preload, renderer, adapters, persistence, and domain
logic separate. Reuse the established components and contracts. Small supporting
read-model or IPC changes are in scope when necessary for correct existing GUI
behavior; new scientific workflows or backend frameworks are not.

Derive metrics and state from persisted facts. Preserve immutable identities,
source custody, finite budgets, explicit approvals, and sealed-evidence isolation.
Do not expose raw dataset rows or sealed diagnostics through this usability pass.
Keep permitted archival summaries separate from development feedback.

Never fabricate candidates, results, readiness, recommendations, integrity checks,
or success to make a screen look complete. Keep live managed data, legacy
recordings, and test fixtures distinguishable. Do not attach historical Nomos
runs to its new managed identity without an explicitly defined import contract.

Use temporary fixtures for writes during testing. The real managed Nomos project
at C:\Users\yanfi\EncoderGym\Projects\Nomos is available for opt-in read-only
verification. Do not alter its artifacts or the source checkout at
C:\Users\yanfi\PycharmProjects\fitz-tool. Do not duplicate its onboarding.

Do not start training, evaluation, external calls, paid services, or downloads.
Do not add cloud deployment, authentication, multi-user collaboration, distributed
execution, or autonomous optimization. Explain missing capabilities honestly
instead of disguising them with frontend simulations.

## 6. Implement, verify, and commit in coherent stages

Prioritize the highest-impact usability problems and deliver a bounded, cohesive
pass. Improve shared hierarchy and interaction patterns, then apply them to the
key journeys, then use the actual renderer to find and fix remaining friction.
Do not stop after changing CSS or producing an audit.

Follow AGENTS.md validation requirements for implementation stages: cargo
fmt-check, cargo check-all, cargo lint, and cargo test-all, alongside the UI's
checks, build, and relevant tests. Use deterministic offline fixtures; ordinary
tests must not depend on Nomos, credentials, downloads, a GPU, or external APIs.

Exercise actual Electron interactions and restart persistence, not just mocked
render output. Add focused regression coverage for changed behavior. Review
before/after screenshots of representative states, including dialogs and narrow
windows; inspect them visually and iterate. Do not weaken evidence assertions
or bless confusing behavior merely to make tests pass.

Commit coherent, verified stages with clear messages. Update user-facing docs to
match the delivered behavior. Record the rationale and verification evidence in
docs/gui-usability-pass.md, keeping generated QA artifacts out of source control
unless the repository's conventions explicitly call for them.

## Done means

- A new user can distinguish New from Open, create or reopen a managed project,
  understand its baseline and dataset state, and identify a real next action.
- At least two generic projects can be switched and reopened after restart
  without mixed evidence, stale state, lost identity, or dependence on Nomos.
- Local dataset import is understandable from selection through preview,
  confirmation, progress, result, and recovery; custody is not confused with
  training readiness.
- Populated model comparisons and model/run details have distinct purposes,
  explain recorded outcomes accurately, and preserve navigation context.
- Empty and error states are useful; primary actions remain reachable; key
  journeys work with keyboard input and supported window sizes/themes.
- The real Nomos workspace remains intact and displays correctly in read-only
  verification. Architecture and evidence boundaries remain intact.
- The selected pass's high-impact usability issues are resolved, relevant
  checks pass, actual-renderer QA is complete, and working stages are committed.

Finish with a concise account of what became easier, the important design
decisions, how to launch the app, verification results, commit references, and
any genuine limitations. Distinguish unimplemented backend capabilities from
unfinished GUI work. Do not claim completion based on visual polish alone.
