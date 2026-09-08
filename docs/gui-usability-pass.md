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

Pending implementation and verification. Baseline green checks above do not
establish completion of the usability goal.
