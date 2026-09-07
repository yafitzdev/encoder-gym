import type { CandidateAttempt, DevelopmentResult, RunRecord, WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { bytesLabel, candidateDescription, candidateName, candidateRows, dateLabel, developmentStatus, durationLabel, metricInfo, primaryMetric, resultSummary, runLabel, runName, score, delta, setupId, setupName, suiteName, timeLabel, type CandidateRow } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, scoreStack, sectionHeader, status, tabs, tag } from "./components.js";
import { h, type Child } from "./dom.js";

export interface DetailState { failedOnly: boolean }
function requirement(check: DevelopmentResult["checks"][number]): string {
  const c = check.condition;
  if (c.kind === "minimum_improvement") return `Improve by at least ${delta(c.value, check.metric)}`;
  if (c.kind === "maximum_regression") return c.value === 0 ? "No regression" : `Allow at most ${delta(c.value, check.metric).replace("+", "")} regression`;
  return `${c.kind.replaceAll("_", " ")}: ${score(c.value, check.metric)}`;
}
export function renderResults(row: CandidateRow, state: DetailState, actions: Actions): HTMLElement {
  const c = row.candidate;
  const allChecks = c.development.flatMap(d => d.checks);
  const failed = allChecks.filter(g => !g.passed).length;
  const s = developmentStatus(row);
  return h("div", {},
    h("div", { class: "result-banner" }, h("div", {}, status(s.label, s.tone), h("h2", {}, c.failure ? "This attempt could not complete" : failed ? `${failed} of ${allChecks.length} development checks failed` : s.key === "passed" ? "All development requirements met" : "Evidence is incomplete"), h("p", {}, c.failure ? c.failure.reason : s.key === "passed" ? resultSummary(row) + "." : "The recorded requirements apply to every development suite independently.")), tag(`${c.development.length} / ${row.run.baselines.length} suites`)),
    h("div", { class: "section-heading" }, h("h2", {}, "Baseline vs candidate"), h("label", { class: "checkbox-label" }, h("input", { id: "failed-only", type: "checkbox", checked: state.failedOnly, onChange: (e: Event) => { state.failedOnly = (e.target as HTMLInputElement).checked; actions.render(); } }), `Failed checks only (${failed})`)),
    ...row.run.baselines.map(base => {
      const d = c.development.find(d => d.baseline.id === base.id);
      if (!d) return h("section", { class: "evidence-section" }, sectionHeader(suiteName(base.suite), tag("Not evaluated")), h("p", { class: "section-note" }, "No development report was recorded for this candidate on this suite."));
      const checks = d.checks.filter(g => !state.failedOnly || !g.passed);
      return h("section", { class: "evidence-section" },
        sectionHeader(suiteName(base.suite), status(d.verdict === "passed" ? "Passed" : "Failed", d.verdict === "passed" ? "success" : "danger")),
        h("div", { class: "table-scroll", tabindex: "0", "aria-label": `${suiteName(base.suite)} recorded checks` }, h("table", { class: "evidence-table" },
          h("thead", {}, h("tr", {}, ...["Metric", "Baseline", "Candidate", "Change", "Requirement", "Check"].map((text, i) => h("th", { scope: "col", class: i > 0 && i < 4 ? "numeric" : "" }, text)))),
          h("tbody", {}, ...checks.map(g => h("tr", {},
            h("th", { scope: "row" }, h("button", { class: "inline-link metric-info", type: "button", onClick: () => actions.help(g.metric) }, metricInfo(g.metric).label)),
            h("td", { class: "numeric score" }, score(g.baseline, g.metric)), h("td", { class: "numeric score" }, score(g.candidate, g.metric)),
            h("td", { class: "numeric score " + (g.improvement === 0 ? "muted" : g.improvement > 0 ? "success" : "danger") }, delta(g.candidate - g.baseline, g.metric)),
            h("td", { class: "requirement" }, requirement(g)), h("td", {}, status(g.passed ? "Pass" : "Fail", g.passed ? "success" : "danger")),
          ))),
        )),
        !checks.length ? h("p", { class: "section-note" }, "No failed checks in this suite. Turn off the filter to inspect all checks.") : null,
        details("Report identities", facts([["Baseline report", copyField(base.fingerprint, actions.copy)], ["Candidate report", copyField(d.report.fingerprint, actions.copy)], ["Suite", copyField(base.suiteFingerprint, actions.copy)], ["Assessment", copyField(d.assessmentId, actions.copy)]])),
      );
    }),
    h("p", { class: "table-footnote" }, "Check outcomes come from the recorded evaluation contract. Displayed scores are rounded; a positive change does not guarantee a passing check."),
  );
}
function parameterLabel(key: string): string {
  const labels: Record<string, string> = { learning_rate: "Learning rate", specialist_weight: "Reference weight", reference_model: "Reference checkpoint", query_strategy: "Query representation", positive_strategy: "Positive representation", repair_snapshot_id: "Training snapshot", repair_snapshot_fingerprint: "Snapshot fingerprint", repair_total_rows: "Training examples", repair_base_rows: "Base examples", repair_delta_rows: "Repair examples" };
  return labels[key] ?? key.replaceAll("_", " ").replace(/^./, c => c.toUpperCase());
}
function trainingDetails(candidate: CandidateAttempt, run: RunRecord, actions: Actions): HTMLElement {
  return h("div", { class: "detail-columns" },
    h("section", {}, sectionHeader("Training configuration"), h("p", { class: "section-note" }, candidateDescription(candidate)), facts(Object.entries(candidate.parameters).filter(([k]) => !k.startsWith("repair_") || k === "repair_total_rows").map(([k, v]) => [parameterLabel(k), String(v)]))),
    h("section", {}, sectionHeader("Training data"), ...run.inputs.map(i => h("div", { class: "artifact-item" }, h("strong", {}, i.key.split(/[\\/]/).at(-1)), h("span", { class: "muted" }, bytesLabel(i.bytes)), details("Identity", copyField(i.fingerprint, actions.copy)))),
      candidate.parameters.repair_delta_rows ? h("div", { class: "artifact-item" }, h("strong", {}, "Approved repair delta"), h("span", { class: "muted" }, `${candidate.parameters.repair_delta_rows} examples`)) : null,
      candidate.parameters.repair_snapshot_id ? details("Immutable training snapshot", facts([["Snapshot", copyField(String(candidate.parameters.repair_snapshot_id), actions.copy)], ["Fingerprint", copyField(String(candidate.parameters.repair_snapshot_fingerprint), actions.copy)]])) : null),
  );
}
function candidateArtifact(candidate: CandidateAttempt, actions: Actions): HTMLElement {
  const m = candidate.model;
  return h("section", {}, sectionHeader("Candidate artifact"), m ? facts([["Format", m.format], ["Size", bytesLabel(m.bytes)], ["Build time", durationLabel(candidate.durationSeconds)], ["Artifact", copyField(m.key, actions.copy)], ["Model fingerprint", copyField(m.fingerprint, actions.copy)], ["Candidate ID", copyField(candidate.id, actions.copy)]]) : empty("No completed model recorded", "This attempt has not produced a completed artifact."));
}
export function renderCandidate(workspace: WorkspaceSnapshot, id: string, tab: string, state: DetailState, actions: Actions, runId?: string): HTMLElement {
  let row = candidateRows(workspace).find(r => r.candidate.id === id);
  const exactRun = runId ? workspace.runs.find(r => r.id === runId) : undefined;
  const exactCandidate = exactRun?.candidates.find(c => c.id === id);
  if (runId && (!exactRun || !exactCandidate)) return empty("Attempt not found", "This exact candidate attempt is no longer in the loaded records. No results from another run have been substituted.", button("Back to models", () => actions.navigate({ page: "models" })));
  if (row && exactRun && exactCandidate) row = { ...row, run: exactRun, candidate: exactCandidate };
  if (!row) return empty("Candidate not found", "This candidate is not in the loaded workspace.", button("Back to models", () => actions.navigate({ page: "models" })));
  const c = row.candidate;
  return h("div", { class: "page-content" },
    button("All models", () => actions.navigate({ page: "models" }), "back-link", "back"),
    pageHeader(candidateName(c), candidateDescription(c), button(`Open ${runLabel(row.run, workspace)}`, () => actions.navigate({ page: "run", id: row.run.id }), "secondary", "arrow")),
    h("div", { class: "detail-context" }, tag("Candidate"), h("span", {}, setupName(row.run)), h("span", {}, dateLabel(row.run.createdAt))),
    tabs([["results", "Results"], ["training", "Training & data"], ["artifact", "Model & history"]], tab, value => actions.navigate({ page: "candidate", id, tab: value, runId })),
    h("div", { id: "detail-panel", role: "tabpanel", "aria-labelledby": `tab-${tab}` },
      tab === "training" ? trainingDetails(c, row.run, actions) : tab === "artifact" ? h("div", {}, candidateArtifact(c, actions), sectionHeader("Run history"), h("p", { class: "section-note" }, "A candidate can be recovered in a later run. Its earlier attempt remains part of the record."), ...row.attempts.map(r => runListItem(r, workspace, actions))) : renderResults(row, state, actions)),
  );
}

