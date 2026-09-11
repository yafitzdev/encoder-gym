import type { ManagedRunStatus, NativeProgress } from "../managed-control.js";
import type { OptimizationPageState } from "./optimization-page.js";
import { h } from "./dom.js";

const phaseLabels: Partial<Record<NativeProgress["phase"], string>> = {
  checking_files: "Checking model files",
  checking_training_data: "Checking training data",
  loading_model: "Loading model",
  preparing_batches: "Preparing training batches",
  training: "Training candidate",
  saving_checkpoint: "Saving candidate checkpoint",
  evaluating_retrieval: "Evaluating retrieval",
  evaluating_agent: "Evaluating agent performance",
};

export function runIsWorking(run: ManagedRunStatus, executing?: string): boolean {
  if (["completed", "failed", "cancelled"].includes(run.state)) return false;
  return executing === "resume" || run.activity?.running === true || run.worker?.state === "running";
}

export function updateRunClocks(root: HTMLElement, now = Date.now()): void {
  for (const node of root.querySelectorAll<HTMLElement>("[data-elapsed-start]")) {
    const seconds = Math.max(0, Math.floor((now - Number(node.dataset.elapsedStart)) / 1000));
    const minutes = Math.floor(seconds / 60), hours = Math.floor(minutes / 60);
    node.textContent = hours ? `${hours}h ${minutes % 60}m ${seconds % 60}s` : minutes ? `${minutes}m ${seconds % 60}s` : `${seconds}s`;
  }
  for (const node of root.querySelectorAll<HTMLElement>("[data-checked-at]")) {
    const seconds = Math.max(0, Math.floor((now - Number(node.dataset.checkedAt)) / 1000));
    node.textContent = seconds < 3 ? "Updated just now" : `Last update ${seconds}s ago`;
    node.classList.toggle("is-stale", seconds >= 10);
  }
}

export function liveRun(run: ManagedRunStatus, state: OptimizationPageState): HTMLElement {
  const activity = run.activity?.running ? run.activity : undefined;
  const start = activity?.startedAt ?? run.worker?.started_at;
  const startedAt = start ? Date.parse(start) : state.operationStartedAt;
  const counter = activity?.completed !== undefined && activity.total !== undefined ? activity : undefined;
  return h("section", { class: "live-run", "aria-label": "Live run progress" },
    h("div", { class: "live-run-heading" },
      h("div", { role: "status" }, h("div", { class: "eyebrow" }, "Current task"), h("h3", {}, activity ? phaseLabels[activity.phase] ?? activity.phase.replaceAll("_", " ") : run.stage.label)),
      startedAt !== undefined && Number.isFinite(startedAt) ? h("div", { class: "live-elapsed" }, h("span", {}, "Elapsed"), h("strong", { "data-elapsed-start": String(startedAt) })) : null),
    counter ? h("div", { class: "live-counter" },
      h("div", {}, h("span", {}, counter.phase === "preparing_batches" ? "Preparation batches" : "Training steps"), h("strong", {}, `${counter.completed!.toLocaleString()} / ${counter.total!.toLocaleString()}`)),
      h("progress", { value: String(counter.completed), max: String(counter.total), "aria-label": counter.phase === "preparing_batches" ? "Preparation batches" : "Training steps" })) : null,
    h("div", { class: "live-run-meta" },
      h("span", { class: "run-freshness", ...(state.statusCheckedAt ? { "data-checked-at": String(state.statusCheckedAt) } : {}) }, "Waiting for status…"),
      run.worker?.state === "running" ? h("span", {}, "Worker active") : null,
      run.worker?.memory_bytes ? h("span", {}, `${(run.worker.memory_bytes / 1024 ** 3).toFixed(1)} GB RAM`) : null,
      run.worker?.cpu_milliseconds !== undefined ? h("span", {}, `${Math.floor(run.worker.cpu_milliseconds / 1000).toLocaleString()}s CPU time`) : null),
    state.statusError ? h("p", { class: "run-connection-warning", role: "alert" }, "Live updates unavailable. ", state.statusError) : null);
}

export function recentActivity(run: ManagedRunStatus): HTMLElement | null {
  const events = [...(run.timeline ?? []), ...(run.activity?.events.map(event => ({ at: event.at, label: phaseLabels[event.phase] ?? event.phase.replaceAll("_", " ") })) ?? [])]
    .filter(event => Number.isFinite(Date.parse(event.at)))
    .sort((a, b) => Date.parse(a.at) - Date.parse(b.at)).slice(-8).reverse();
  if (!events.length) return null;
  return h("section", { class: "run-activity", "aria-label": "Recent activity" },
    h("h3", {}, "Activity"), h("ol", {}, ...events.map(event => h("li", {},
      h("time", { datetime: event.at }, new Date(event.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })),
      h("span", {}, event.label)))));
}
