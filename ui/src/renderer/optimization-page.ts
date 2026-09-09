import type { ManagedLaunchPreview, ManagedOptimizationReport, ManagedReadiness, ManagedRunStatus, OptimizationManifestChoice, PreparedOptimizationChoice, ReadinessCheck, ReadinessState } from "../managed-control.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import { localDateTimeLabel } from "./catalog.js";
import { button, details, facts, pageHeader, sectionHeader, status, tag } from "./components.js";
import { h } from "./dom.js";

export interface OptimizationPageState {
  loading: boolean;
  executing?: string;
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
function launchMetric(label: string, value: string, detail: string): HTMLElement {
  return h("div", { class: "launch-metric" }, h("dt", {}, label), h("dd", {}, value), h("span", {}, detail));
}
function durationLabel(seconds: number): string {
  if (seconds === 0) return "0s";
  if (seconds % 3600 === 0) return `${seconds / 3600}h`;
  if (seconds % 60 === 0) return `${seconds / 60}m`;
  return `${seconds}s`;
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
      h("p", {}, report.sealed_evidence.used ? "One authorized sealed result informed the terminal decision. Sealed rows and scores remain outside this report." : "No sealed result was used in this run.")),
    h("div", { class: "report-summary" },
      facts([["Training population", `${report.training_data_change.total_rows.toLocaleString()} rows`], ["Approved change", `+${report.training_data_change.delta_rows.toLocaleString()} rows over ${report.training_data_change.base_rows.toLocaleString()} base`], ["Candidate hypotheses", String(report.approved_repair.candidate_hypotheses.length)], ["Recorded failures", String(report.budget_and_recovery.candidate_failure_events)]]),
      facts([["Training recorded", durationLabel(report.budget_and_recovery.observed_training_seconds)], ["Development reports", String(report.budget_and_recovery.observed_development_evaluations)], ["Sealed reports", String(report.budget_and_recovery.observed_sealed_evaluations)], ["Journal events", String(report.budget_and_recovery.optimization_event_count + report.budget_and_recovery.campaign_event_count + report.budget_and_recovery.experiment_event_count)]])),
    h("div", { class: "report-candidates" }, ...report.candidate_results.map((candidate, index) => h("section", { class: "report-candidate" },
      h("div", { class: "report-candidate-heading" }, h("div", {}, h("div", { class: "eyebrow" }, `Candidate ${index + 1}`), h("h3", {}, candidate.checkpoint?.key ?? candidate.candidate_id)), status(candidate.state.replaceAll("_", " "), candidate.state === "failed" ? "danger" : "neutral")),
      candidate.checkpoint ? facts([["Format", candidate.checkpoint.format], ["Artifact size", `${candidate.checkpoint.bytes.toLocaleString()} bytes`], ["Completed training", durationLabel(candidate.checkpoint.training_duration_seconds)], ["Checkpoint fingerprint", candidate.checkpoint.fingerprint]]) : h("p", { class: "section-note" }, "No completed checkpoint was recorded for this candidate."),
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
function candidateRecipe(preview: ManagedLaunchPreview): HTMLElement {
  return details(`Inspect ${preview.candidateRecipes.length === 1 ? "the candidate recipe" : `${preview.candidateRecipes.length} candidate recipes`}`,
    h("div", { class: "readiness-details" }, ...preview.candidateRecipes.map(recipe => h("section", { class: "candidate-recipe" },
      h("strong", {}, `Candidate ${recipe.sequence}`),
      facts([["Training ceiling", `${recipe.maximumTrainingSeconds.toLocaleString()} seconds`], ...Object.entries(recipe.parameters).map(([key, value]) => [fieldLabel(key), parameterLabel(value)] as [string, string])])))));
}
function externalWork(workspace: ManagedWorkspace, state: OptimizationPageState, readiness: ManagedReadiness | undefined): string {
  const authorizedCalls = state.prepared?.externalCalls ?? readiness?.optimizationAuthority?.budget.maximum_external_calls;
  if (authorizedCalls === 0) return "No external provider calls are authorized for this run. Reserving it will not read a provider credential.";
  if (authorizedCalls === undefined) return "No provider operation is part of reservation. Any external prerequisite must already be approved and frozen by its owning workflow.";
  const configured = workspace.providerCatalog?.providers.map(provider => `${fieldLabel(provider.role)}: ${provider.model}`).join(" · ");
  return `The frozen repair authority records a ceiling of ${authorizedCalls} external ${authorizedCalls === 1 ? "call" : "calls"}; reservation does not execute them.${configured ? ` Configured providers: ${configured}.` : " Required provider configuration must be completed before any external work."}`;
}
function reservationCopy(state: OptimizationPageState, existing: boolean): string {
  if (existing) return "This definition already owns a run; no duplicate will be created.";
  const externalCalls = state.prepared?.externalCalls;
  return externalCalls === undefined
    ? "This persists the immutable definition and one run identity. Reservation does not train, evaluate, or contact a provider."
    : `This persists the immutable definition and one run identity. The frozen authority records ${externalCalls} external ${externalCalls === 1 ? "call" : "calls"}; reservation does not execute them.`;
}
function actionFor(check: ReadinessCheck, actions: OptimizationPageActions): HTMLElement | null {
  const key = check.nextAction?.key;
  if (!key) return null;
  if (key === "upgrade-workspace") return button(check.nextAction!.label, actions.upgrade, "secondary");
  if (key === "import-dataset") return button(check.nextAction!.label, actions.openData, "secondary");
  if (key.startsWith("bind-") || key.startsWith("rebind-") || key.startsWith("repair-scientific") || key === "configure-providers") return button("Open project settings", actions.openSettings, "secondary");
  if (key === "prepare-optimization") return button("Prepare approved run", actions.prepare, "secondary");
  if (key === "resume-optimization") return button("Open existing run", actions.start, "secondary");
  return null;
}
const categoryCopy: Record<string, [string, string]> = {
  workspace: ["Repair the managed workspace", "Verify the files and project registry before relying on any scientific result."],
  models: ["Establish the active baseline", "A run must be tied to exactly one current immutable reference model."],
  scientific: ["Connect the scientific runtime", "Verify the task adapter, its exact project snapshot, and the contained scientific store."],
  data: ["Approve the training input", "Imported sources are custody only; training needs immutable scientific authority."],
  evaluation: ["Prepare development and sealed evaluation", "Both evidence roles must exist before candidates can be compared and accepted."],
  optimization: ["Prepare the reviewed run", "Freeze the approved snapshot and bind its benchmark generation, candidate space, and finite budgets."],
  recovery: ["Resolve the existing run", "Continue or replace the run already associated with this exact definition."],
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
      const display = labels[state], copy = categoryCopy[group[0]!.category] ?? [group[0]!.summary, group[0]!.evidence];
      const actionable = group.find(check => check.nextAction && actionFor(check, actions));
      return h("section", { class: `readiness-row readiness-${state}` },
        h("div", { class: "readiness-copy" }, h("div", { class: "readiness-heading" }, h("h3", {}, copy[0]), status(display.label, display.tone)), h("p", {}, copy[1]),
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
      h("h3", {}, run.stage.label), h("p", {}, run.stage.detail),
      terminalReason ? h("p", { class: "terminal-reason" }, terminalReason) : null,
      run.state === "failed" && terminalReason ? h("p", { class: "terminal-guidance" }, "No further stage will execute automatically. Inspect the complete report and journal before deciding whether a successor run is safe.") : null,
      progress && progress.total_units > 0 ? h("p", { class: "stage-progress" }, `${progress.completed_units} of ${progress.total_units} candidate build and development-evaluation steps durably recorded`) : null,
      busy && run.stage.execution === "native" ? h("p", { class: "stage-caution" }, "This native stage can take a while. The screen is reading the journal as new facts are committed; it does not estimate unfinished work.") : null) : null,
    next || !terminal && !busy ? h("div", { class: "run-actions" }, next, !terminal && !busy ? button("Cancel before next stage", actions.cancel, "secondary") : null) : null,
    accepted ? h("div", { class: `promotion-result ${activeAcceptedModel ? "is-active" : "is-pending"}` },
      h("div", {}, h("div", { class: "eyebrow" }, activeAcceptedModel ? "Baseline updated" : "Operator decision required"),
        h("h3", {}, activeAcceptedModel ? "The accepted candidate is now the project baseline" : "The candidate passed final acceptance"),
        h("p", {}, activeAcceptedModel ? "Its checkpoint is in managed custody. Reconnect a scientific runtime for this new baseline before starting another run." : "The run proved this exact checkpoint passed the sealed gates. It remains a candidate until you explicitly promote it.")),
      promote) : run.state === "completed" && run.decision === "retain_baseline" ? h("div", { class: "promotion-result is-retained" }, h("div", {}, h("div", { class: "eyebrow" }, "Final decision"), h("h3", {}, "The project baseline was retained"), h("p", {}, "No candidate met the complete acceptance contract, so the baseline pointer did not change."))) : null,
    inspectReport ? h("div", { class: "report-action" }, inspectReport, h("p", {}, "Loads the persisted aggregate development evidence and provenance. It never reveals sealed rows or sealed scores.")) : null,
    h("section", { class: "run-usage", "aria-label": "Recorded work" },
      h("div", { class: "run-usage-heading" }, h("div", {}, h("div", { class: "eyebrow" }, "Recorded work"), h("p", {}, `Completed outputs compared with the ${limitKind} capacity. These are durable records, not a live utilization estimate.`))),
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

function launchSummary(state: OptimizationPageState, readiness: ManagedReadiness | undefined, preview: ManagedLaunchPreview | undefined, unfinished: number, activeAcceptedModel: boolean): { title: string; detail: string; label: string; tone: "success" | "warning" | "danger" | "neutral" } {
  const run = state.run;
  if (run) {
    if (run.state === "failed") return { title: "Run needs attention", detail: "The durable journal stopped on a failed or uncertain stage. Inspect the recorded diagnostic before deciding whether recovery is safe.", label: "Failed", tone: "danger" };
    if (run.state === "cancelled") return { title: "Run cancelled", detail: "No further stage will execute. Completed artifacts and journal transitions remain available for inspection.", label: "Cancelled", tone: "neutral" };
    if (run.state === "completed" && run.decision === "promote_candidate") return activeAcceptedModel
      ? { title: "Baseline updated", detail: "The sealed-accepted checkpoint is now the project's active baseline and the previous revision remains in immutable history.", label: "Promoted", tone: "success" }
      : { title: "Candidate accepted", detail: "The exact checkpoint passed final acceptance. It remains a candidate until you explicitly advance the project baseline.", label: "Decision required", tone: "warning" };
    if (run.state === "completed" && run.decision === "retain_baseline") return { title: "Baseline retained", detail: "No candidate satisfied the complete acceptance contract, so the active baseline did not change.", label: "Complete", tone: "neutral" };
    if (run.state === "completed") return { title: "Run completed", detail: "The run is terminal. Inspect its persisted decision and evidence before taking any model action.", label: "Complete", tone: "success" };
    return { title: "Run paused at a safe boundary", detail: "The last stage committed its durable facts. The run panel below shows the one action that can advance it.", label: run.state === "planned" ? "Reserved" : "In progress", tone: "warning" };
  }
  const existing = preview?.existingRun;
  if (existing) return { title: "Existing run recovered", detail: "This immutable definition already owns a run. Open it to inspect its persisted stage and next safe action.", label: "Run found", tone: "success" };
  if (preview) return { title: "Prepared for reservation", detail: "The owner workflow resolved the exact approved snapshot, suites, candidates, and budgets shown below.", label: "Prepared", tone: "success" };
  if (readiness?.report.runnable) return { title: "Ready to reserve", detail: "Every required fact resolves against the active baseline. Reserving the run makes no external call.", label: "Ready", tone: "success" };
  return { title: `${unfinished} setup ${unfinished === 1 ? "step" : "steps"} remain`, detail: "Complete these in order. Each step is derived from the workspace and scientific stores—not from what the screen happens to show.", label: readiness ? labels[readiness.report.overall].label : "Checking", tone: readiness ? labels[readiness.report.overall].tone : "neutral" };
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
  return h("div", { class: "page-content optimization-page" },
    pageHeader(state.run ? "Optimization run" : "Start optimization", state.run ? "Follow one finite run from reservation through evidence-backed baseline decision." : "Turn this project's reviewed scientific inputs into one finite, recoverable run.", button("Refresh checks", actions.refresh, "ghost", "refresh")),
    state.error ? h("section", { class: "operation-failure", role: "alert" }, h("strong", {}, state.errorTitle ?? "Could not inspect launch readiness"), h("p", {}, state.error)) : null,
    state.loading && !state.executing && !report ? h("div", { class: "workspace-progress", role: "status" }, "Checking persisted project state…") : null,
    state.loading && !state.executing && report ? h("div", { class: "workspace-progress", role: "status" }, "Replaying the approved native evidence and verifying its artifact tree… This one-time integrity step can take several minutes for a large encoder project.") : null,
    state.executing && !state.run ? h("div", { class: "workspace-progress", role: "status" }, state.executing === "start" ? "Reserving the immutable run…" : "Updating the durable run record…") : null,
    report ? h("section", { class: "launch-summary" },
      h("div", {}, h("div", { class: "eyebrow" }, state.run ? "Run outcome" : "Launch status"), h("h2", {}, summary.title), h("p", {}, summary.detail)),
      status(summary.label, summary.tone)) : null,
    state.run ? runPanel(state.run, state, actions, activeAcceptedModel) : null,
    readiness?.optimizationAuthority && !preview ? h("section", { class: "launch-definition" }, sectionHeader("Approved repair on record", tag("Recorded", "accent")),
      h("p", { class: "section-note" }, readiness.optimizationAuthority.hypotheses.join(" ")),
      facts([["Candidates", String(readiness.optimizationAuthority.candidateCount)], ["Qualified repair rows", readiness.optimizationAuthority.deltaRows.toLocaleString()], ["Training ceiling", `${readiness.optimizationAuthority.budget.maximum_training_seconds.toLocaleString()} seconds`], ["External calls", String(readiness.optimizationAuthority.budget.maximum_external_calls)], ["Sealed uses", String(readiness.optimizationAuthority.budget.maximum_sealed_uses)], ["Authority expires", localDateTimeLabel(readiness.optimizationAuthority.validUntil)]]),
      h("p", { class: "section-note" }, readiness.optimizationAuthority.trainingSnapshotId ? "Its immutable training snapshot is recorded. Prepare replays the complete scientific lineage and resolves the final run definition." : "Prepare replays the complete scientific lineage, freezes the approved native delta as a logical training snapshot, and resolves the final run definition. It does not train, evaluate, expose sealed evidence, or contact a provider."),
      button(state.loading ? "Preparing…" : "Prepare approved run", actions.prepare, "primary", "arrow")) : null,
    readiness && !preview ? readinessList(readiness, actions, readiness.optimizationAuthority ? new Set(["optimization.preview"]) : new Set()) : null,
    preview ? h("section", { class: "launch-definition" }, sectionHeader("Exact run definition", tag(state.prepared?.name ?? state.manifest?.name ?? "Reviewed selection", "accent")),
      h("div", { class: "launch-objective" }, h("div", { class: "eyebrow" }, "Reviewed objective"), h("h3", {}, preview.runName),
        authority?.hypotheses.length ? h("p", {}, authority.hypotheses.join(" ")) : h("p", {}, "The immutable definition references its reviewed proposal; no editable hypothesis is accepted at reservation.")),
      h("dl", { class: "launch-metrics", "aria-label": "Hard run limits" },
        launchMetric("Candidates", `${preview.candidateCount}`, `${preview.candidateRecipes.length} frozen ${preview.candidateRecipes.length === 1 ? "recipe" : "recipes"}`),
        launchMetric("Iterations", `${preview.budget.maximum_iterations}`, "finite campaign"),
        launchMetric("Training", durationLabel(preview.budget.maximum_training_seconds), `${preview.budget.maximum_training_seconds.toLocaleString()} seconds ceiling`),
        launchMetric("Development", `${preview.budget.maximum_development_evaluations}`, "evaluations maximum")),
      h("div", { class: "launch-boundaries" },
        h("section", {}, h("div", { class: "eyebrow" }, "Development evidence"), h("strong", {}, `${preview.developmentSuites.length} ${preview.developmentSuites.length === 1 ? "suite" : "suites"}`), h("p", {}, preview.developmentSuites.join(" · "))),
        h("section", {}, h("div", { class: "eyebrow" }, "Final acceptance"), h("strong", {}, preview.sealedSuite), h("p", {}, "One separately confirmed sealed use after development eligibility."))),
      h("div", { class: "launch-disclosures" }, candidateRecipe(preview),
        details("Inspect immutable identities and complete limits", facts([["Baseline", runBaseline?.name ?? active?.name ?? workspace.manifest.name + " baseline"], ["Training snapshot", preview.trainingSnapshotId], ["Benchmark generation", preview.benchmarkGenerationId], ["Evaluation time ceiling", `${preview.maximumEvaluationSeconds.toLocaleString()} seconds`], ...budgetFacts(preview)]))),
      h("div", { class: "external-work" }, h("div", { class: "eyebrow" }, "External work"), h("p", {}, externalWork(workspace, state, readiness))),
      state.prepared ? h("p", { class: "section-note" }, state.prepared.createdTrainingSnapshot ? "The approved logical training snapshot was created during preparation." : "The existing approved logical training snapshot was reused exactly.") : null,
      state.run ? null : h("div", { class: "launch-actions" }, button(preview.existingRun ? "Open existing run" : "Reserve optimization run", actions.start, "primary", "runs"), h("p", {}, reservationCopy(state, Boolean(preview.existingRun))))) : null,
  );
}
