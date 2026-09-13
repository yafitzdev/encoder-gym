import type { NativeProgress } from "../managed-control.js";
import type { ProjectActivityAction, ProjectActivityFailure, ProjectActivityLog } from "../project-activity.js";
import { validateNativeProgress } from "../native-progress.js";

export interface InputRunActivity {
  actionId: string;
  state: ProjectActivityAction["state"];
  startedAt: string;
  updatedAt: string;
  progress?: NativeProgress;
  failure?: ProjectActivityFailure;
  stages: string[];
  events?: { at: string; progress: NativeProgress }[];
}

/** Project activity is the durable source for desktop orchestration progress. */
export function inputRunActivity(log: ProjectActivityLog, runId: string): InputRunActivity | undefined {
  const action = log.actions.find(candidate => candidate.operation === "optimization.run"
    && candidate.references.some(reference => reference.kind === "run" && reference.id === runId));
  if (!action) return undefined;
  const progressEvents = action.events.filter(event => event.state === "progress" && event.stage);
  const latest = progressEvents.at(-1);
  const progressOf = (event: (typeof progressEvents)[number]): NativeProgress => validateNativeProgress({
    phase: event.stage, completed: event.completed, total: event.total,
    subject: event.references?.find(item => item.kind === "progress_subject")?.id,
    unit: event.references?.find(item => item.kind === "progress_unit")?.id,
  }) ?? { phase: "checking_files" };
  const failure = action.events.findLast(event => event.state === "failed")?.failure;
  return {
    actionId: action.action_id,
    state: action.state,
    startedAt: action.started_at,
    updatedAt: action.events.at(-1)?.created_at ?? action.started_at,
    ...(latest ? { progress: progressOf(latest) } : {}),
    ...(failure ? { failure } : {}),
    stages: [...new Set(progressEvents.map(event => event.stage!))],
    events: progressEvents.slice(-5).map(event => ({ at: event.created_at, progress: progressOf(event) })),
  };
}

export const inputRunStageLabel = (stage: string): string => ({
  verifying_file: "Verifying file checksum",
  verifying_rows: "Validating training rows",
  checking_model: "Checking baseline model",
  checking_dataset: "Checking dataset",
  checking_evaluation: "Checking evaluation",
  checking_runtime: "Checking local runtime",
  loading_training_rows: "Loading training rows",
  writing_training_rows: "Building training dataset",
  checking_materialized_project: "Checking prepared project",
  loading_evaluation_protocol: "Loading evaluation",
  creating_candidate: "Creating candidate",
  creating_experiment: "Creating optimization run",
  registering_candidate: "Adding candidate to Models",
  optimization_complete: "Complete",
  checking_files: "Checking model files",
  checking_training_data: "Checking training data",
  loading_model: "Loading model",
  preparing_batches: "Preparing batches",
  training: "Training candidate",
  saving_checkpoint: "Saving candidate",
  evaluating_retrieval: "Evaluating retrieval",
  evaluating_agent: "Evaluating agent",
}[stage] ?? stage.replaceAll("_", " "));

export interface InputRunStageContext {
  model: string;
  dataset: string;
  datasetRows?: number;
  evaluation: string;
  developmentSuites: string[];
  finalSuite?: string;
  finalEvaluation: boolean;
}

/** Short artifact-level context for the currently executing persisted stage. */
export function inputRunStageDetail(progress: NativeProgress, context: InputRunStageContext): string {
  if (progress.subject) return progress.subject;
  const data = context.dataset;
  const suites = context.finalEvaluation && context.finalSuite
    ? context.finalSuite
    : context.developmentSuites.join(", ") || context.evaluation;
  return ({
    verifying_file: "File checksum",
    verifying_rows: data,
    checking_model: context.model,
    checking_dataset: data,
    checking_evaluation: context.evaluation,
    checking_runtime: "Model runtime · trainer · evaluation database",
    loading_training_rows: data,
    writing_training_rows: data,
    checking_materialized_project: `${context.model} + ${context.dataset}`,
    loading_evaluation_protocol: context.evaluation,
    creating_candidate: `${context.model} → Candidate 1`,
    creating_experiment: `${context.evaluation} · ${context.developmentSuites.length} development suites`,
    registering_candidate: "Candidate 1",
    optimization_complete: "",
    checking_files: context.model,
    checking_training_data: data,
    loading_model: context.model,
    preparing_batches: data,
    training: "Candidate 1",
    saving_checkpoint: "Candidate 1",
    evaluating_retrieval: `${context.evaluation} · ${suites}`,
    evaluating_agent: `${context.evaluation} · ${suites}`,
  }[progress.phase] ?? context.evaluation);
}
