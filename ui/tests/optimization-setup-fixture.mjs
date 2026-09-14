import { randomUUID } from "node:crypto";
export const fingerprint = "sha256:" + "a".repeat(64);
export function setupFixture() {
  const projectId = randomUUID();
  const model = { id: randomUUID(), name: "Baseline", fingerprint };
  const baseline = { id: randomUUID(), modelArtifactId: model.id, fingerprint };
  const version = { id: randomUUID(), datasetId: randomUUID(), projectId, number: 1, fingerprint };
  const benchmark = { id: randomUUID(), projectId, number: 1, fingerprint };
  const provider = role => ({ role, endpoint: "https://provider.example.test", model: `${role}-model` });
  const workspace = { folder: "owned-project", manifest: { id: projectId }, benchmarkVersions: [benchmark], providerCatalog: { id: randomUUID(), fingerprint, providers: [provider("advisor"), provider("generation")] },
    modelCatalog: { artifacts: [model], baselineRevisions: [baseline], activeBaselineRevisionId: baseline.id }, modelDatasetLinks: [{ modelId: model.id, version }] };
  const inputs = { projectId, baselineRevision: { id: baseline.id, fingerprint }, model: { id: model.id, fingerprint }, dataset: version, benchmark: { id: benchmark.id, fingerprint } };
  const datasets = [{ dataset: { id: version.datasetId, projectId, name: "Base" }, versions: [{ version, rows: 10 }] }];
  const preview = { expectedParent: null, inputs, modelName: model.name, datasetRows: 10, benchmarkNumber: 1 };
  const saved = request => ({ actionId: randomUUID(), setup: { id: request.id, number: 1, parent: request.expectedParent ? { id: request.expectedParent, fingerprint } : null, inputs: request.inputs, createdAt: new Date().toISOString(), fingerprint } });
  return { projectId, model, baseline, version, benchmark, workspace, inputs, datasets, preview, saved };
}
