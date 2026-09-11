import type { CandidateAttempt, DevelopmentResult, RunRecord, WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { bytesLabel, candidateName, candidateRows, dateLabel, developmentStatus, durationLabel, metricInfo, primaryMetric, runLabel, runName, score, delta, setupId, setupName, suiteName, timeLabel, type CandidateRow } from "./catalog.js";
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
    h("div", { class: "result-banner" }, h("div", {}, status(s.label, s.tone), h("h2", {}, c.failure ? "Attempt failed" : failed ? `${failed} / ${allChecks.length} checks failed` : s.key === "passed" ? "All checks passed" : "Incomplete evidence"), c.failure ? h("p", { class: "danger" }, c.failure.reason) : null), tag(`${c.development.length} / ${row.run.baselines.length} suites`)),
    h("div", { class: "section-heading" }, h("h2", {}, "Baseline vs candidate"), h("label", { class: "checkbox-label" }, h("input", { id: "failed-only", type: "checkbox", checked: state.failedOnly, onChange: (e: Event) => { state.failedOnly = (e.target as HTMLInputElement).checked; actions.render(); } }), `Failed checks only (${failed})`)),
    ...row.run.baselines.map(base => {
      const d = c.development.find(d => d.baseline.id === base.id);
      if (!d) return h("section", { class: "evidence-section" }, sectionHeader(suiteName(base.suite), tag("Not evaluated")));
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
        !checks.length ? empty("No failed checks") : null,
        details("Report identities", facts([["Baseline report", copyField(base.fingerprint, actions.copy)], ["Candidate report", copyField(d.report.fingerprint, actions.copy)], ["Suite", copyField(base.suiteFingerprint, actions.copy)], ["Assessment", copyField(d.assessmentId, actions.copy)]])),
      );
    }),
  );
}
function parameterLabel(key: string): string {
  const labels: Record<string, string> = { learning_rate: "Learning rate", specialist_weight: "Reference weight", reference_model: "Reference checkpoint", query_strategy: "Query representation", positive_strategy: "Positive representation", repair_snapshot_id: "Training snapshot", repair_snapshot_fingerprint: "Snapshot fingerprint", repair_total_rows: "Training examples", repair_base_rows: "Base examples", repair_delta_rows: "Repair examples" };
  return labels[key] ?? key.replaceAll("_", " ").replace(/^./, c => c.toUpperCase());
}
export function trainingDetails(candidate: CandidateAttempt, run: RunRecord, actions: Actions): HTMLElement {
  return h("div", { class: "detail-columns" },
    h("section", {}, sectionHeader("Training configuration"), facts(Object.entries(candidate.parameters).filter(([k]) => !k.startsWith("repair_") || k === "repair_total_rows").map(([k, v]) => [parameterLabel(k), String(v)]))),
    h("section", {}, sectionHeader("Training data"), ...run.inputs.map(i => h("div", { class: "artifact-item" }, h("strong", {}, i.key.split(/[\\/]/).at(-1)), h("span", { class: "muted" }, bytesLabel(i.bytes)), details("Identity", copyField(i.fingerprint, actions.copy)))),
      candidate.parameters.repair_delta_rows ? h("div", { class: "artifact-item" }, h("strong", {}, "Added training data"), h("span", { class: "muted" }, `${candidate.parameters.repair_delta_rows} examples`)) : null,
      candidate.parameters.repair_snapshot_id ? details("Immutable training snapshot", facts([["Snapshot", copyField(String(candidate.parameters.repair_snapshot_id), actions.copy)], ["Fingerprint", copyField(String(candidate.parameters.repair_snapshot_fingerprint), actions.copy)]])) : null),
  );
}

