import type { NativeProgress } from "./managed-control.js";
import type { ProjectActivityNarrative } from "./project-activity.js";
import { isOptimizationStage } from "./optimization-stages.js";

const phases = new Set([
  "agent_analysis", "data_generation",
  "finalizing_iteration",
  "checking_model", "checking_dataset", "checking_evaluation", "checking_runtime",
  "loading_training_rows", "writing_training_rows", "checking_materialized_project",
  "loading_evaluation_protocol", "creating_candidate", "creating_experiment",
  "registering_candidate", "development_decision", "final_decision", "optimization_complete", "verifying_file", "verifying_rows",
  "checking_files", "checking_training_data", "loading_model", "preparing_batches",
  "training", "saving_checkpoint", "evaluating_retrieval", "evaluating_agent",
]);

/** Shared trust boundary for native stdout telemetry and the renderer bridge. */
export function validateNativeProgress(input: unknown): NativeProgress | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input)) return;
  const value = input as Record<string, unknown>;
  if (!phases.has(value.phase as string) || Object.keys(value).some(key => !["phase", "completed", "total", "subject", "unit", "narrative", "runStage", "iteration"].includes(key))) return;
  if (value.iteration !== undefined && (!Number.isSafeInteger(value.iteration) || (value.iteration as number) < 1 || (value.iteration as number) > 0xffff_ffff)) return;
  if (value.runStage !== undefined && !isOptimizationStage(value.runStage)) return;
  if (value.subject !== undefined && (typeof value.subject !== "string" || !/^[a-zA-Z0-9._ -]{1,160}$/.test(value.subject))) return;
  if (typeof value.subject === "string" && /\bBearer\b|\bsk-|\bhf_[a-z0-9]/i.test(value.subject)) return;
  if (value.unit !== undefined && value.unit !== "bytes") return;
  if (value.completed !== undefined || value.total !== undefined) {
    if (!Number.isSafeInteger(value.completed) || !Number.isSafeInteger(value.total) || (value.completed as number) < 0 || (value.total as number) < 1
      || (value.total as number) > (value.unit === "bytes" ? Number.MAX_SAFE_INTEGER : 1_000_000_000) || (value.completed as number) > (value.total as number)) return;
  }
  if (value.unit && value.completed === undefined) return;
  const narrative = value.narrative === undefined ? undefined : validateNarrative(value.narrative);
  if (value.narrative !== undefined && !narrative) return;
  return { phase: value.phase as NativeProgress["phase"],
    ...(value.iteration !== undefined ? { iteration: value.iteration as number } : {}),
    ...(isOptimizationStage(value.runStage) ? { runStage: value.runStage } : {}),
    ...(value.completed !== undefined ? { completed: value.completed as number, total: value.total as number } : {}),
    ...(value.subject !== undefined ? { subject: value.subject as string } : {}),
    ...(value.unit === "bytes" ? { unit: "bytes" } : {}),
    ...(narrative ? { narrative } : {}),
  };
}

function validateNarrative(input: unknown): ProjectActivityNarrative | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input)) return;
  const value = input as Record<string, unknown>;
  if (Object.keys(value).some(key => !["origin", "kind", "summary"].includes(key))) return;
  if (!(["agent", "generation", "system"] as unknown[]).includes(value.origin)
    || !(["intent", "reasoning", "action", "observation", "decision", "next_step"] as unknown[]).includes(value.kind)
    || typeof value.summary !== "string" || !value.summary || value.summary.trim() !== value.summary
    || [...value.summary].length > 400 || /[\u0000-\u001f\u007f]/.test(value.summary)
    || /\bBearer\b|\bsk-[a-z0-9]|\bhf_[a-z0-9]|(?:api[_ -]?key|token|secret|password)\s*[:=]/i.test(value.summary)) return;
  return value as unknown as ProjectActivityNarrative;
}
