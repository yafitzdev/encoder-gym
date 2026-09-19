import { randomUUID } from "node:crypto";
export const fingerprint = "sha256:" + "a".repeat(64);
// Deterministic test fixtures only; the application loads presets from the core CLI.
export function agentPresets() {
  const standard = { mode: "standard", objective: "", analysisProtocol: 3, generationCanary: "per_combination_semantic_v3", maximumIterations: 3, maximumAgentTurnsPerIteration: 8, generationConcurrency: 1, maximumRowChanges: 192,
    training: { device: "auto", maximumEpochs: 1, batchSize: 64, learningRateNanos: 3000, maximumSecondsPerIteration: 7200, maximumTrainingRows: null } };
  const quickTest = { ...structuredClone(standard), mode: "quick_test", maximumIterations: 1, maximumAgentTurnsPerIteration: 4, maximumRowChanges: 8 };
  Object.assign(quickTest.training, { batchSize: 8, maximumSecondsPerIteration: 120, maximumTrainingRows: 64 });
  return { standard, quickTest };
}
export function setupFixture() {
  const projectId = randomUUID();
  const model = { id: randomUUID(), name: "Baseline", fingerprint };
  const baseline = { id: randomUUID(), modelArtifactId: model.id, fingerprint, change: { kind: "initialization", source_fingerprint: fingerprint } };
  const version = { id: randomUUID(), datasetId: randomUUID(), projectId, number: 1, fingerprint };
  const benchmark = { id: randomUUID(), projectId, number: 1, fingerprint };
  const provider = role => ({ role, endpoint: "https://provider.example.test", model: `${role}-model`, limits: { maximumRequests: 100, maximumInputTokens: 100000, maximumOutputTokens: 10000, maximumCostMicrousd: 1000000 } });
  const workspace = { folder: "owned-project", manifest: { id: projectId }, benchmarkVersions: [benchmark], providerCatalog: { id: randomUUID(), fingerprint, providers: [provider("advisor"), provider("generation")] },
    modelCatalog: { artifacts: [model], baselineRevisions: [baseline], activeBaselineRevisionId: baseline.id }, modelDatasetLinks: [{ modelId: model.id, version }] };
  const inputs = { projectId, baselineRevision: { id: baseline.id, fingerprint }, model: { id: model.id, fingerprint }, dataset: version, benchmark: { id: benchmark.id, fingerprint } };
  const datasets = [{ dataset: { id: version.datasetId, projectId, name: "Base" }, versions: [{ version, rows: 10 }] }];
  const preview = { expectedParent: null, inputs, modelName: model.name, datasetRows: 10, benchmarkNumber: 1 };
  const saved = request => ({ actionId: randomUUID(), setup: { id: request.id, number: 1, parent: request.expectedParent ? { id: request.expectedParent, fingerprint } : null, inputs: request.inputs, createdAt: new Date().toISOString(), fingerprint } });
  return { projectId, model, baseline, version, benchmark, workspace, inputs, datasets, preview, saved };
}