export function runListItem(run: RunRecord, workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const failed = run.candidates.filter(c => c.failure).length;
  return h("button", { type: "button", class: "run-list-item", onClick: () => actions.navigate({ page: "run", id: run.id }) },
    h("span", { class: "run-number" }, runLabel(run, workspace)), h("span", { class: "run-list-name" }, h("strong", {}, runName(run)), h("small", {}, `${run.candidates.length} candidates · ${setupName(run)}`)),
    h("span", {}, status(failed ? "Execution errors" : run.decision === "retain_baseline" ? "Baseline kept" : run.decision === "promote_candidate" ? "Candidate promoted" : "No final decision", failed ? "danger" : "neutral")),
    h("time", {}, dateLabel(run.createdAt)),
  );
}
function eventLabel(kind: string): string {
  return ({ run_created: "Run created", candidate_training_started: "Model build started", candidate_training_completed: "Model artifact recorded", candidate_development_completed: "Development evaluation recorded", candidate_development_suite_completed: "Development suite recorded", development_selected: "Development selection completed", sealed_authorized: "Final acceptance authorized", sealed_started: "Final acceptance started", sealed_completed: "Final acceptance recorded", finalized: "Final decision recorded", candidate_failed: "Candidate execution failed" } as Record<string, string>)[kind] ?? kind.replaceAll("_", " ");
}
function runRecord(run: RunRecord, workspace: WorkspaceSnapshot, actions: Actions): HTMLElement {
  const command = run.optimizationId ? "synth encoder optimize report --help" : "synth experiment status --help";
  const usedTraining = run.candidates.filter(c => c.model).reduce((n, c) => n + (c.durationSeconds ?? 0), 0);
  return h("div", {}, h("div", { class: "detail-columns" },
    h("section", {}, sectionHeader("Recorded budget"), facts([["Candidate attempts started", `${new Set(run.activity.filter(e => e.kind === "candidate_training_started").map(e => e.candidateId)).size} / ${run.budget.candidates}`], ["Completed build time", `${durationLabel(usedTraining)} / ${durationLabel(run.budget.trainingSeconds)}`], ["Development reports completed", `${run.candidates.reduce((n, c) => n + c.development.length, 0)} / ${run.budget.development}`], ["Final acceptance reports completed", `${["passed", "failed"].includes(run.acceptance.state) ? 1 : 0} / ${run.budget.sealed}`]])),
    h("section", {}, sectionHeader("Frozen run identities"), facts([["Run", copyField(run.id, actions.copy)], ["Protocol", copyField(run.protocolId, actions.copy)], ["Project revision", copyField(run.revision, actions.copy)], ["Journal head", copyField(run.journalHead, actions.copy)], ...(run.optimizationId ? [["Optimization run", copyField(run.optimizationId, actions.copy)] as [string, Child]] : [])])),
  ), sectionHeader("Inspect with the CLI"), h("p", { class: "section-note" }, workspace.source === "recorded" ? "This is an archival example, not a connected workspace. Open its actual local project folder to inspect the original record." : "Check the command's adapter prerequisites, then use the recorded run ID and source database below. The current experiment CLI uses the Nomos adapter; folder registration does not add backend support. This button only copies help, never executes a command."), workspace.source === "local" ? h("div", {}, copyField(command, actions.copy), facts([["Source database", copyField(run.sourceDatabase, actions.copy)]])) : null);
}
export function renderRun(workspace: WorkspaceSnapshot, id: string, tab: string, actions: Actions): HTMLElement {
  const run = workspace.runs.find(r => r.id === id);
  if (!run) return empty("Run not found", "This run is not in the loaded workspace.", button("All runs", () => actions.navigate({ page: "runs" })));
  const failures = run.candidates.filter(c => c.failure);
  const usedSealed = ["passed", "failed"].includes(run.acceptance.state);
  const changeTab = (t: string) => actions.navigate({ page: "run", id, tab: t });
  return h("div", { class: "page-content" }, button("All runs", () => actions.navigate({ page: "runs" }), "back-link", "back"),
    pageHeader(runName(run), `${runLabel(run, workspace)} · ${dateLabel(run.createdAt)} · ${timeLabel(run.createdAt)}`, tag(run.decision === "retain_baseline" ? "Baseline kept" : run.decision ?? "In progress")),
    tabs([["overview", "Overview"], ["activity", "Activity"], ["record", "Budget & record"]], tab, changeTab),
    h("div", { id: "detail-panel", role: "tabpanel", "aria-labelledby": `tab-${tab}` },
      tab === "activity" ? h("ol", { class: "activity-list" }, ...run.activity.map(e => h("li", {}, h("span", { class: "activity-sequence" }, e.sequence), h("div", {}, h("strong", {}, eventLabel(e.kind)), e.candidateId ? h("span", {}, run.candidates.find(c => c.id === e.candidateId) ? candidateName(run.candidates.find(c => c.id === e.candidateId)!) : e.candidateId) : null), h("time", {}, timeLabel(e.at))))) : tab === "record" ? runRecord(run, workspace, actions) : h("div", {},
        h("div", { class: "run-summary" }, h("div", {}, h("div", { class: "eyebrow" }, "Recorded outcome"), h("h2", {}, run.decision === "retain_baseline" ? "The baseline was kept" : run.decision === "promote_candidate" ? "A candidate was promoted" : "No final decision recorded"),
          h("p", {}, failures.length ? "Execution errors were recorded. Open the candidate history for recovered attempts." : usedSealed ? "Development selected a candidate for final acceptance. The final assessment determined this outcome." : run.selectedCandidateId ? "A candidate was selected in development. Final acceptance has not completed." : run.decision ? "No candidate was selected for final acceptance. The sealed evaluation was not run." : "The run has not recorded a final decision. Inspect activity for its last persisted stage.")),
          h("div", { class: "run-summary-facts" }, h("strong", {}, String(run.candidates.length)), h("span", {}, "candidates"), h("strong", {}, String(run.baselines.length)), h("span", {}, "development suites"))),
        sectionHeader("Candidates in this run"),
        ...run.candidates.map(c => {
          const row = { candidate: c, run, attempts: [run] };
          const s = developmentStatus(row);
          return h("div", { class: "run-candidate" }, h("div", {}, h("h3", {}, candidateName(c)), h("p", {}, candidateDescription(c)), c.failure ? h("p", { class: "danger" }, c.failure.reason) : status(s.label, s.tone)), button("Inspect candidate", () => actions.navigate({ page: "candidate", id: c.id, runId: run.id }), "secondary", "arrow"));
        }),
        sectionHeader("What was evaluated"), h("p", { class: "section-note" }, setupName(run) + ". Each candidate is checked against the baseline on every suite in this run."),
        h("div", { class: "inline-group" }, ...run.baselines.map(b => tag(suiteName(b.suite)))),
        h("section", { class: "acceptance-note" }, h("div", {}, status(usedSealed ? "Final acceptance used" : run.acceptance.state === "unused" ? "Final acceptance not used" : "Final acceptance " + run.acceptance.state, "neutral")),
          h("p", {}, usedSealed ? `Recorded outcome: ${run.acceptance.state}. ${run.acceptance.failedMetrics.length ? "Failed checks: " + run.acceptance.failedMetrics.map(k => metricInfo(k).label).join(", ") + "." : ""}` : "No sealed scores are available for model comparison."),
          h("small", {}, "Final acceptance evidence is kept separate from development comparisons. Its historical use does not establish current benchmark availability.")),
      ),
    ),
  );
}

