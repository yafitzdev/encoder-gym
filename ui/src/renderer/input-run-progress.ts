import type { NativeProgress } from "../managed-control.js";
import { inputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { spinner } from "./components.js";
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
}

export function pendingInputRunProgress(): HTMLElement {
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": "true" },
    h("div", { class: "optimization-progress-state" },
      h("div", { class: "optimization-progress-title" }, spinner(), h("strong", {}, "Starting run"))),
    h("p", { class: "optimization-progress-detail" }, "Creating the run record"),
    progressSteps(0, false));
}

/** The single live-status presentation shared by Optimize and Runs. */
export function inputRunProgress(options: InputRunProgressOptions): HTMLElement {
  const { run, running, startedAt, activity, context } = options;
  const phase = inputOptimizationPhase(run.state);
  const failed = run.state.endsWith("_failed");
  const result = run.state === "candidate_accepted" ? "Candidate passed"
    : run.state === "candidate_rejected" ? "Candidate did not pass"
    : run.state === "baseline_retained" ? "No improvement"
    : run.state === "cancelled" ? "Cancelled" : undefined;
  const active = phase === "complete" ? phases.length : Math.max(0, phases.indexOf(phase));
  const progress = activity?.progress;
  const activeProgress: NativeProgress = progress ?? { phase: fallback[phase] };
  const stage = inputRunStageLabel(activeProgress.phase);
  const current = result ?? (failed ? `Failed while ${stage.toLocaleLowerCase()}` : stage);
  const detail = result ? "" : inputRunStageDetail(activeProgress, context);
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(running) },
    h("div", { class: "optimization-progress-state" },
      h("div", { class: "optimization-progress-title" }, running ? spinner() : null, h("strong", {}, current)),
      running && startedAt ? h("span", { "data-elapsed-start": String(startedAt) }) : null),
    detail ? h("p", { class: "optimization-progress-detail" }, detail) : null,
    progress?.completed !== undefined && progress.total !== undefined ? h("div", { class: "optimization-live-progress" },
      h("progress", { value: progress.completed, max: progress.total }),
      h("span", {}, `${progress.completed.toLocaleString()} / ${progress.total.toLocaleString()}`)) : null,
    progressSteps(active, failed));
}

function progressSteps(active: number, failed: boolean): HTMLElement {
  return h("ol", { class: "optimization-progress-steps" },
    ...phases.map((item, index) => h("li", { class: index < active ? "complete" : index === active ? failed ? "failed" : "active" : "" }, labels[item])));
}
