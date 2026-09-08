# GUI usability pass

## Starting point

Goal: `goal.md`. Starting checkpoint: `91e8d2c` (clean worktree).
This is a presentation and existing-interaction pass, not a new execution phase.

Baseline verification on 2026-09-08: `npm run smoke` passed both legacy and
managed journeys and independent restarts. `npm run verify:current` verified
the real Nomos baseline and two datasets / 6,800 rows; saved library unchanged.
Before screenshots are retained locally in ignored `ui/qa/before-usability/`.

## Observations and design judgments

These findings come from code inspection, renderer interactions, and reviewed
screenshots, not a claim of user research or measured usability improvement.

1. Managed Models shows a large centered search-style empty state beneath a
   baseline card. It says to configure training/evaluation without making clear
   that this is not an available desktop action. The page needs a compact,
   explicit explanation of candidates and the actual project's available data.
2. Dataset cards repeat large provenance caveats. Counts, purpose, filenames,
   and deeper custody details compete. Use a scan-friendly collection with one
   shared preparation explanation and expandable per-file details.
3. The 760px New Project dialog scrolls content beneath sticky buttons, while
   progress/errors follow the buttons in document flow. Separate heading,
   scrollable fields, and a fixed feedback/action footer. Explain disabled
   confirmation and display the complete proposed destination before copying.
4. Errors surface raw IPC prefixes and filesystem diagnostics, including in
   temporary toasts. Use persistent, actionable messages with optional technical
   details. Never discard the cause or falsely classify an unknown failure.
5. Managed Runs implies CLI records will automatically appear, which is not
   implemented. Empty Runs/Benchmarks must state this boundary directly.
6. Settings leads with identifiers and repeats backend-policy paragraphs.
   Prioritize name/location and file verification; disclose technical metadata.
7. Project navigation rebuilds buttons without stable focus identity, and
   loading retains enabled old content during some operations. Verify focus,
   stale-response isolation, feedback and recovery as part of the shared shell.
8. Populated comparison/detail screens already have useful distinct contracts.
   Preserve exact comparisons while reducing introductory clutter and checking
   that returning from details restores the user's place.

## Page responsibilities

- Sidebar: persistent project library and scoped navigation; New and Open remain
  distinct. Nomos is never a shell default.
- Models: baseline, candidate collection, meaningful matched comparisons; for
  new projects, explain the current state and offer an implemented action.
- Datasets: imported files and their intended use, import, expandable custody
  details; imports are not approved training snapshots.
- Model details: exact identity, origin, and evaluation evidence.
- Runs / run details: recorded experiments / the history behind one outcome.
- Benchmarks: measurement and comparison boundaries.
- Settings: workspace management, verification, and non-destructive removal.

## Implementation stages

1. Managed project comprehension and onboarding: compact Models/Datasets,
   honest empty runs/evaluations, focused settings, accessible scroll-contained
   dialogs and recoverable errors. Preserve custody and all native contracts.
2. Shared interaction and evidence refinement: navigation/focus/context,
   persistent operation feedback, populated comparison/details, and recovery.
3. Acceptance audit: exercise generic multi-project and real read-only Nomos
   journeys, review screenshots at supported sizes/themes, address remaining
   high-impact issues, and update documentation.

Each implementation stage requires the UI checks and actual Electron acceptance,
the four AGENTS.md Rust gates, interface review, and a coherent commit.
Use temporary fixtures for writes. Real Nomos verification remains read-only.

## Completion evidence

### Stage 1 — Managed project comprehension and onboarding

Implemented compact baseline/candidate setup, scan-friendly dataset rows with
expandable provenance, one primary import action in empty/populated states,
plain managed-run/evaluation availability, and name/location-first settings.
New/import dialogs have independently scrolling fields and fixed feedback/actions.
Creation shows its complete destination and why confirmation is unavailable;
optional task metadata is disclosed. Known import errors lead with recovery,
preserve raw diagnostics on demand, and retain entered names on failed copies.
Success says "imported", not "ready".

Verification so far: 29 deterministic UI tests and both two-process Electron
journeys pass. Added checks cover fixed footer geometry at 760px, actions at
390px, cancelled-dialog focus, exact destination, held-out rejection, source
drift after preview, retained input, no failed-import publication, and accessible
dataset details. The final UI rerun after visual refinements passed as well.
Nomos read-only verification again reports the same baseline and 6,800 records,
with unchanged library bytes. Current Models/Datasets/Settings and dialog
screenshots were visually reviewed. `cargo fmt-check`, `cargo check-all`,
`cargo lint`, and full `cargo test-all` all passed (`RUST_TEST_THREADS=1`).

Interface review: no domain, persistence, native-file or IPC contracts changed.
The new pure error-description helper only maps existing diagnostics to display
copy; the backend still makes every validation and custody decision.

Stage 1 commit: `4d9a729`. Audit checkpoint: `e33aa59`.

### Stage 2 — Navigation, feedback, and evidence refinement

Project switching now resumes that project's page and scroll position. Clicking
the already-selected folder still returns to Models. All models / All runs links
restore collection position and the originating control across detail tabs.
Stable control identities preserve keyboard focus through renderer refreshes.
Navigation history remains renderer-owned, with independent instances per project.