export function renderCompare(workspace: WorkspaceSnapshot, rows: CandidateRow[], actions: Actions): HTMLElement {
  if (!rows.length) return empty("Choose candidates to compare", "Select up to 3 models from one evaluation setup.", button("Choose models", () => actions.navigate({ page: "models" })));
  if (rows.some(row => setupId(row.run) !== setupId(rows[0]!.run))) return empty("Comparison setup changed", "These candidates no longer share one evaluation setup. Choose a compatible selection from Models.", button("Choose models", () => actions.navigate({ page: "models" })));
  const reference = rows[0]!.run;
  return h("div", { class: "page-content" }, button("All models", () => actions.navigate({ page: "models" }), "back-link", "back"),
    pageHeader("Compare models", `${rows.length} candidates against the baseline · ${setupName(reference)}`),
    ...reference.baselines.map(base => h("section", { class: "compare-matrix" }, sectionHeader(suiteName(base.suite)),
      h("div", { class: "table-scroll", tabindex: "0", "aria-label": "Side-by-side model comparison" }, h("table", { class: "comparison-matrix" },
        h("thead", {}, h("tr", {}, h("th", { scope: "col" }, "Metric"), h("th", { scope: "col", class: "baseline-column" }, "Baseline"), ...rows.map(row => h("th", { scope: "col" }, h("button", { type: "button", class: "candidate-link", onClick: () => actions.navigate({ page: "candidate", id: row.candidate.id }) }, candidateName(row.candidate)), h("small", {}, runLabel(row.run, workspace)))))),
        h("tbody", {},
          ...Object.keys(base.metrics).sort((a, b) => Number(b === primaryMetric(reference)) - Number(a === primaryMetric(reference)) || a.localeCompare(b)).map(k => h("tr", {},
            h("th", { scope: "row" }, h("button", { class: "inline-link metric-info", type: "button", onClick: () => actions.help(k) }, metricInfo(k).label)), h("td", { class: "baseline-column score" }, score(base.metrics[k], k)),
            ...rows.map(row => { const d = row.candidate.development.find(d => d.report.suite === base.suite); return h("td", {}, scoreStack(d?.report.metrics[k], d?.baseline.metrics[k], k, row.run.directions[k])); }),
          )),
          h("tr", { class: "matrix-result" }, h("th", { scope: "row" }, "Suite result"), h("td", { class: "baseline-column" }, "Reference"), ...rows.map(row => { const d = row.candidate.development.find(d => d.report.suite === base.suite); return h("td", {}, status(d?.verdict === "passed" ? "Passed" : d?.verdict === "failed" ? "Failed" : "Not evaluated", d?.verdict === "passed" ? "success" : d?.verdict === "failed" ? "danger" : "neutral")); })),
        ),
      )),
    )),
    h("p", { class: "table-footnote" }, "Only candidates from the same evaluation setup are shown together. Green changes indicate improvement, not promotion eligibility."),
  );
}
