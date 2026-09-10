import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions } from "./actions.js";
import { bytesLabel, candidateName, dateLabel, durationLabel, metricInfo, runLabel, score, suiteName } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tabs, tag } from "./components.js";
import { renderResults, trainingDetails, runListItem, type DetailState } from "./detail-pages.js";
import { h } from "./dom.js";
import { findModel, type InventoryModel } from "./model-inventory.js";

export function renderModel(workspace: WorkspaceSnapshot, id: string, tab: string, state: DetailState, actions: Actions, runId?: string): HTMLElement {
  let model: InventoryModel | undefined = findModel(workspace, id);
  const run = runId ? workspace.runs.find(r => r.id === runId) : undefined;
  const candidate = run?.candidates.find(c => c.id === id || c.model?.id === id || c.model?.id === model?.catalogArtifact?.sourceModel?.id);
  if (runId && (!run || !candidate?.model)) return empty("Model not found", button("All models", () => actions.backTo("models")));
  if (run && candidate?.model) model = {
    ...(model ?? { id: candidate.model.id, name: candidateName(candidate), role: "Candidate", artifact: candidate.model }),
    evidence: { candidate, run, attempts: [run] },
  };
  if (!model) return empty("Model not found", button("All models", () => actions.backTo("models")));
  const m = model, artifact = m.artifact, catalog = m.catalogArtifact, row = m.evidence;
  const fingerprint = catalog?.sourceModel?.fingerprint ?? artifact.fingerprint;
  const baselineReports = workspace.runs.filter(r => r.baseline.fingerprint === fingerprint).flatMap(r => r.baselines);
  if (m.role === "Baseline") baselineReports.push(...(workspace.baselineEvaluations ?? []));
  const reports = [...new Map(baselineReports.map(r => [r.id, r])).values()];
  const selectedTab = ["overview", "results", "training", "artifact"].includes(tab) ? tab : "overview";
  const parent = catalog?.parentModelId ? findModel(workspace, catalog.parentModelId) : row ? findModel(workspace, row.run.baseline.id) : undefined;
  const snapshotId = catalog?.trainingSnapshot?.id ?? row?.candidate.parameters.repair_snapshot_id;
  const snapshotRows = row?.candidate.parameters.repair_total_rows;
  const trainingData = workspace.managed?.modelDatasetLinks?.find(link => link.projectId === catalog?.projectId && link.modelId === catalog.id && link.modelFingerprint === catalog.fingerprint);
  const overview = h("div", { class: "detail-columns" },
    h("section", {}, sectionHeader("Model"), facts([
      ["Status", m.role], ["Format", artifact.format], ["Size", bytesLabel(artifact.bytes)],
      ["Created", catalog ? dateLabel(catalog.createdAt) : row ? dateLabel(row.run.createdAt) : "Not recorded"],
      ...(parent ? [["Starting model", button(parent.name, () => actions.navigate({ page: "model", id: parent.id }), "ghost small")]] as [string, HTMLElement][] : []),
      ...(row ? [["Run", button(runLabel(row.run, workspace), () => actions.navigate({ page: "run", id: row.run.id }), "ghost small")]] as [string, HTMLElement][] : []),
    ])),
    h("section", {}, sectionHeader("Training dataset"), trainingData ? h("div", {},
      h("p", {}, `${trainingData.inputs.reduce((total, input) => total + input.rows, 0).toLocaleString()} rows · Version ${trainingData.version.number}`),
      button("Inspect dataset", () => actions.navigate({ page: "dataset", id: trainingData.version.id }), "ghost small", "arrow")) : snapshotId ? h("div", {},
      h("p", {}, snapshotRows ? `${Number(snapshotRows).toLocaleString()} examples` : "Recorded training version"),
      button("Inspect training record", () => actions.navigate({ page: "model", id: m.id, tab: "training", runId }), "ghost small", "arrow")) : h("p", { class: "muted" }, "Training version not recorded")));
  const evaluations = row ? renderResults(row, state, actions) : reports.length ? h("div", {}, ...reports.map(report => h("section", { class: "evidence-section" },
    sectionHeader(suiteName(report.suite)), facts(Object.entries(report.metrics).map(([key, value]) => [metricInfo(key).label, score(value, key)])),
    details("Evaluation version", copyField(report.suiteFingerprint, actions.copy))))) : empty("No evaluations");
  const record = h("div", {}, facts([
    ["Artifact", copyField(artifact.key, actions.copy)], ["Fingerprint", copyField(artifact.fingerprint, actions.copy)], ["Model ID", copyField(m.id, actions.copy)],
    ...(snapshotId ? [["Training snapshot", copyField(String(snapshotId), actions.copy)]] as [string, HTMLElement][] : []),
    ...(trainingData ? [["Dataset version", copyField(trainingData.version.id, actions.copy)], ["Training provenance", trainingData.evidence.kind === "importedManifest" ? "Recorded final-stage inputs; ancestor training unknown" : "Completed training receipt"], ["Dataset link", copyField(trainingData.fingerprint, actions.copy)]] as [string, HTMLElement | string][] : []),
    ...(row ? [["Training time", durationLabel(row.candidate.durationSeconds)]] as [string, string][] : []),
  ]), row ? h("section", {}, sectionHeader("Run history"), ...row.attempts.map(r => runListItem(r, workspace, actions))) : null);
  return h("div", { class: "page-content detail-page model-view", "data-model-id": m.id },
    button("All models", () => actions.backTo("models"), "back-link", "back"),
    pageHeader(m.name, tag(m.role, m.role === "Baseline" ? "accent" : "neutral")),
    tabs([["overview", "Overview"], ["results", "Evaluation"], ["training", "Training"], ["artifact", "Details"]], selectedTab,
      value => actions.navigate({ page: "model", id: m.id, tab: value, runId })),
    h("div", { id: "detail-panel", role: "tabpanel", "aria-labelledby": `tab-${selectedTab}` },
      selectedTab === "overview" ? overview : selectedTab === "results" ? evaluations : selectedTab === "artifact" ? record : row ? trainingDetails(row.candidate, row.run, actions) : empty("Training configuration not recorded")));
}
