import type { ManagedLaunchPreview, ManagedReadiness, ManagedRunStatus, OptimizationManifestChoice, PreparedOptimizationChoice, ReadinessCheck, ReadinessState } from "../managed-control.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import { button, details, facts, pageHeader, sectionHeader, status, tag } from "./components.js";
import { h } from "./dom.js";

export interface OptimizationPageState {
  loading: boolean;
  error?: string;
  readiness?: ManagedReadiness;
  manifest?: OptimizationManifestChoice;
  prepared?: PreparedOptimizationChoice;
  run?: ManagedRunStatus;
}
export interface OptimizationPageActions {
  refresh(): void;
  prepare(): void;
  chooseManifest(): void;
  start(): void;
  resume(): void;
  authorizeSealed(): void;
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
function readinessList(readiness: ManagedReadiness, actions: OptimizationPageActions): HTMLElement {
  const required = readiness.report.checks.filter(check => check.required);
  const blocked = required.filter(check => check.state !== "ready");
  const groups = [...new Set(blocked.map(check => check.category))].map(category => blocked.filter(check => check.category === category));
  const ready = required.filter(check => check.state === "ready");
  return h("section", { class: "readiness-list", "aria-label": "Launch requirements" },
    groups.length ? sectionHeader("Do these next", tag(String(groups.length))) : sectionHeader("Required setup", tag("Complete", "accent")),
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
function runPanel(run: ManagedRunStatus, actions: OptimizationPageActions): HTMLElement {
  const terminal = ["completed", "cancelled", "failed"].includes(run.state);
  const next = run.next_command === "resume" ? button("Run next stage", actions.resume, "primary") : run.next_command === "authorize-sealed" ? button("Review sealed authorization", actions.authorizeSealed, "primary") : null;
  return h("section", { class: "run-control" }, sectionHeader("Reserved run", tag(run.state.replaceAll("_", " "))),
    facts([["Run", run.run_id], ["Stopped because", run.stopped_reason.replaceAll("_", " ")], ["Durable transitions", String(run.last_sequence)], ["Next safe action", run.next_command.replaceAll("-", " ")]]),
    run.failed_or_uncertain ? h("p", { class: "form-error", role: "alert" }, run.failed_or_uncertain) : null,
    h("div", { class: "inline-group" }, next, !terminal ? button("Cancel before next stage", actions.cancel, "secondary") : null));
}

export function renderOptimization(workspace: ManagedWorkspace, state: OptimizationPageState, actions: OptimizationPageActions): HTMLElement {
  const readiness = state.manifest?.readiness ?? state.readiness;
  const preview = state.prepared?.launchPreview ?? readiness?.launchPreview;
  const report = readiness?.report;
  const unfinished = report ? new Set(report.checks.filter(check => check.required && check.state !== "ready").map(check => check.category)).size : 0;
  const active = workspace.modelCatalog?.artifacts.find(model => model.id === workspace.modelCatalog?.baselineRevisions.find(revision => revision.id === workspace.modelCatalog?.activeBaselineRevisionId)?.modelArtifactId);
  return h("div", { class: "page-content optimization-page" },
    pageHeader("Start optimization", "Turn this project's reviewed scientific inputs into one finite, recoverable run.", button("Refresh checks", actions.refresh, "ghost", "refresh")),
    state.error ? h("section", { class: "operation-failure", role: "alert" }, h("strong", {}, "Could not inspect launch readiness"), h("p", {}, state.error)) : null,
    state.loading && !report ? h("div", { class: "workspace-progress", role: "status" }, "Checking persisted project state…") : null,
    state.loading && report ? h("div", { class: "workspace-progress", role: "status" }, "Replaying the approved native evidence and verifying its artifact tree… This one-time integrity step can take several minutes for a large encoder project.") : null,
    report ? h("section", { class: "launch-summary" },
      h("div", {}, h("div", { class: "eyebrow" }, "Launch status"), h("h2", {}, preview ? "Prepared for reservation" : report.runnable ? "Ready to reserve" : `${unfinished} setup ${unfinished === 1 ? "step" : "steps"} remain`), h("p", {}, preview ? "The owner workflow resolved the exact approved snapshot, suites, candidates, and budgets shown below." : report.runnable ? "Every required fact resolves against the active baseline. Reserving the run makes no external call." : "Complete these in order. Each step is derived from the workspace and scientific stores—not from what the screen happens to show.")),
      status(preview ? "Prepared" : labels[report.overall].label, preview ? "success" : labels[report.overall].tone)) : null,
    readiness?.optimizationAuthority && !preview ? h("section", { class: "launch-definition" }, sectionHeader("Current approved repair", tag("Reviewed", "accent")),
      h("p", { class: "section-note" }, readiness.optimizationAuthority.hypotheses.join(" ")),
      facts([["Candidates", String(readiness.optimizationAuthority.candidateCount)], ["Qualified repair rows", readiness.optimizationAuthority.deltaRows.toLocaleString()], ["Training ceiling", `${readiness.optimizationAuthority.budget.maximum_training_seconds.toLocaleString()} seconds`], ["External calls", String(readiness.optimizationAuthority.budget.maximum_external_calls)], ["Sealed uses", String(readiness.optimizationAuthority.budget.maximum_sealed_uses)], ["Authority expires", new Date(readiness.optimizationAuthority.validUntil).toLocaleString()]]),
      h("p", { class: "section-note" }, readiness.optimizationAuthority.trainingSnapshotId ? "Its immutable training snapshot already exists. Preparing resolves the final run definition." : "Preparing replays the approved native delta, freezes its logical training snapshot, and resolves the final run definition. It does not train, evaluate, expose sealed evidence, or contact a provider."),
      button(state.loading ? "Preparing…" : "Prepare approved run", actions.prepare, "primary", "arrow")) : null,
    readiness && !preview ? readinessList(readiness, actions) : null,
    preview ? h("section", { class: "launch-definition" }, sectionHeader("Exact run definition", tag(state.prepared?.name ?? state.manifest?.name ?? "Reviewed selection", "accent")),
      facts([["Active baseline", active?.name ?? workspace.manifest.name + " baseline"], ["Training snapshot", preview.trainingSnapshotId], ["Benchmark generation", preview.benchmarkGenerationId], ...budgetFacts(preview)]),
      h("p", { class: "section-note" }, "Sealed evidence remains unavailable to generation, training, development analysis, and the advisor. Its single use requires a later explicit authorization."),
      state.prepared ? h("p", { class: "section-note" }, state.prepared.createdTrainingSnapshot ? "The approved logical training snapshot was created during preparation." : "The existing approved logical training snapshot was reused exactly.") : null,
      state.run ? runPanel(state.run, actions) : h("div", { class: "launch-actions" }, button(preview.existingRun ? "Open existing run" : "Reserve optimization run", actions.start, "primary", "runs"), h("p", {}, preview.existingRun ? "This definition already owns a run; no duplicate will be created." : `This persists the immutable definition and run identity. Preparation authorized ${state.prepared?.externalCalls ?? 0} external calls; reserving does not execute them.`))) : null,
  );
}
