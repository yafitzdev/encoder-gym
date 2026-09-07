# Encoder Gym interface contract

The project is the home of one baseline encoder and every candidate created
against it. Models is the default page. It answers: what is the baseline, how
does each candidate compare, and which result should I inspect?

## Information architecture

- Models: baseline identity, development comparison of all candidate models,
  search, evaluation-setup filtering, and explicit side-by-side comparison.
- Model detail: one candidate versus its own frozen baseline, the exact recorded
  checks, configuration, model identity, and links to its run attempts.
- Runs: chronological experiment records. Run detail owns the hypothesis,
  candidates, training inputs, progress, finite usage, and provenance.
- Benchmarks: the development evaluation setups used in these records. Explain
  which comparisons are meaningful and keep final acceptance separate.
- Project: workspace, data source, recorded baseline artifacts, and source
  revision. A reading guide explains baseline, candidate, run, and checks.

## Evidence contract

The Electron shell and typed bridge stay in place. Infrastructure reads local
experiment journals into a small presentation read model. The renderer never
opens SQLite or starts a trainer. A portable recorded snapshot supports the
browser preview and a workspace that is not connected. Source and capture time
are always visible. Refresh reads persisted facts; it does not run evaluation.

Every development report retains its own baseline report, suite fingerprint,
metric-contract fingerprint, recorded verdict, and recorded checks. Comparisons
are grouped by exact evaluation setup. Never rank candidates across different
setups, substitute another suite's score, or infer acceptance from a rounded
delta. An improvement can still fail a minimum-improvement requirement.

Candidate identity is independent of run identity. Recovered attempts retain
their run history while Models lists each candidate once. A missing report is
shown as missing. Completion of a run and acceptance of a candidate are separate.

Sealed metrics, rows, and predictions are excluded from the comparison read
model. Run history may show only recorded acceptance use, verdict, and failed
check names. Historical dates never imply current freshness or authorization.
The desktop's read-only connection is not a replacement for CLI Doctor.

## Presentation and behavior

Use a quiet graphite desktop with clear text contrast, compact navigation, and
one dominant comparison table. Baseline is a stable anchor. Color accompanies
words and indicates metric direction or a recorded result; it is not decoration.
Show plain labels with technical names available beside them or in help.

Search, filters, sorting, model/run links, comparison selection, tabs, clipboard
feedback, theme selection, back/forward, and workspace refresh must work.
Preserve comparison filters when opening details and returning. Empty results,
unavailable evidence, and connection failure have explicit recovery paths.

Verify the actual renderer at desktop and narrow widths, keyboard navigation,
both themes, and evidence isolation. Verify presentation logic with focused tests
for matching authority, incomplete evidence, recovered identities, and deltas.
Run the repository's required checks before committing each coherent stage.
