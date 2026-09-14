/** Immutable run configuration. No credentials or renderer-provided paths. */
export interface OptimizationAgentSettings {
  mode: "standard" | "quick_test";
  objective: string;
  maximumIterations: number;
  maximumAgentTurnsPerIteration: number;
  generationConcurrency: number;
  maximumRowChanges: number;
  training: {
    device: "auto" | "cpu" | "cuda";
    maximumEpochs: number;
    batchSize: number;
    learningRateNanos: number;
    maximumSecondsPerIteration: number;
    maximumTrainingRows: number | null;
  };
}

export function parseOptimizationAgentSettings(value: unknown): OptimizationAgentSettings {
  const object = (item: unknown, keys: string[]): Record<string, unknown> => {
    if (!item || typeof item !== "object" || Array.isArray(item) || Object.keys(item).length !== keys.length || Object.keys(item).some(key => !keys.includes(key))) throw new Error("Invalid agent settings.");
    return item as Record<string, unknown>;
  };
  const bounded = (item: unknown, maximum: number): number => {
    if (typeof item !== "number" || !Number.isSafeInteger(item) || item < 1 || item > maximum) throw new Error("Agent settings exceed supported limits.");
    return item;
  };
  const item = object(value, ["mode", "objective", "maximumIterations", "maximumAgentTurnsPerIteration", "generationConcurrency", "maximumRowChanges", "training"]);
  const training = object(item.training, ["device", "maximumEpochs", "batchSize", "learningRateNanos", "maximumSecondsPerIteration", "maximumTrainingRows"]);
  if ((item.mode !== "standard" && item.mode !== "quick_test") || typeof item.objective !== "string" || [...item.objective].length > 4_000 || /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(item.objective)) throw new Error("Invalid agent objective or mode.");
  if (training.device !== "auto" && training.device !== "cpu" && training.device !== "cuda") throw new Error("Invalid training device.");
  const result: OptimizationAgentSettings = {
    mode: item.mode, objective: item.objective,
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
  if (result.mode === "quick_test" && (result.maximumIterations !== 1 || result.maximumAgentTurnsPerIteration > 4 || result.maximumRowChanges > 8 || result.training.maximumEpochs !== 1 || result.training.maximumSecondsPerIteration > 120 || result.training.maximumTrainingRows === null || result.training.maximumTrainingRows > 64)) throw new Error("Quick-test limits are invalid.");
  return result;
}
