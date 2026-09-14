import type { NativeProgress } from "../managed-control.js";
import { inputOptimizationPhase, type InputOptimizationRun } from "../input-optimization.js";
import { spinner } from "./components.js";
import { h } from "./dom.js";
import { inputRunStageDetail, inputRunStageLabel, mergedActivity, stagedActivity, type InputRunActivity, type InputRunActivityEntry, type InputRunStageContext } from "./input-run-activity.js";
import { optimizationStages, stageLabels, taskStage, type OptimizationStage } from "../optimization-stages.js";

const phases = optimizationStages, labels = stageLabels;
export interface ActivityView { stage?: OptimizationStage; scroll: Partial<Record<OptimizationStage, number>> }
interface ActivityNavigation { view: ActivityView; change: (stage?: OptimizationStage) => void; key: string }
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
  navigation?: ActivityNavigation;
}

export function pendingInputRunProgress(progress?: NativeProgress, animationKey = "pending-run", events: InputRunActivityEntry[] = [], controls?: HTMLElement, running = true, navigation?: ActivityNavigation): HTMLElement {
  const selected = navigation?.view.stage ?? "checking_inputs";
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(running) },
    progressSteps(0, false, selected, navigation),
    h("div", { class: "optimization-current-work" },
      h("div", { class: "optimization-progress-state" },
        h("div", { class: "optimization-progress-title" }, running ? spinner(animationKey) : null, h("strong", {}, running ? "Checking inputs" : "Paused")), controls)),
    activityStream(stagedActivity(mergedActivity(undefined, events, progress)).filter(event => event.stage === selected), undefined, running && selected === "checking_inputs", selected, navigation));
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
  const events = stagedActivity(mergedActivity(activity, options.liveEvents ?? [], progress, options.liveProgressAt));
  // Checksums are steps within the current stage, never a reason to jump back.
  const tasks = [...(activity?.stages ?? []), ...events.map(event => event.progress.phase), ...(observed ? [observed] : [])];
  const stagePhase = events.at(-1)?.stage ?? tasks.reverse().map(taskStage).find(value => value !== undefined);
  const visiblePhase = options.registrationPending ? "saving_candidate" : phase === "complete" ? phase : stagePhase ?? phase;
  const active = visiblePhase === "complete" ? phases.length : Math.max(0, phases.indexOf(visiblePhase));
  const activeProgress: NativeProgress = progress ?? { phase: fallback[phase] };
  const phaseLabel = visiblePhase === "complete" ? "Complete" : labels[visiblePhase];
  const current = result ?? (failed ? `${labels[phase as keyof typeof labels] ?? phaseLabel} failed` : running ? phaseLabel : `Paused · ${phaseLabel}`);
  const selected = options.navigation?.view.stage ?? (visiblePhase === "complete" ? events.at(-1)?.stage ?? "evaluating" : visiblePhase);
  const selectedEvents = events.filter(event => event.stage === selected);
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(running) },
    progressSteps(active, failed, selected, options.navigation),
    h("div", { class: "optimization-current-work" },
      h("div", { class: "optimization-progress-state" },
        h("div", { class: "optimization-progress-title" }, running ? spinner(options.animationKey ?? `run-progress:${run.id}`) : null, h("strong", {}, current)),
        h("div", { class: "optimization-status-controls" }, running && startedAt ? h("span", { class: "muted" }, "Elapsed ", h("span", { "data-elapsed-start": String(startedAt) })) : null, options.controls))),
    activityStream(selectedEvents.length || events.length ? selectedEvents : selected === visiblePhase ? [{ at: activity?.updatedAt ?? new Date().toISOString(), progress: activeProgress }] : [], context,
      running && selected === events.at(-1)?.stage, selected, options.navigation));
}

function activityStream(events: InputRunActivityEntry[], context: InputRunStageContext | undefined, running: boolean, selected: OptimizationStage, navigation?: ActivityNavigation): HTMLElement {
  const list = h("ol", { class: "focus-events", "aria-label": `${labels[selected]} activity`,
    onScroll: (event: Event) => { if (navigation) navigation.view.scroll[selected] = (event.target as HTMLElement).scrollTop; } }, ...events.slice().reverse().map((event, index) => {
      const narrative = event.narrative, live = running && index === 0;
      const detail = narrative || event.label ? "" : context ? inputRunStageDetail(event.progress, context) : event.progress.subject ?? "";
      return h("li", { "data-activity-stage": selected, class: `focus-event ${narrative ? `narrative ${narrative.origin} ${narrative.kind}` : "work"}${live ? " current" : ""}` },
        h("time", { datetime: event.at }, new Date(event.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })),
        h("span", { class: "focus-event-kind" }, narrative ? ({ intent: "Intent", reasoning: "Reason", action: "Action", observation: "Result", decision: "Decision", next_step: "Next" })[narrative.kind] : live ? "Now" : "Work",
          narrative ? h("small", {}, narrative.origin === "agent" ? "Agent" : "System") : null),
        h("div", { class: "focus-event-copy" }, h("strong", {}, event.label ?? narrative?.summary ?? inputRunStageLabel(event.progress.phase)),
          detail ? h("span", {}, detail) : null,
          index === 0 ? counterBar(event.progress) : null));
    }));
  queueMicrotask(() => { if (list.isConnected && navigation) list.scrollTop = navigation.view.scroll[selected] ?? 0; });
  return h("div", { class: "optimization-activity", id: navigation ? `${navigation.key}-activity` : undefined, role: "region", "aria-label": `${labels[selected]} activity` },
    h("div", { class: "activity-heading" }, h("h3", { class: "focus-activity-title" }, "Activity · ", labels[selected]),
      navigation?.view.stage ? h("button", { type: "button", class: "button ghost small", onClick: () => navigation.change() }, "Latest") : null),
    events.length ? list : h("p", { class: "muted activity-empty" }, "No recorded activity"));
}

function counterBar(progress: NativeProgress): HTMLElement | null {
  if (progress.completed === undefined || progress.total === undefined) return null;
  return h("div", { class: "optimization-live-progress meter-only" },
    h("progress", { value: progress.completed, max: progress.total, "aria-label": progress.subject ?? inputRunStageLabel(progress.phase) }));
}

function progressSteps(active: number, failed: boolean, selected: OptimizationStage, navigation?: ActivityNavigation): HTMLElement {
  return h("ol", { class: "optimization-progress-steps", "aria-label": "Run stages" },
    ...phases.map((item, index) => h("li", {
      class: index < active ? "complete" : index === active ? failed ? "failed" : "active" : "",
      "aria-current": index === active ? "step" : null,
    }, navigation ? h("button", { type: "button", id: `${navigation.key}-stage-${item}`, "aria-controls": `${navigation.key}-activity`, "aria-pressed": String(selected === item),
      onClick: () => navigation.change(item), onKeydown: (event: KeyboardEvent) => {
        const target = event.key === "ArrowRight" ? (index + 1) % phases.length : event.key === "ArrowLeft" ? (index + phases.length - 1) % phases.length : event.key === "Home" ? 0 : event.key === "End" ? phases.length - 1 : undefined;
        if (target === undefined) return;
        event.preventDefault(); navigation.change(phases[target]); document.getElementById(`${navigation.key}-stage-${phases[target]}`)?.focus();
      } }, labels[item]) : labels[item])));
}
