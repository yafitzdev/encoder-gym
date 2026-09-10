import type { ManagedLaunchPreview, ManagedOptimizationReport, ManagedReadiness, ManagedRunStatus, OptimizationManifestChoice, PreparedOptimizationChoice, ReadinessCheck, ReadinessState } from "../managed-control.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import { localDateTimeLabel } from "./catalog.js";
import { button, details, facts, pageHeader, sectionHeader, status, tag } from "./components.js";
import { h } from "./dom.js";

export interface OptimizationPageState {
  loading: boolean;
  executing?: string;
  operationStartedAt?: number;
  error?: string;
  errorTitle?: string;
  readiness?: ManagedReadiness;
  manifest?: OptimizationManifestChoice;
  prepared?: PreparedOptimizationChoice;
  run?: ManagedRunStatus;
  report?: ManagedOptimizationReport;
}
export interface OptimizationPageActions {
  refresh(): void;
  prepare(): void;
  chooseManifest(): void;
  start(): void;
  resume(): void;
  authorizeSealed(): void;
  loadReport(): void;
  promote(): void;
  cancel(): void;
  openSettings(): void;
  openData(): void;
  openRuns(): void;
  upgrade(): void;
}

const labels: Record<ReadinessState, { label: string; tone: "success" | "warning" | "danger" | "neutral" }> = {
  ready: { label: "Ready", tone: "success" },
  action_required: { label: "Action needed", tone: "warning" },
  blocked: { label: "Blocked", tone: "danger" },
  stale: { label: "Out of date", tone: "danger" },
  unavailable: { label: "Unavailable", tone: "neutral" },
};
function budgetFacts(preview: ManagedLaunchPreview): [string, string][] {
  const budget = preview.budget;
  return [
    ["Candidates", `${preview.candidateCount} / ${budget.maximum_candidates}`],
    ["Iterations", String(budget.maximum_iterations)],
    ["Training ceiling", `${budget.maximum_training_seconds.toLocaleString()} seconds`],
    ["Development evaluations", String(budget.maximum_development_evaluations)],
    ["Sealed evaluations", String(budget.maximum_sealed_evaluations)],
    ["Backend operations", String(budget.maximum_backend_operations)],
  ];
}
function launchMetric(label: string, value: string, _detail?: string): HTMLElement {
  return h("div", { class: "launch-metric" }, h("dt", {}, label), h("dd", {}, value));
}
function durationLabel(seconds: number): string {
  if (seconds === 0) return "0s";
  if (seconds % 3600 === 0) return `${seconds / 3600}h`;
  if (seconds % 60 === 0) return `${seconds / 60}m`;
  return `${seconds}s`;
}
function elapsedLabel(startedAt: number | undefined): string {
  if (startedAt === undefined) return "";
  const seconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  const minutes = Math.floor(seconds / 60);
  return minutes ? `${minutes}m ${seconds % 60}s` : `${seconds}s`;
}
function journalSpanLabel(createdAt: string, updatedAt: string): string {
  const seconds = Math.max(0, Math.floor((Date.parse(updatedAt) - Date.parse(createdAt)) / 1000));
  if (!Number.isFinite(seconds)) return "Unavailable";
  return seconds === 0 ? "Under 1s" : durationLabel(seconds);
}
function parameterLabel(value: string | number | boolean): string {
  return typeof value === "string" ? value : String(value);
}
function fieldLabel(value: string): string {
  return value.replaceAll("_", " ").replace(/^./, first => first.toUpperCase());
}
function reportMetricValue(value: number | undefined): string {
  return value === undefined ? "Not recorded" : value.toLocaleString(undefined, { maximumSignificantDigits: 7 });
}
function reportPanel(report: ManagedOptimizationReport): HTMLElement {
  const decision = report.decision?.replaceAll("_", " ") ?? "No final decision";
  return h("section", { class: "run-report", "aria-label": "Complete run report" },
    sectionHeader("Complete run report", tag("Aggregate evidence", "accent")),
    h("div", { class: "report-outcome" }, h("div", { class: "eyebrow" }, "Persisted decision"), h("h3", {}, decision),
      tag(report.sealed_evidence.used ? "Sealed evidence used" : "No sealed evidence")),
    h("div", { class: "report-summary" },
      facts([["Training population", `${report.training_data_change.total_rows.toLocaleString()} rows`], ["Approved change", `+${report.training_data_change.delta_rows.toLocaleString()} rows over ${report.training_data_change.base_rows.toLocaleString()} base`], ["Candidate hypotheses", String(report.approved_repair.candidate_hypotheses.length)], ["Recorded failures", String(report.budget_and_recovery.candidate_failure_events)]]),
      facts([["Training recorded", durationLabel(report.budget_and_recovery.observed_training_seconds)], ["Development reports", String(report.budget_and_recovery.observed_development_evaluations)], ["Sealed reports", String(report.budget_and_recovery.observed_sealed_evaluations)], ["Journal events", String(report.budget_and_recovery.optimization_event_count + report.budget_and_recovery.campaign_event_count + report.budget_and_recovery.experiment_event_count)]])),
    h("div", { class: "report-candidates" }, ...report.candidate_results.map((candidate, index) => h("section", { class: "report-candidate" },
      h("div", { class: "report-candidate-heading" }, h("div", {}, h("div", { class: "eyebrow" }, `Candidate ${index + 1}`), h("h3", {}, candidate.checkpoint?.key ?? candidate.candidate_id)), status(candidate.state.replaceAll("_", " "), candidate.state === "failed" ? "danger" : "neutral")),
      candidate.checkpoint ? facts([["Format", candidate.checkpoint.format], ["Artifact size", `${candidate.checkpoint.bytes.toLocaleString()} bytes`], ["Completed training", durationLabel(candidate.checkpoint.training_duration_seconds)], ["Checkpoint fingerprint", candidate.checkpoint.fingerprint]]) : tag("No checkpoint", "warning"),
      ...candidate.development_suites.map(suite => {
        const keys = [...new Set([...Object.keys(suite.baseline_metrics), ...Object.keys(suite.candidate_metrics)])];
        return details(`${suite.suite} · ${suite.verdict.replaceAll("_", " ")}`, h("div", { class: "report-suite" },
          facts(keys.map(key => [fieldLabel(key), `${reportMetricValue(suite.baseline_metrics[key])} baseline → ${reportMetricValue(suite.candidate_metrics[key])} candidate`] as [string, string])),
          facts([["Baseline report", suite.baseline_report_id], ["Candidate report", suite.candidate_report_id], ["Assessment", suite.assessment_id], ["Failed gates", String(suite.failed_gates.length)]])));
      })))),
    details("Inspect provenance chain", facts([
      ["Project", report.project.id], ["Project revision", report.project.revision], ["Baseline model", report.project.baseline_model.fingerprint],
      ["Diagnosis", report.diagnosis.fingerprint], ["Repair proposal", report.approved_repair.proposal_fingerprint], ["Delta selection", report.approved_repair.delta_selection_fingerprint],
      ["Training snapshot", report.training_data_change.snapshot_fingerprint], ["Benchmark generation", report.sealed_evidence.generation_id], ["Optimization journal", report.provenance_head],
    ])),
    h("div", { class: "evidence-limits" }, h("div", { class: "eyebrow" }, "Known evidence limits"), h("ul", {}, ...report.known_evidence_limits.map(limit => h("li", {}, limit)))));
}
function technicalRunDetails(preview: ManagedLaunchPreview, baseline: string): HTMLElement {
  return details("Technical details", h("div", { class: "readiness-details" },
    facts([
      ["Definition", preview.runName], ["Baseline", baseline], ["Training snapshot", preview.trainingSnapshotId],
      ["Benchmark generation", preview.benchmarkGenerationId], ["Final evaluation", preview.sealedSuite], ["Evaluation time ceiling", `${preview.maximumEvaluationSeconds.toLocaleString()} seconds`],
      ...budgetFacts(preview),
    ]),
    ...preview.candidateRecipes.map(recipe => h("section", { class: "candidate-recipe" },
      h("strong", {}, `Candidate ${recipe.sequence}`),
      facts([["Training ceiling", `${recipe.maximumTrainingSeconds.toLocaleString()} seconds`], ...Object.entries(recipe.parameters).map(([key, value]) => [fieldLabel(key), parameterLabel(value)] as [string, string])])))));
}
function actionFor(check: ReadinessCheck, actions: OptimizationPageActions): HTMLElement | null {
  const key = check.nextAction?.key;
  if (!key) return null;
  if (key === "upgrade-workspace") return button(check.nextAction!.label, actions.upgrade, "secondary");
  if (key === "import-dataset") return button(check.nextAction!.label, actions.openData, "secondary");
  if (key.startsWith("bind-") || key.startsWith("rebind-") || key.startsWith("repair-scientific") || key === "configure-providers" || key === "configure-provider-secret") return button("Open project settings", actions.openSettings, "secondary");
  if (key === "inspect-scientific-history") return button(check.nextAction!.label, actions.openRuns, "secondary");
  if (key === "prepare-optimization") return button("Review run", actions.prepare, "secondary");
  if (key === "resume-optimization") return button("Open existing run", actions.start, "secondary");
  return null;
}
const categoryCopy: Record<string, string> = {
  workspace: "Repair managed workspace",
  models: "Establish active baseline",
  scientific: "Connect scientific runtime",
  data: "Approve training input",
  evaluation: "Prepare evaluation",
  optimization: "Prepare run",
  recovery: "Resolve existing run",
};
const severity: Record<ReadinessState, number> = { ready: 0, action_required: 1, unavailable: 2, stale: 3, blocked: 4 };
function readinessList(readiness: ManagedReadiness, actions: OptimizationPageActions, handledKeys: ReadonlySet<string> = new Set()): HTMLElement {
  const required = readiness.report.checks.filter(check => check.required && !handledKeys.has(check.key));
  const blocked = required.filter(check => check.state !== "ready");
  const groups = [...new Set(blocked.map(check => check.category))].map(category => blocked.filter(check => check.category === category));
  const ready = required.filter(check => check.state === "ready");
  return h("section", { class: "readiness-list", "aria-label": "Launch requirements" },
    groups.length ? sectionHeader("Do these next", tag(String(groups.length))) : sectionHeader("Required foundations", tag("Complete", "accent")),
    ...groups.map(group => {
      const state = group.reduce((worst, check) => severity[check.state] > severity[worst] ? check.state : worst, group[0]!.state);
      const display = labels[state], copy = categoryCopy[group[0]!.category] ?? group[0]!.summary;
      const actionable = group.find(check => check.nextAction && actionFor(check, actions));
      return h("section", { class: `readiness-row readiness-${state}` },
        h("div", { class: "readiness-copy" }, h("div", { class: "readiness-heading" }, h("h3", {}, copy), status(display.label, display.tone)),
          details(`Inspect ${group.length} backend ${group.length === 1 ? "check" : "checks"}`, h("div", { class: "readiness-details" }, ...group.map(check => h("section", {}, h("strong", {}, check.summary), h("p", {}, check.evidence)))))),
        actionable ? actionFor(actionable, actions) : null);
    }),
    ready.length ? details(`${ready.length} required ${ready.length === 1 ? "foundation is" : "foundations are"} ready`, h("div", { class: "readiness-details" }, ...ready.map(check => h("section", {}, h("strong", {}, check.summary), h("p", {}, check.evidence))))) : null,
  );
}
function runPanel(run: ManagedRunStatus, state: OptimizationPageState, actions: OptimizationPageActions, activeAcceptedModel: boolean): HTMLElement {
  const terminal = ["completed", "cancelled", "failed"].includes(run.state);
  const busy = Boolean(state.executing);
  const accepted = run.state === "completed" && run.decision === "promote_candidate";
  const nextLabel = run.next_command === "authorize-sealed" ? "Authorize one sealed evaluation" : run.stage.label;
  const next = run.next_command === "resume" ? button(busy ? "Stage running…" : nextLabel, actions.resume, "primary") : run.next_command === "authorize-sealed" ? button(nextLabel, actions.authorizeSealed, "primary") : null;
  if (next instanceof HTMLButtonElement) next.disabled = busy;
  const promote = accepted && !activeAcceptedModel ? button(busy ? "Promoting…" : "Promote accepted candidate", actions.promote, "primary", "arrow") : null;
  if (promote instanceof HTMLButtonElement) promote.disabled = busy;
  const progress = run.stage.development;
  const recorded = run.recorded_usage;
  const reserved = run.budgets.reserved;
  const ceiling = run.budgets.maximum;
  const limitKind = reserved ? "reserved" : "ceiling";
  const incompleteAccounting = recorded.candidates_failed > 0 || Boolean(run.failed_or_uncertain);
  const finalDecisionCard = accepted || run.state === "completed" && run.decision === "retain_baseline";
  const inspectReport = terminal && !state.report ? button(state.executing === "report" ? "Loading report…" : "Inspect complete run report", actions.loadReport, "secondary") : null;
  const terminalReason = run.failed_or_uncertain && (run.state === "failed" || run.state === "cancelled") ? run.failed_or_uncertain : undefined;
  if (inspectReport instanceof HTMLButtonElement) inspectReport.disabled = busy;
  return h("section", { class: "run-control" }, sectionHeader(terminal ? "Run record" : "Reserved run", tag(run.state.replaceAll("_", " "))),
    !finalDecisionCard ? h("div", { class: `run-stage ${busy ? "is-running" : ""} ${terminalReason ? `is-${run.state}` : ""}`, role: busy ? "status" : run.state === "failed" ? "alert" : undefined },
      h("div", { class: "eyebrow" }, busy ? "Executing now" : run.state === "failed" ? "Recorded failure" : run.state === "cancelled" ? "Operator cancellation" : terminal ? "Final state" : "Next safe stage"),
      h("h3", {}, run.stage.label),
      terminalReason ? h("p", { class: "terminal-reason" }, terminalReason) : null,
      run.state === "failed" && terminalReason ? tag("No automatic continuation", "danger") : null,
      progress && progress.total_units > 0 ? tag(`${progress.completed_units} / ${progress.total_units} stages recorded`, "accent") : null,
      busy && run.stage.execution === "native" ? tag("Native stage running") : null) : null,
    next || !terminal && !busy ? h("div", { class: "run-actions" }, next, !terminal && !busy ? button("Cancel before next stage", actions.cancel, "secondary") : null) : null,
    accepted ? h("div", { class: `promotion-result ${activeAcceptedModel ? "is-active" : "is-pending"}` },
      h("div", {}, h("div", { class: "eyebrow" }, activeAcceptedModel ? "Baseline updated" : "Operator decision required"),
        h("h3", {}, activeAcceptedModel ? "Accepted candidate is now the baseline" : "Candidate passed final acceptance")),
      promote) : run.state === "completed" && run.decision === "retain_baseline" ? h("div", { class: "promotion-result is-retained" }, h("div", {}, h("div", { class: "eyebrow" }, "Final decision"), h("h3", {}, "Baseline retained"))) : null,
    inspectReport ? h("div", { class: "report-action" }, inspectReport) : null,
    h("section", { class: "run-usage", "aria-label": "Recorded work" },
      h("div", { class: "run-usage-heading" }, h("div", {}, h("div", { class: "eyebrow" }, "Recorded work"))),
      h("dl", { class: "run-metrics" },
        launchMetric("Models trained", `${recorded.models_trained} / ${reserved?.candidates ?? ceiling.maximum_candidates}`, `completed / ${limitKind}`),
        launchMetric("Training time", `${durationLabel(recorded.training_seconds)} / ${durationLabel(reserved?.training_seconds ?? ceiling.maximum_training_seconds)}`, `completed outputs / ${limitKind}`),
        launchMetric("Development", `${recorded.development_evaluations} / ${reserved?.development_evaluations ?? ceiling.maximum_development_evaluations}`, `reports / ${limitKind}`),
        launchMetric("Sealed", `${recorded.sealed_evaluations} / ${reserved?.sealed_evaluations ?? ceiling.maximum_sealed_evaluations}`, `reports / ${limitKind}`)),
      incompleteAccounting ? h("p", { class: "usage-caution" }, "Completed-output totals exclude work that ended without a durable completion record. Inspect the failure before treating the difference as unused capacity.") : null,
      details("Inspect reservation ledger", facts([
        ["Reserved iterations", reserved ? `${reserved.iterations} / ${ceiling.maximum_iterations}` : `Not attached / ${ceiling.maximum_iterations}`],
        ["Reserved backend operations", reserved ? `${reserved.backend_operations} / ${ceiling.maximum_backend_operations}` : `Not attached / ${ceiling.maximum_backend_operations}`],
        ["Unreserved candidates", String(run.budgets.remaining_unreserved?.candidates ?? ceiling.maximum_candidates)],
        ["Unreserved training time", `${(run.budgets.remaining_unreserved?.training_seconds ?? ceiling.maximum_training_seconds).toLocaleString()} seconds`],
        ["Failed candidate records", String(recorded.candidates_failed)],
      ]))),
    state.report ? reportPanel(state.report) : null,
    details("Run identity and journal", facts([["Run", run.run_id], ["Reserved at", localDateTimeLabel(run.created_at)], ["Last durable transition", localDateTimeLabel(run.last_transition_at)], ["Journal span", journalSpanLabel(run.created_at, run.last_transition_at)], ["Stopped because", run.stopped_reason.replaceAll("_", " ")], ["Durable transitions", String(run.last_sequence)], ["Journal head", run.head_fingerprint]])));
}

