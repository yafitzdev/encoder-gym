import assert from "node:assert/strict";
import test from "node:test";
import { parseOptimizationAgentSettings, parseOptimizationAgentPresets, providerLimitsWithin } from "../dist/evidence/optimization-agent-settings.js";
import { agentPresets, setupFixture } from "./optimization-setup-fixture.mjs";

test("settings preserve legacy shape and strictly bound optional whole-run provider ceilings", () => {
  const presets = agentPresets(), limits = setupFixture().workspace.providerCatalog.providers[0].limits;
  assert.deepEqual(parseOptimizationAgentPresets(presets), presets);
  assert.equal(Object.hasOwn(parseOptimizationAgentSettings(presets.standard), "providerLimits"), false);
  const settings = { ...presets.standard, providerLimits: { advisor: { ...limits }, generation: { ...limits } } };
  assert.deepEqual(parseOptimizationAgentSettings(settings), settings);
  for (const key of Object.keys(limits)) {
    const reduced = { ...limits, [key]: limits[key] - 1 };
    assert.ok(providerLimitsWithin(reduced, limits));
    assert.equal(providerLimitsWithin({ ...limits, [key]: limits[key] + 1 }, limits), false);
  }
  for (const alter of [
    value => value.providerLimits.advisor.maximumRequests = 1_000_001,
    value => value.providerLimits.advisor.maximumInputTokens = 0,
    value => value.providerLimits.generation.maximumOutputTokens = 0.5,
    value => value.providerLimits.generation.maximumCostMicrousd = -1,
    value => value.providerLimits.generation.maximumInputTokens = Number.MAX_SAFE_INTEGER + 1,
    value => value.providerLimits.path = "untrusted",
    value => value.providerLimits.advisor.apiKey = "secret",
    value => value.training.command = "untrusted",
  ]) {
    const invalid = structuredClone(settings); alter(invalid);
    assert.throws(() => parseOptimizationAgentSettings(invalid));
  }
  for (const alter of [value => value.maximumIterations = 2, value => value.maximumAgentTurnsPerIteration = 5,
    value => value.maximumRowChanges = 9, value => value.training.maximumEpochs = 2,
    value => value.training.maximumSecondsPerIteration = 121, value => value.training.maximumTrainingRows = null]) {
    const invalid = structuredClone(presets.quickTest); alter(invalid);
    assert.throws(() => parseOptimizationAgentSettings(invalid));
  }
});
