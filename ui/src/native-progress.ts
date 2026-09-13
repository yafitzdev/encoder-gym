import type { NativeProgress } from "./managed-control.js";

const phases = new Set([
  "checking_model", "checking_dataset", "checking_evaluation", "checking_runtime",
  "loading_training_rows", "writing_training_rows", "checking_materialized_project",
  "loading_evaluation_protocol", "creating_candidate", "creating_experiment",
  "registering_candidate", "optimization_complete", "verifying_file", "verifying_rows",
  "checking_files", "checking_training_data", "loading_model", "preparing_batches",
  "training", "saving_checkpoint", "evaluating_retrieval", "evaluating_agent",
]);

/** Shared trust boundary for native stdout telemetry and the renderer bridge. */
export function validateNativeProgress(input: unknown): NativeProgress | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input)) return;
  const value = input as Record<string, unknown>;
  if (!phases.has(value.phase as string) || Object.keys(value).some(key => !["phase", "completed", "total", "subject", "unit"].includes(key))) return;
  if (value.subject !== undefined && (typeof value.subject !== "string" || !/^[a-zA-Z0-9._ -]{1,160}$/.test(value.subject))) return;
  if (typeof value.subject === "string" && /\bBearer\b|\bsk-|\bhf_[a-z0-9]/i.test(value.subject)) return;
  if (value.unit !== undefined && value.unit !== "bytes") return;
  if (value.completed !== undefined || value.total !== undefined) {
    if (!Number.isSafeInteger(value.completed) || !Number.isSafeInteger(value.total) || (value.completed as number) < 0 || (value.total as number) < 1
      || (value.total as number) > (value.unit === "bytes" ? Number.MAX_SAFE_INTEGER : 1_000_000_000) || (value.completed as number) > (value.total as number)) return;
  }
  if (value.unit && value.completed === undefined) return;
  return { phase: value.phase as NativeProgress["phase"],
    ...(value.completed !== undefined ? { completed: value.completed as number, total: value.total as number } : {}),
    ...(value.subject !== undefined ? { subject: value.subject as string } : {}),
    ...(value.unit === "bytes" ? { unit: "bytes" } : {}),
  };
}

export function progressCounter(progress: NativeProgress): string | undefined {
  if (progress.completed === undefined || progress.total === undefined) return;
  if (progress.unit === "bytes") return;
  const unit = progress.phase === "verifying_rows" || progress.phase === "writing_training_rows" ? " rows" : progress.phase === "training" ? " steps" : progress.phase === "preparing_batches" ? " batches" : "";
  return `${progress.completed.toLocaleString()} / ${progress.total.toLocaleString()}${unit}`;
}