function launchSummary(state: OptimizationPageState, readiness: ManagedReadiness | undefined, preview: ManagedLaunchPreview | undefined, unfinished: number, activeAcceptedModel: boolean): { title: string; label: string; tone: "success" | "warning" | "danger" | "neutral" } {
  const run = state.run;
  if (run) {
    if (run.state === "failed") return { title: "Run needs attention", label: "Failed", tone: "danger" };
    if (run.state === "cancelled") return { title: "Run cancelled", label: "Cancelled", tone: "neutral" };
    if (run.state === "completed" && run.decision === "promote_candidate") return activeAcceptedModel
      ? { title: "Baseline updated", label: "Promoted", tone: "success" }
      : { title: "Candidate accepted", label: "Decision required", tone: "warning" };
    if (run.state === "completed" && run.decision === "retain_baseline") return { title: "Baseline retained", label: "Complete", tone: "neutral" };
    if (run.state === "completed") return { title: "Run completed", label: "Complete", tone: "success" };
    return { title: "Run paused", label: run.state === "planned" ? "Reserved" : "In progress", tone: "warning" };
  }
  const existing = preview?.existingRun;
  if (existing) return { title: "Existing run", label: "Run found", tone: "success" };
  if (preview) return { title: "Prepared", label: "Prepared", tone: "success" };
  if (readiness?.report.runnable) return { title: "Ready", label: "Ready", tone: "success" };
  return { title: `${unfinished} setup ${unfinished === 1 ? "step" : "steps"}`, label: readiness ? labels[readiness.report.overall].label : "Checking", tone: readiness ? labels[readiness.report.overall].tone : "neutral" };
}

