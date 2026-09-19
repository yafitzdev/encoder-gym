import type { OptimizationProviderLimits } from "./optimization-launch.js";

export interface OptimizationAgentPresets { standard: OptimizationAgentSettings; quickTest: OptimizationAgentSettings }
export interface OptimizationStartOptions { id: string; settings: OptimizationAgentSettings }

/** Immutable run configuration. No credentials or renderer-provided paths. */
export interface OptimizationAgentSettings {
  mode: "standard" | "quick_test";
  objective: string;
  /** Host-owned dataset inspection semantics; absent historical runs are V1. */
  analysisProtocol: 1 | 2 | 3;
  maximumIterations: number;
  maximumAgentTurnsPerIteration: number;
  generationConcurrency: number;
  /** Absent historical policy remains disabled; never filled from defaults. */
  generationCanary?: "first_batch_all_admitted_v1" | "per_combination_semantic_v3";
  maximumRowChanges: number;
  training: {
    device: "auto" | "cpu" | "cuda";
    maximumEpochs: number;
    batchSize: number;
    learningRateNanos: number;
    maximumSecondsPerIteration: number;
    maximumTrainingRows: number | null;
  };
  providerLimits?: { advisor: OptimizationProviderLimits; generation: OptimizationProviderLimits };
}

export function parseOptimizationProviderLimits(value: unknown): OptimizationProviderLimits {
  const keys = ["maximumRequests", "maximumInputTokens", "maximumOutputTokens", "maximumCostMicrousd"];
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== keys.length || Object.keys(value).some(key => !keys.includes(key))) throw new Error("Invalid provider ceilings.");
  const item = value as Record<string, unknown>;
  for (const key of keys) if (typeof item[key] !== "number" || !Number.isSafeInteger(item[key]) || item[key] < (key === "maximumCostMicrousd" ? 0 : 1)) throw new Error("Provider ceilings must be finite whole numbers.");
  if ((item.maximumRequests as number) > 1_000_000) throw new Error("Provider request ceiling exceeds the supported limit.");
  return Object.fromEntries(keys.map(key => [key, item[key]])) as unknown as OptimizationProviderLimits;
}

export function providerLimitsWithin(requested: OptimizationProviderLimits, maximum: OptimizationProviderLimits): boolean {
  return (Object.keys(requested) as (keyof OptimizationProviderLimits)[]).every(key => requested[key] <= maximum[key]);
}

export function parseOptimizationAgentPresets(value: unknown): OptimizationAgentPresets {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== 2 || !Object.hasOwn(value, "standard") || !Object.hasOwn(value, "quickTest")) throw new Error("Invalid optimization presets.");
  const item = value as Record<string, unknown>;
  const standard = parseOptimizationAgentSettings(item.standard), quickTest = parseOptimizationAgentSettings(item.quickTest);
  if (standard.mode !== "standard" || quickTest.mode !== "quick_test" || standard.providerLimits || quickTest.providerLimits) throw new Error("Invalid optimization preset modes.");
  return { standard, quickTest };
}

export function parseOptimizationAgentSettings(value: unknown): OptimizationAgentSettings {
  const object = (item: unknown, keys: string[], optional: string[] = []): Record<string, unknown> => {
    if (!item || typeof item !== "object" || Array.isArray(item) || keys.some(key => !Object.hasOwn(item, key)) || Object.keys(item).some(key => !keys.includes(key) && !optional.includes(key))) throw new Error("Invalid agent settings.");
    return item as Record<string, unknown>;
  };
  const bounded = (item: unknown, maximum: number): number => {
    if (typeof item !== "number" || !Number.isSafeInteger(item) || item < 1 || item > maximum) throw new Error("Agent settings exceed supported limits.");
    return item;
  };
  const item = object(value, ["mode", "objective", "maximumIterations", "maximumAgentTurnsPerIteration", "generationConcurrency", "maximumRowChanges", "training"], ["providerLimits", "analysisProtocol", "generationCanary"]);
  const training = object(item.training, ["device", "maximumEpochs", "batchSize", "learningRateNanos", "maximumSecondsPerIteration", "maximumTrainingRows"]);
  if ((item.mode !== "standard" && item.mode !== "quick_test") || typeof item.objective !== "string" || [...item.objective].length > 4_000 || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(item.objective)) throw new Error("Invalid agent objective or mode.");
  if (training.device !== "auto" && training.device !== "cpu" && training.device !== "cuda") throw new Error("Invalid training device.");
  const result: OptimizationAgentSettings = {
    mode: item.mode, objective: item.objective,
    analysisProtocol: (item.analysisProtocol === undefined ? 1 : bounded(item.analysisProtocol, 3)) as 1 | 2 | 3,
    maximumIterations: bounded(item.maximumIterations, 10),
    maximumAgentTurnsPerIteration: bounded(item.maximumAgentTurnsPerIteration, 32),
    generationConcurrency: bounded(item.generationConcurrency, 16),
    maximumRowChanges: bounded(item.maximumRowChanges, 5_000),
    training: {
      device: training.device,
      maximumEpochs: bounded(training.maximumEpochs, 10),
      batchSize: bounded(training.batchSize, 256),
      learningRateNanos: bounded(training.learningRateNanos, 1_000_000),
      maximumSecondsPerIteration: bounded(training.maximumSecondsPerIteration, 21_600),
      maximumTrainingRows: training.maximumTrainingRows === null ? null : bounded(training.maximumTrainingRows, 1_000_000),
    },
  };
  if (item.providerLimits !== undefined) {
    const limits = object(item.providerLimits, ["advisor", "generation"]);
    result.providerLimits = { advisor: parseOptimizationProviderLimits(limits.advisor), generation: parseOptimizationProviderLimits(limits.generation) };
  }
  if (item.generationCanary !== undefined) {
    if (item.generationCanary !== "first_batch_all_admitted_v1" && item.generationCanary !== "per_combination_semantic_v3") throw new Error("Invalid generation canary policy.");
    result.generationCanary = item.generationCanary;
  }
  if (result.analysisProtocol === 2 && result.maximumAgentTurnsPerIteration < 3) throw new Error("Dataset-intelligence analysis requires at least three Agent turns.");
  if (result.analysisProtocol === 3 && result.maximumAgentTurnsPerIteration < 4) throw new Error("Evidence-driven repair analysis requires at least four Agent turns.");
  if (result.generationCanary === "per_combination_semantic_v3" && result.analysisProtocol !== 3) throw new Error("Semantic per-combination canaries require analysis protocol V3.");
  if (result.mode === "quick_test" && (result.maximumIterations !== 1 || result.maximumAgentTurnsPerIteration > 4 || result.maximumRowChanges > 8 || result.training.maximumEpochs !== 1 || result.training.maximumSecondsPerIteration > 120 || result.training.maximumTrainingRows === null || result.training.maximumTrainingRows > 64)) throw new Error("Quick-test limits are invalid.");
  return result;
}
