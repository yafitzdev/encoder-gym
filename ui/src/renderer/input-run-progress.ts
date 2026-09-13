import type { NativeProgress } from "../managed-control.js";
import { inputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { spinner } from "./components.js";
import { progressCounter } from "../native-progress.js";
import { h } from "./dom.js";
import { inputRunStageDetail, inputRunStageLabel, type InputRunActivity, type InputRunStageContext } from "./input-run-activity.js";

const phases = ["checking_inputs", "preparing_data", "starting", "training", "saving_candidate", "evaluating"] as const;
const labels = {
  checking_inputs: "Checking inputs", preparing_data: "Preparing data", starting: "Starting", training: "Training",
  saving_candidate: "Saving candidate", evaluating: "Evaluating",
} as const;
const fallback: Record<ReturnType<typeof inputOptimizationPhase>, NativeProgress["phase"]> = {
  checking_inputs: "checking_model", preparing_data: "loading_training_rows", starting: "loading_evaluation_protocol",
  training: "checking_files", saving_candidate: "registering_candidate", evaluating: "checking_evaluation", complete: "optimization_complete",
};

export interface InputRunProgressOptions {
  run: InputOptimizationRun;
  running: boolean;
  startedAt?: number;
  activity?: InputRunActivity;
  context: InputRunStageContext;
  liveProgress?: NativeProgress;
  liveProgressAt?: number;
  registrationPending?: boolean;
}

export function pendingInputRunProgress(progress?: NativeProgress): HTMLElement {
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": "true" },
    progressSteps(0, false),
    h("div", { class: "optimization-current-work" },
      h("div", { class: "optimization-progress-state" },
        h("div", { class: "optimization-progress-title" }, spinner(), h("strong", {}, progress ? inputRunStageLabel(progress.phase) : "Starting run"))),
      h("p", { class: "optimization-progress-detail" }, progress?.subject ?? "Creating the run record"),
      progress ? counterBar(progress) : null));
}

/** The single live-status presentation shared by Optimize and Runs. */
export function inputRunProgress(options: InputRunProgressOptions): HTMLElement {
  const { run, running, startedAt, activity, context } = options;
  const phase = options.registrationPending ? "saving_candidate" : inputOptimizationPhase(run.state);
  const failed = run.state.endsWith("_failed");
  const result = options.registrationPending ? undefined : run.state === "candidate_accepted" ? "Candidate passed"
    : run.state === "candidate_rejected" ? "Candidate did not pass"
    : run.state === "baseline_retained" ? "No improvement"
    : run.state === "cancelled" ? "Cancelled" : undefined;
  const progress = options.registrationPending && !running ? { phase: "registering_candidate" as const } : running ? options.liveProgress ?? activity?.progress : activity?.progress;
  const observed = progress?.phase;
  const visiblePhase = phase === "complete" ? phase : observed && ["evaluating_retrieval", "evaluating_agent"].includes(observed) ? "evaluating"
    : observed === "saving_checkpoint" || observed === "registering_candidate" ? "saving_candidate"
    : observed && ["checking_training_data", "loading_model", "preparing_batches", "training"].includes(observed) ? "training"
    : observed && ["loading_training_rows", "writing_training_rows", "checking_materialized_project"].includes(observed) ? "preparing_data"
    : observed && ["loading_evaluation_protocol", "creating_candidate", "creating_experiment"].includes(observed) ? "starting" : phase;
  const active = visiblePhase === "complete" ? phases.length : Math.max(0, phases.indexOf(visiblePhase));
  const activeProgress: NativeProgress = progress ?? { phase: fallback[phase] };
  const stage = inputRunStageLabel(activeProgress.phase);
  const phaseLabel = phase === "complete" ? "Complete" : labels[phase];
  const current = result ?? (failed ? `${phaseLabel} failed` : running ? stage : `Paused · ${stage}`);
  const detail = result ? "" : inputRunStageDetail(activeProgress, context);
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(running) },
    progressSteps(active, failed),
    h("div", { class: "optimization-current-work" },
      h("div", { class: "optimization-progress-state" },
        h("div", { class: "optimization-progress-title" }, running ? spinner() : null, h("strong", {}, current)),
        running && startedAt ? h("span", {}, "Elapsed ", h("span", { "data-elapsed-start": String(startedAt) })) : null),
      detail ? h("p", { class: "optimization-progress-detail" }, detail) : null,
      !result && progress ? counterBar(progress) : null,
      running && (options.liveProgressAt || activity?.updatedAt) ? h("small", { class: "optimization-progress-updated muted", "data-checked-at": String(options.liveProgressAt ?? Date.parse(activity!.updatedAt)) }, "Updated just now") : null));
}

function counterBar(progress: NativeProgress): HTMLElement | null {
  if (progress.completed === undefined || progress.total === undefined) return null;
  const label = progressCounter(progress);
  return h("div", { class: "optimization-live-progress" + (label ? "" : " meter-only") },
    h("progress", { value: progress.completed, max: progress.total, "aria-label": progress.subject ?? inputRunStageLabel(progress.phase) }),
    label ? h("span", {}, label) : null);
}

function progressSteps(active: number, failed: boolean): HTMLElement {
  return h("ol", { class: "optimization-progress-steps", "aria-label": "Run stages" },
    ...phases.map((item, index) => h("li", {
      class: index < active ? "complete" : index === active ? failed ? "failed" : "active" : "",
      "aria-current": index === active ? "step" : null,
    }, labels[item])));
}
