import type { TrainingMetrics } from "./managed-control.js";
import type { ProjectActivityReference } from "./project-activity.js";

const keys = { elapsedSeconds: "training_elapsed_seconds", remainingSeconds: "training_remaining_seconds", finalLoss: "training_final_loss" } as const;

export function validateTrainingMetrics(input: unknown): TrainingMetrics | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input)) return;
  const value = input as Record<string, unknown>, names = Object.keys(value);
  if (!names.length || names.some(key => !Object.hasOwn(keys, key))) return;
  for (const key of names) {
    const metric = value[key];
    if (typeof metric !== "number" || !Number.isFinite(metric)) return;
    if (key === "finalLoss" ? Math.abs(metric) > 1e12 : !Number.isSafeInteger(metric) || metric < 0 || metric > 31_536_000) return;
  }
  return { ...value } as TrainingMetrics;
}

/** Existing immutable activity references carry only these bounded numbers. */
export function trainingMetricReferences(input?: TrainingMetrics): ProjectActivityReference[] {
  const value = validateTrainingMetrics(input);
  return value ? Object.entries(keys).flatMap(([key, kind]) => value[key as keyof TrainingMetrics] === undefined ? [] : [{ kind, id: String(value[key as keyof TrainingMetrics]) }]) : [];
}

export function trainingMetricsFromReferences(references: ProjectActivityReference[] = []): TrainingMetrics | undefined {
  const value: Record<string, number> = {};
  for (const [key, kind] of Object.entries(keys)) {
    const found = references.filter(item => item.kind === kind);
    if (found.length > 1) return;
    if (found.length) {
      const text = found[0]!.id;
      if (!/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:e[+-]?\d+)?$/i.test(text)) return;
      value[key] = Number(text);
    }
  }
  return validateTrainingMetrics(value);
}