export function renderOptimization(workspace: ManagedWorkspace, state: OptimizationPageState, actions: OptimizationPageActions): HTMLElement {
  const readiness = state.manifest?.readiness ?? state.readiness;
  const preview = state.prepared?.launchPreview ?? readiness?.launchPreview;
  const report = readiness?.report;
  const unfinished = report ? new Set(report.checks.filter(check => check.required && check.state !== "ready").map(check => check.category)).size : 0;
  const active = workspace.modelCatalog?.artifacts.find(model => model.id === workspace.modelCatalog?.baselineRevisions.find(revision => revision.id === workspace.modelCatalog?.activeBaselineRevisionId)?.modelArtifactId);
  const runBaseline = workspace.modelCatalog?.artifacts.find(model => model.id === workspace.modelCatalog?.baselineRevisions.find(revision => revision.id === workspace.scientificBinding?.baselineRevisionId)?.modelArtifactId);
  const activeAcceptedModel = Boolean(state.run && active?.producingRun?.id === state.run.artifacts.experiment_run_id);
  const authority = state.prepared?.authority ?? readiness?.optimizationAuthority;
  const summary = launchSummary(state, readiness, preview, unfinished, activeAcceptedModel);
  const reserveAction = preview && !state.run
    ? button(state.executing === "start" ? "Starting…" : preview.existingRun ? "Open run" : "Start run", actions.start, "primary", "runs")
    : null;
  if (reserveAction instanceof HTMLButtonElement) reserveAction.disabled = Boolean(state.executing);
  return h("div", { class: "page-content optimization-page" },
    pageHeader(state.run ? "Run" : "New run", button("Refresh", actions.refresh, "ghost", "refresh")),
    state.error ? h("section", { class: "operation-failure", role: "alert" }, h("strong", {}, state.errorTitle ?? "Could not inspect launch readiness"), h("p", {}, state.error)) : null,
    state.loading && !state.executing ? h("div", { class: "workspace-progress", role: "status" }, "Checking…") : null,
    state.executing === "prepare" ? h("div", { class: "workspace-progress", role: "status" }, "Preparing…") : null,
    state.executing && state.executing !== "prepare" && !state.run ? h("div", { class: "workspace-progress", role: "status" }, state.executing === "start" ? `Starting… ${elapsedLabel(state.operationStartedAt)}` : "Updating…") : null,
    report && (!preview || state.run) ? h("section", { class: "launch-summary" },
      h("div", {}, h("div", { class: "eyebrow" }, state.run ? "Run outcome" : "Launch status"), h("h2", {}, summary.title)),
      status(summary.label, summary.tone)) : null,
    state.run ? runPanel(state.run, state, actions, activeAcceptedModel) : null,
    readiness?.optimizationAuthority && !preview ? h("section", { class: "launch-definition" }, sectionHeader("Run", tag("Ready", "accent")),
      h("dl", { class: "launch-metrics", "aria-label": "Run limits" },
        launchMetric("Candidates", String(readiness.optimizationAuthority.candidateCount)),
        launchMetric("Training", durationLabel(readiness.optimizationAuthority.budget.maximum_training_seconds)),
        launchMetric("Training data", `${(readiness.optimizationAuthority.baseTrainingInputs + readiness.optimizationAuthority.deltaRows).toLocaleString()} rows`),
        launchMetric("API calls", String(readiness.optimizationAuthority.budget.maximum_external_calls))),
      h("div", { class: "launch-actions" }, button(state.loading ? "Preparing…" : "Review run", actions.prepare, "primary", "arrow"))) : null,
    readiness && !preview ? readinessList(readiness, actions, readiness.optimizationAuthority ? new Set(["optimization.preview"]) : new Set()) : null,
    preview ? h("section", { class: "launch-definition" }, sectionHeader("Run", tag("Ready", "accent")),
      h("dl", { class: "launch-metrics", "aria-label": "Run summary" },
        launchMetric("Candidates", String(preview.candidateCount)),
        launchMetric("Training", durationLabel(preview.budget.maximum_training_seconds)),
        launchMetric("Evaluations", String(preview.budget.maximum_development_evaluations)),
        launchMetric("API calls", String(authority?.budget.maximum_external_calls ?? 0))),
      h("div", { class: "launch-boundaries" },
        h("section", {}, h("div", { class: "eyebrow" }, "Baseline"), h("strong", {}, runBaseline?.name ?? active?.name ?? `${workspace.manifest.name} baseline`)),
        h("section", {}, h("div", { class: "eyebrow" }, "Training data"), h("strong", {}, authority ? `${(authority.baseTrainingInputs + authority.deltaRows).toLocaleString()} rows` : preview.trainingSnapshotId)),
        h("section", {}, h("div", { class: "eyebrow" }, "Evaluation"), h("strong", {}, preview.developmentSuites.join(" · ")))),
      reserveAction ? h("div", { class: "launch-actions" }, reserveAction) : null,
      h("div", { class: "launch-disclosures" }, technicalRunDetails(preview, runBaseline?.name ?? active?.name ?? `${workspace.manifest.name} baseline`))) : null,
  );
}
