import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { parseOptimizationHistory } from "../dist/evidence/optimization-history.js";

const projectId = randomUUID(), runId = randomUUID();
const first = { id: randomUUID(), number: 1, createdAt: new Date().toISOString(), startingModelId: randomUUID(), inputDatasetVersionId: randomUUID(), benchmarkVersionId: randomUUID(),
  experimentRunId: randomUUID(), qualifiedDatasetVersionId: randomUUID(), trainingDatasetVersionId: randomUUID(), modelId: randomUUID(),
  completed: true, noChange: false, selected: false, developmentPassed: false,
  checks: [{ reportId: randomUUID(), suite: "development", metric: "loss", direction: "lower_is_better", baseline: .5, candidate: .6, passed: false }] };
const history = () => ({ projectId, runId, iterations: [structuredClone(first)] });

test("history preserves rejected candidate custody and explicit lower-is-better comparisons", () => {
  assert.deepEqual(parseOptimizationHistory(history(), projectId, runId), [first]);
  const value = history(); value.iterations.push({ ...first, id: randomUUID(), number: 2, noChange: true, developmentPassed: null,
    experimentRunId: null, qualifiedDatasetVersionId: null, trainingDatasetVersionId: null, modelId: null, checks: [] });
  assert.equal(parseOptimizationHistory(value, projectId, runId)[1].noChange, true);
});

test("history rejects foreign identity, injected evidence, invalid ordering and false custody", () => {
  for (const alter of [
    value => value.runId = randomUUID(), value => value.projectId = randomUUID(),
    value => value.iterations[0].number = 2, value => value.iterations[0].sealedScore = 0,
    value => value.iterations[0].selected = true, value => value.iterations[0].modelId = null,
    value => value.iterations[0].experimentRunId = null, value => value.iterations[0].completed = false,
    value => value.iterations[0].noChange = true, value => value.iterations[0].checks[0].candidate = Infinity,
    value => value.iterations[0].checks[0].direction = "guess", value => value.iterations[0].checks[0].rawRows = [],
  ]) { const value = history(); alter(value); assert.throws(() => parseOptimizationHistory(value, projectId, runId)); }
});