export function runListItem(run: RunRecord, workspace: WorkspaceSnapshot, actions: Actions, open?: () => void, label = runLabel(run, workspace)): HTMLElement {
  const failed = run.candidates.filter(c => c.failure).length;
  const row = h("button", { type: "button", id: "run-" + run.id, class: "run-list-item", onClick: () => actions.navigate({ page: "run", id: run.id }) },
    h("span", { class: "run-number" }, label), h("span", { class: "run-list-name" }, h("strong", {}, runName(run)), h("small", {}, `${run.candidates.length} ${run.candidates.length === 1 ? "candidate" : "candidates"} · ${setupName(run)}`)),
    h("span", {}, status(failed ? "Execution errors" : run.decision === "retain_baseline" ? "Baseline kept" : run.decision === "promote_candidate" ? "Candidate accepted" : "No final decision", failed ? "danger" : "neutral")),
    h("time", {}, dateLabel(run.createdAt)),
  );
  if (!open) return row;
  const continuation = button("Continue", open, "secondary", "arrow");
  continuation.dataset.optimizationLink = "true";
  return h("article", { class: "run-list-linked" }, row, continuation);
}
function eventLabel(kind: string): string {
  return ({ run_created: "Run created", candidate_training_started: "Model build started", candidate_training_completed: "Model artifact recorded", candidate_development_completed: "Development evaluation recorded", candidate_development_suite_completed: "Development suite recorded", development_selected: "Development selection completed", sealed_authorized: "Final acceptance authorized", sealed_started: "Final acceptance started", sealed_completed: "Final acceptance recorded", finalized: "Final decision recorded", candidate_failed: "Candidate execution failed" } as Record<string, string>)[kind] ?? kind.replaceAll("_", " ");
}
function runRecord(run: RunRecord, actions: Actions): HTMLElement {
  const usedTraining = run.candidates.filter(c => c.model).reduce((n, c) => n + (c.durationSeconds ?? 0), 0);
  return h("div", {}, h("div", { class: "detail-columns" },
    h("section", {}, sectionHeader("Recorded budget"), facts([["Candidate attempts started", `${new Set(run.activity.filter(e => e.kind === "candidate_training_started").map(e => e.candidateId)).size} / ${run.budget.candidates}`], ["Completed build time", `${durationLabel(usedTraining)} / ${durationLabel(run.budget.trainingSeconds)}`], ["Development reports completed", `${run.candidates.reduce((n, c) => n + c.development.length, 0)} / ${run.budget.development}`], ["Final acceptance reports completed", `${["passed", "failed"].includes(run.acceptance.state) ? 1 : 0} / ${run.budget.sealed}`]])),
    h("section", {}, sectionHeader("Frozen run identities"), facts([["Run", copyField(run.id, actions.copy)], ["Protocol", copyField(run.protocolId, actions.copy)], ["Project revision", copyField(run.revision, actions.copy)], ["Journal head", copyField(run.journalHead, actions.copy)], ...(run.optimizationId ? [["Optimization run", copyField(run.optimizationId, actions.copy)] as [string, Child]] : [])])),
  ));
}
function recordedRunProgress(run: RunRecord): HTMLElement {
  const trained = run.candidates.some(candidate => candidate.model || candidate.failure);
  const evaluated = run.candidates.some(candidate => candidate.development.length);
  const decided = !!run.decision;
  const complete = [true, trained, evaluated, decided];
  const active = complete.findIndex(done => !done);
  return h("ol", { class: "optimization-progress-steps recorded-run-steps", "aria-label": "Run progress" },
    ...["Created", "Training", "Evaluation", "Decision"].map((label, index) => h("li", { class: complete[index] ? "complete" : index === active ? "active" : "" }, label)));
}
export function renderRun(workspace: WorkspaceSnapshot, id: string, tab: string, actions: Actions): HTMLElement {
  const run = workspace.runs.find(r => r.id === id);
  if (!run) return empty("Run not found", button("All runs", () => actions.navigate({ page: "runs" })));
  const failures = run.candidates.filter(c => c.failure);
  const usedSealed = ["passed", "failed"].includes(run.acceptance.state);
  const changeTab = (t: string) => actions.navigate({ page: "run", id, tab: t });
  return h("div", { class: "page-content detail-page" }, button("All runs", () => actions.backTo("runs"), "back-link", "back"),
    pageHeader(runLabel(run, workspace), h("div", { class: "inline-group" }, tag(dateLabel(run.createdAt)), tag(timeLabel(run.createdAt)))),
    tabs([["overview", "Overview"], ["activity", "Activity"], ["record", "Budget & record"]], tab, changeTab),
    h("div", { id: "detail-panel", role: "tabpanel", "aria-labelledby": `tab-${tab}` },
      tab === "activity" ? h("ol", { class: "activity-list" }, ...run.activity.map(e => h("li", {}, h("span", { class: "activity-sequence" }, e.sequence), h("div", {}, h("strong", {}, eventLabel(e.kind)), e.candidateId ? h("span", {}, run.candidates.find(c => c.id === e.candidateId) ? candidateName(run.candidates.find(c => c.id === e.candidateId)!) : e.candidateId) : null), h("time", {}, timeLabel(e.at))))) : tab === "record" ? runRecord(run, actions) : h("div", {},
        h("div", { class: "run-summary" }, h("div", {}, h("div", { class: "eyebrow" }, "Outcome"), h("h2", {}, run.decision === "retain_baseline" ? "Baseline kept" : run.decision === "promote_candidate" ? "Candidate accepted" : "No final decision"),
          failures.length ? status("Execution errors", "danger") : status(usedSealed ? `Final acceptance ${run.acceptance.state}` : run.selectedCandidateId ? "Awaiting final acceptance" : "Development complete", "neutral")),
          h("div", { class: "run-summary-facts" }, h("strong", {}, String(run.candidates.length)), h("span", {}, "candidates"), h("strong", {}, String(run.baselines.length)), h("span", {}, "development suites"))),
        recordedRunProgress(run),
        sectionHeader("Candidates in this run"),
        ...run.candidates.map(c => {
          const row = { candidate: c, run, attempts: [run] };
          const s = developmentStatus(row);
          return h("div", { class: "run-candidate" }, h("div", {}, h("h3", {}, candidateName(c)), c.failure ? h("p", { class: "danger" }, c.failure.reason) : status(s.label, s.tone)), c.model ? button("Inspect model", () => actions.navigate({ page: "model", id: c.model!.id, runId: run.id }), "secondary", "arrow") : null);
        }),
        sectionHeader("Evaluation", tag(setupName(run))), h("div", { class: "inline-group" }, ...run.baselines.map(b => tag(suiteName(b.suite)))),
        h("section", { class: "acceptance-note" }, h("div", {}, status(usedSealed ? "Final acceptance used" : run.acceptance.state === "unused" ? "Final acceptance not used" : "Final acceptance " + run.acceptance.state, "neutral")),
          usedSealed && run.acceptance.failedMetrics.length ? h("div", { class: "inline-group" }, ...run.acceptance.failedMetrics.map(k => tag(metricInfo(k).label, "danger"))) : null),
      ),
    ),
  );
}

export function renderCompare(workspace: WorkspaceSnapshot, rows: CandidateRow[], actions: Actions): HTMLElement {
  if (!rows.length) return empty("No candidates selected", button("Choose models", () => actions.navigate({ page: "models" })));
  if (rows.some(row => setupId(row.run) !== setupId(rows[0]!.run))) return empty("Incompatible evaluation setups", button("Choose models", () => actions.navigate({ page: "models" })));
  const reference = rows[0]!.run;
  return h("div", { class: "page-content" }, button("All models", () => actions.backTo("models"), "back-link", "back"),
    pageHeader("Compare models", h("div", { class: "inline-group" }, tag(`${rows.length} candidates`), tag(setupName(reference)))),
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
  );
}
