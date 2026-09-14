import type { NativeProgress } from "../managed-control.js";
import { inputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { spinner } from "./components.js";
import { h } from "./dom.js";
import { inputRunStageDetail, inputRunStageLabel, mergedActivity, type InputRunActivity, type InputRunActivityEntry, type InputRunStageContext } from "./input-run-activity.js";

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
  animationKey?: string;
  liveEvents?: InputRunActivityEntry[];
  controls?: HTMLElement;
}

export function pendingInputRunProgress(progress?: NativeProgress, animationKey = "pending-run", events: InputRunActivityEntry[] = [], controls?: HTMLElement, running = true): HTMLElement {
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(running) },
    progressSteps(0, false),
    h("div", { class: "optimization-current-work" },
      h("div", { class: "optimization-progress-state" },
        h("div", { class: "optimization-progress-title" }, running ? spinner(animationKey) : null, h("strong", {}, running ? "Checking inputs" : "Paused")), controls)),
    activityStream(mergedActivity(undefined, events, progress), undefined, running));
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
  const events = mergedActivity(activity, options.liveEvents ?? [], progress, options.liveProgressAt);
  // Checksums are steps within the current stage, never a reason to jump back.
  const tasks = [...(activity?.stages ?? []), ...events.map(event => event.progress.phase), ...(observed ? [observed] : [])];
  const stagePhase = tasks.reverse().map(taskPhase).find(value => value !== undefined);
  const visiblePhase = options.registrationPending ? "saving_candidate" : phase === "complete" ? phase : stagePhase ?? phase;
  const active = visiblePhase === "complete" ? phases.length : Math.max(0, phases.indexOf(visiblePhase));
  const activeProgress: NativeProgress = progress ?? { phase: fallback[phase] };
  const phaseLabel = visiblePhase === "complete" ? "Complete" : labels[visiblePhase];
  const current = result ?? (failed ? `${labels[phase as keyof typeof labels] ?? phaseLabel} failed` : running ? phaseLabel : `Paused · ${phaseLabel}`);
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(running) },
    progressSteps(active, failed),
    h("div", { class: "optimization-current-work" },
      h("div", { class: "optimization-progress-state" },
        h("div", { class: "optimization-progress-title" }, running ? spinner(options.animationKey ?? `run-progress:${run.id}`) : null, h("strong", {}, current)),
        h("div", { class: "optimization-status-controls" }, running && startedAt ? h("span", { class: "muted" }, "Elapsed ", h("span", { "data-elapsed-start": String(startedAt) })) : null, options.controls))),
    activityStream(events.length ? events : [{ at: activity?.updatedAt ?? new Date().toISOString(), progress: activeProgress }], context, running));
}

function taskPhase(task: string): (typeof phases)[number] | undefined {
  if (["evaluating_retrieval", "evaluating_agent", "development_decision", "final_decision"].includes(task)) return "evaluating";
  if (["saving_checkpoint", "registering_candidate"].includes(task)) return "saving_candidate";
  if (["checking_training_data", "loading_model", "preparing_batches", "training"].includes(task)) return "training";
  if (["loading_training_rows", "writing_training_rows", "checking_materialized_project"].includes(task)) return "preparing_data";
  if (["loading_evaluation_protocol", "creating_candidate", "creating_experiment"].includes(task)) return "starting";
  return undefined;
}

function activityStream(events: InputRunActivityEntry[], context: InputRunStageContext | undefined, running: boolean): HTMLElement {
  return h("div", { class: "optimization-activity" }, h("h3", { class: "focus-activity-title" }, "Activity"),
    h("ol", { class: "focus-events", "aria-label": "Run activity" }, ...events.slice().reverse().map((event, index) => {
      const narrative = event.narrative, live = running && index === 0;
      const detail = narrative ? "" : context ? inputRunStageDetail(event.progress, context) : event.progress.subject ?? "";
      return h("li", { class: `focus-event ${narrative ? `narrative ${narrative.origin} ${narrative.kind}` : "work"}${live ? " current" : ""}` },
        h("time", { datetime: event.at }, new Date(event.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })),
        h("span", { class: "focus-event-kind" }, narrative ? ({ intent: "Intent", reasoning: "Reason", action: "Action", observation: "Result", decision: "Decision", next_step: "Next" })[narrative.kind] : live ? "Now" : "Work",
          narrative ? h("small", {}, narrative.origin === "agent" ? "Agent" : "System") : null),
        h("div", { class: "focus-event-copy" }, h("strong", {}, narrative?.summary ?? inputRunStageLabel(event.progress.phase)),
          detail ? h("span", {}, detail) : null,
          index === 0 ? counterBar(event.progress) : null));
    })));
}

function counterBar(progress: NativeProgress): HTMLElement | null {
  if (progress.completed === undefined || progress.total === undefined) return null;
  return h("div", { class: "optimization-live-progress meter-only" },
    h("progress", { value: progress.completed, max: progress.total, "aria-label": progress.subject ?? inputRunStageLabel(progress.phase) }));
}

function progressSteps(active: number, failed: boolean): HTMLElement {
  return h("ol", { class: "optimization-progress-steps", "aria-label": "Run stages" },
    ...phases.map((item, index) => h("li", {
      class: index < active ? "complete" : index === active ? failed ? "failed" : "active" : "",
      "aria-current": index === active ? "step" : null,
    }, labels[item])));
}
