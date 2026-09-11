import type { ModelArtifact as CatalogArtifact } from "../managed-workspace.js";
import type { ModelArtifact, WorkspaceSnapshot } from "../workspace.js";
import { candidateName, modelName, type CandidateRow } from "./catalog.js";

export interface InventoryModel {
  id: string; name: string; role: "Baseline" | "Candidate" | "Previous baseline";
  artifact: ModelArtifact; catalogArtifact?: CatalogArtifact; evidence?: CandidateRow;
}

export type BaselineAction = { kind: "promote"; runId: string } | { kind: "restore"; revisionId: string };

/** Only durable acceptance or immutable baseline history can offer a baseline action. */
export function baselineAction(workspace: WorkspaceSnapshot, model: InventoryModel): BaselineAction | undefined {
  const catalog = workspace.managed?.modelCatalog;
  const active = catalog?.baselineRevisions.find(revision => revision.id === catalog.activeBaselineRevisionId);
  if (model.role === "Candidate" && model.evidence?.run.optimizationId
    && model.evidence.run.decision === "promote_candidate"
    && model.evidence.run.selectedCandidateId === model.evidence.candidate.id
    && model.evidence.run.acceptance.state === "passed"
    && model.catalogArtifact?.parentModelId === active?.modelArtifactId) {
    return { kind: "promote", runId: model.evidence.run.optimizationId };
  }
  if (model.role === "Previous baseline") {
    const revision = [...(catalog?.baselineRevisions ?? [])].reverse()
      .find(candidate => candidate.modelArtifactId === model.id && candidate.id !== catalog?.activeBaselineRevisionId);
    if (revision) return { kind: "restore", revisionId: revision.id };
  }
  return undefined;
}

/** Artifact identity and producing run bind evidence; names and outcomes never do. */
export function modelInventory(workspace: WorkspaceSnapshot): InventoryModel[] {
  const catalog = workspace.managed?.modelCatalog;
  const evidence = (fingerprint: string, runId?: string): CandidateRow | undefined => {
    const attempts = workspace.runs.filter(run => !runId || run.id === runId)
      .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    for (const run of attempts) {
      const candidate = run.candidates.find(c => c.model?.fingerprint === fingerprint);
      if (candidate) return { candidate, run, attempts: attempts.filter(r => r.candidates.some(c => c.id === candidate.id)) };
    }
    return undefined;
  };
  if (catalog) {
    const active = catalog.baselineRevisions.find(r => r.id === catalog.activeBaselineRevisionId)?.modelArtifactId;
    return catalog.artifacts.map((artifact): InventoryModel => ({
      id: artifact.id, name: artifact.name,
      role: artifact.id === active ? "Baseline" : catalog.baselineRevisions.some(r => r.modelArtifactId === artifact.id) ? "Previous baseline" : "Candidate",
      artifact: { id: artifact.id, key: artifact.path, fingerprint: artifact.fingerprint, format: artifact.format, bytes: artifact.bytes },
      catalogArtifact: artifact,
      evidence: evidence(artifact.sourceModel?.fingerprint ?? artifact.fingerprint, artifact.producingRun?.id),
    })).sort((a, b) => Number(b.role === "Baseline") - Number(a.role === "Baseline") || (b.catalogArtifact?.createdAt ?? "").localeCompare(a.catalogArtifact?.createdAt ?? ""));
  }
  const models: InventoryModel[] = [{ id: workspace.baseline.id, name: modelName(workspace.baseline.key), role: "Baseline", artifact: workspace.baseline }];
  for (const run of workspace.runs) for (const candidate of run.candidates) {
    if (!candidate.model || models.some(m => m.artifact.fingerprint === candidate.model!.fingerprint)) continue;
    models.push({ id: candidate.model.id, name: candidateName(candidate), role: "Candidate", artifact: candidate.model, evidence: evidence(candidate.model.fingerprint) });
  }
  return models;
}

export function findModel(workspace: WorkspaceSnapshot, id: string): InventoryModel | undefined {
  return modelInventory(workspace).find(model => model.id === id || model.evidence?.candidate.id === id || model.catalogArtifact?.sourceModel?.id === id);
}