Folder-operation failures persist above the current page until dismissed or
superseded, with actionable wording and collapsed exact diagnostics. Missing
managed projects retain their identity and recovery controls; their settings no
longer describe them as legacy journals. Loading has visible progress and inert
content, and folder operations, rename, and removal reject duplicate submission.
Verification now uses the existing revision-fenced selection reader: a delayed
response cannot overwrite a newer read after switching A → B → A.

Comparison filters have visible labels and disabled comparison explains the
selection requirement at narrow widths too. Model/run detail spacing is tighter;
the run header no longer duplicates its outcome. Benchmark copy describes exact
comparison groups concisely. Copy controls have field-specific accessible names.
Long project headings wrap without pushing actions or paths outside the page.

Final verification on 2026-09-08:

- `npm run check`: Rust CLI build, TypeScript, renderer build, all 33 Node tests.
- `npm run smoke`: both legacy and managed journeys and independent restarts
  passed in four actual Electron processes. Added regression checks cover
  collection scroll/focus, per-project route restoration, persistent Open errors,
  invalid-rename correction, delayed verification, and identity-safe recovery.
- New-dialog Enter, Tab order/trapping, and Escape restoration use Chromium
  keyboard input in hidden windows, without taking OS focus. A deliberately
  delayed real dataset import rejects Escape and a second submit; only one
  backend call occurs, then the normal custody operation completes.
- Creation, import error, and busy-import controls remain visible at 760×560.
  Fields scroll separately from feedback/actions. Long names and paths fit at
  760px and 390px. Standard page coverage also includes 1024, 1280, and 1440px.
- `cargo fmt-check`, `cargo check-all`, `cargo lint`, and full `cargo test-all`
  passed for this implementation stage, with `RUST_TEST_THREADS=1` for tests.
  No Rust code or dependency changes were made.
- `npm run verify:current` passed after the final renderer changes. Real Nomos
  retains project ID `0dd64b24-47d2-4cfb-9523-6c0b65dc4a46`, the same baseline
  fingerprint, two datasets / 6,800 records, and byte-identical saved library.

The keyboard harness initially used OS-focus-dependent input, which cannot work
in hidden windows; it now uses Chromium input with focus emulation. An earlier
reopen assertion clicked while the newly selected project was still loading;
the test now waits for that read. Neither issue required bypassing product guards
or weakening evidence assertions.

### Stage 3 — Final acceptance audit

Reviewed actual-renderer screenshots against the starting captures: managed
Models, Datasets, Settings, creation/import dialogs, failure/progress states,
populated legacy comparison and candidate/run details, benchmarks, narrow pages,
and long names. Removed repeated explanations and metadata from the initial scan
without hiding custody details. The minimum-size dialog review confirmed that
errors and busy feedback do not obscure the action footer. Light/dark collection
and welcome views remain readable. Representative token contrast checks exceed
4.5:1 for subtle text on raised/sidebar surfaces and for error text; this is not
a claim of a comprehensive accessibility certification.

| Goal acceptance area | Delivered evidence |
| --- | --- |
| New versus Open, baseline state, real next step | Distinct welcome/sidebar actions; real fixture checkpoint copies; managed Models links to the project's dataset page, not unsupported execution. |
| Generic multi-project isolation and restart | Two managed identities plus generic classification/similarity legacy fixtures; separate metrics, filters, selected candidates, routes, move/reopen and restart checks. |
| Dataset journey and readiness | Selection/preview, held-out rejection, source drift, retained input, fixed feedback, slow-operation guard, successful custody copy, source-vs-snapshot wording, no row payload in renderer. |
| Comparison and detail responsibilities | All 15 archival model identities, 26 checks, failed/recovered attempts, matched baseline/setup assertions, result versus acceptance distinction, separate model/run tabs, restored collection focus. |
| Empty/error/keyboard/window states | Empty and baseline-only fixtures, unsupported/corrupt reader tests, missing/wrong folder recovery, persistent errors, actual keyboard input, 760×560 dialogs, narrow layouts and both themes. |
| Nomos and architecture | Opt-in read-only verification above; no source-checkout writes. No scientific contracts, IPC/preload permissions, persistence schema, training, evaluation, network calls or downloads added. |
| Complete bounded pass | All eight audit priorities addressed; stages implemented and verified, not merely proposed. Working changes committed with this audit and updated UI documentation. |

Boundary review: the only main-process change passes the existing backend to the
test harness for delayed-response tests. Runtime native pickers, fixed CLI
adapter, typed bridge, domain ownership, exact comparison identities, and sealed
evidence projection remain unchanged. Tests use temporary fixtures; no historical
Nomos runs were attached to its managed identity. Generated QA images stay ignored.

Remaining product limits are explicit, not unfinished GUI actions: local
checkpoint custody supports the existing safe-format contract, not arbitrary
trainer compatibility; dataset preparation and scientific execution remain CLI
workflows; this desktop does not launch them or link their history automatically.
Hugging Face onboarding remains deferred. The OS file-picker shell is not driven
by acceptance tests (its chosen paths are injected). These findings are an
engineering/visual review, not measured novice-user research.
