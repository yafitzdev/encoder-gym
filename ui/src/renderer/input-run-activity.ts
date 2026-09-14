import type { NativeProgress } from "../managed-control.js";
import type { ProjectActivityAction, ProjectActivityFailure, ProjectActivityLog, ProjectActivityNarrative } from "../project-activity.js";
import { validateNativeProgress } from "../native-progress.js";
import { isOptimizationStage, operationStages, stageLabels, taskStage, type OptimizationStage } from "../optimization-stages.js";

export interface InputRunActivityEntry { at: string; progress: NativeProgress; stage?: OptimizationStage; label?: string; narrative?: ProjectActivityNarrative }
export interface InputRunActivity {
  actionId: string;
  state: ProjectActivityAction["state"];
  startedAt: string;
  updatedAt: string;
  progress?: NativeProgress;
  failure?: ProjectActivityFailure;
  stages: string[];
  events?: InputRunActivityEntry[];
}

/** Keep each task in the live feed; counter updates replace that task's row. */
export function appendLiveActivity(events: InputRunActivityEntry[], progress: NativeProgress, at = Date.now()): void {
  const entry = { at: new Date(at).toISOString(), progress, ...(progress.narrative ? { narrative: progress.narrative } : {}) };
  const previous = events.at(-1);
  if (previous && !previous.label && previous.progress.runStage === progress.runStage && previous.progress.phase === progress.phase && previous.progress.subject === progress.subject
    && JSON.stringify(previous.narrative) === JSON.stringify(progress.narrative)) events[events.length - 1] = entry;
  else events.push(entry);
}

export function mergedActivity(activity: InputRunActivity | undefined, live: InputRunActivityEntry[], progress?: NativeProgress, at?: number): InputRunActivityEntry[] {
  const events = [...(activity?.events ?? [])];
  const latest = events.at(-1)?.at;
  const merged: InputRunActivityEntry[] = [...events];
  for (const entry of live.filter(event => !latest || Date.parse(event.at) > Date.parse(latest))) {
    appendLiveActivity(merged, entry.progress, Date.parse(entry.at));
  }
  if (progress && (!merged.length || (at !== undefined && at > Date.parse(merged.at(-1)!.at)))) {
    appendLiveActivity(merged, progress, at ?? (activity ? Date.parse(activity.updatedAt) : undefined));
  }
  return merged;
}

/** Project activity is the durable source for desktop orchestration progress. */
export function inputRunActivity(log: ProjectActivityLog, runId: string): InputRunActivity | undefined {
  const related = log.actions.filter(candidate => candidate.references.some(reference => reference.kind === "run" && reference.id === runId));
  const actions = related.filter(candidate => candidate.operation === "optimization.run" || candidate.operation === "optimization.start")
    .sort((a, b) => Date.parse(b.started_at) - Date.parse(a.started_at));
  const action = actions.find(candidate => candidate.operation === "optimization.run") ?? actions[0];
  if (!action) return undefined;
  const progressEvents = actions.slice().reverse().flatMap(candidate => candidate.events)
    .filter(event => event.state === "progress" && event.stage);
  const latest = progressEvents.at(-1);
  const progressOf = (event: (typeof progressEvents)[number]): NativeProgress => validateNativeProgress({
    phase: event.stage, completed: event.completed, total: event.total,
    subject: event.references?.find(item => item.kind === "progress_subject")?.id,
    unit: event.references?.find(item => item.kind === "progress_unit")?.id,
    narrative: event.narrative,
    runStage: event.references?.find(item => item.kind === "run_stage")?.id,
  }) ?? { phase: "checking_files" };
  const failure = action.events.findLast(event => event.state === "failed")?.failure;
  // Older records predate explicit stage references. Their CLI action intervals
  // still identify each command exactly, including retries and reused outputs.
  const spans = related.filter(item => operationStages[item.operation] && item.source === "cli");
  let currentStage: OptimizationStage = "checking_inputs";
  let previousSpan: string | undefined;
  const events = progressEvents.reduce<NonNullable<InputRunActivity["events"]>>((result, event) => {
    const projected = { at: event.created_at, progress: progressOf(event), ...(event.narrative ? { narrative: event.narrative } : {}) };
    const at = Date.parse(event.created_at);
    const span = spans.find(item => Date.parse(item.started_at) <= at && (!item.finished_at || at <= Date.parse(item.finished_at)));
    const command = span ? operationStages[span.operation] : undefined;
    if (span && span.action_id !== previousSpan) { currentStage = command!; previousSpan = span.action_id; }
    const explicit = projected.progress.runStage;
    currentStage = explicit ?? (command && command !== "training" ? command : taskStage(projected.progress.phase) ?? currentStage);
    const previous = result.at(-1);
    if (!projected.narrative && !previous?.narrative && previous?.progress.phase === projected.progress.phase
      && previous.progress.subject === projected.progress.subject && previous.stage === currentStage) result[result.length - 1] = { ...projected, stage: currentStage };
    else result.push({ ...projected, stage: currentStage });
    return result;
  }, []);
  for (const span of spans) {
    const stage = operationStages[span.operation]!;
    for (const event of span.events.filter(event => event.state !== "progress")) {
      events.push({ at: event.created_at, stage, progress: { phase: "checking_files" },
        label: `${stageLabels[stage]} · ${event.state === "started" ? "Started" : event.state === "succeeded" ? "Complete" : "Failed"}` });
    }
  }
  events.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
  return {
    actionId: action.action_id,
    state: action.state,
    startedAt: action.started_at,
    updatedAt: action.events.at(-1)?.created_at ?? action.started_at,
    ...(latest ? { progress: progressOf(latest) } : {}),
    ...(failure ? { failure } : {}),
    stages: [...new Set(progressEvents.map(event => event.stage!))],
    events,
  };
}

/** Group live and historical steps without mistaking a checksum for a new stage. */
export function stagedActivity(events: InputRunActivityEntry[]): InputRunActivityEntry[] {
  let stage: OptimizationStage = "checking_inputs";
  return events.map(event => {
    stage = event.stage ?? (isOptimizationStage(event.progress.runStage) ? event.progress.runStage : taskStage(event.progress.phase) ?? stage);
    return { ...event, stage };
  });
}

/**
 * Keep the durable activity record complete while turning checksum telemetry
 * into one user-level System activity. The actively changing file remains
 * visible until its checksum pass finishes; completed file bursts do not
 * overwhelm the stage history.
 */
export function presentedActivity(events: InputRunActivityEntry[], keepLiveChecksum = false): InputRunActivityEntry[] {
  const presented: InputRunActivityEntry[] = [];
  let checksums: InputRunActivityEntry[] = [];
  const flushChecksums = (atEnd: boolean): void => {
    if (!checksums.length) return;
    const latest = checksums.at(-1)!;
    presented.push(keepLiveChecksum && atEnd ? latest : {
      at: latest.at,
      stage: latest.stage,
      progress: { phase: "verifying_file" },
      label: "Verified required files",
    });
    checksums = [];
  };
  for (const event of events) {
    if (event.progress.phase === "verifying_file" && !event.narrative && !event.label) checksums.push(event);
    else { flushChecksums(false); presented.push(event); }
  }
  flushChecksums(true);
  return presented;
}
export { optimizationStages, stageLabels, type OptimizationStage } from "../optimization-stages.js";

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
  development_decision: "Development decision",
  final_decision: "Final decision",
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
    development_decision: "",
    final_decision: "",
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
