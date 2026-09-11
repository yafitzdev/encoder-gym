import type { ProjectActivityAction, ProjectActivityLog, ProjectActivityReference, ProjectActivityState } from "../project-activity.js";
import { button, copyField, failureNotice, facts, tag, workspacePage } from "./components.js";
import { h } from "./dom.js";

export interface ActivityPageState {
  loading: boolean;
  exporting?: boolean;
  error?: string;
  log?: ProjectActivityLog;
}

export interface ActivityPageActions {
  refresh(): void;
  export(): void;
  copy(value: string): void;
}

const operationLabels: Record<string, string> = {
  "activity.logging_enabled": "Activity logging enabled",
  "project.created": "Project created",
  "project.connected": "Project connected",
  "project.rename": "Project renamed",
  "project.relocate": "Project reconnected",
  "project.forget": "Project removed from app",
  "project.verify": "Project verified",
  "project.upgrade": "Project upgraded",
  "dataset.import": "Dataset imported",
  "optimization.prepare": "Optimization prepared",
  "optimization.reserve": "Run reserved",
  "optimization.resume": "Run continued",
  "models.register_run": "Models registered",
  "dataset.create": "Dataset created",
  "dataset.fork": "Dataset variant created",
  "dataset.revise": "Dataset version saved",
  "dataset.adopt_baseline": "Recorded training data linked",
  "dataset.adopt_run": "Run training data linked",
  "benchmark.adopt_run": "Benchmark version saved",
  "optimization.authorize_external": "External calls authorized",
  "optimization.authorize_sealed": "Sealed evaluation authorized",
  "optimization.cancel": "Run cancelled",
  "model.promote": "Model promoted",
  "model.restore_baseline": "Baseline restored",
  "providers.configure": "Providers configured",
  "credential.save": "Credential saved",
  "credential.remove": "Credential removed",
  "runtime.verify": "Runtime verified",
  "runtime.prepare_python": "Python runtime prepared",
  "runtime.bind": "Runtime connected",
};
const label = (value: string): string => operationLabels[value] ?? value.replaceAll("_", " ").replaceAll(".", " · ");
const time = (value: string): string => new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "medium" }).format(new Date(value));
const shortId = (value: string): string => value.length > 18 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
const stateLabel = (state: ProjectActivityState): string => ({ started: "In progress", progress: "In progress", succeeded: "Succeeded", failed: "Failed" })[state];
const stateTone = (state: ProjectActivityState): string => state === "succeeded" ? "success" : state === "failed" ? "danger" : "accent";
const referenceName = (reference: ProjectActivityReference): string => reference.kind.replaceAll("_", " ");

function renderAction(action: ProjectActivityAction, actions: ActivityPageActions): HTMLElement {
  const last = action.events.at(-1)!;
  return h("details", { class: `activity-action activity-${action.state}` },
    h("summary", {},
      h("time", { datetime: action.started_at }, time(action.started_at)),
      h("strong", {}, label(action.operation)),
      tag(stateLabel(action.state), stateTone(action.state)),
      h("code", {}, shortId(action.action_id))),
    h("div", { class: "activity-action-body" },
      facts([
        ["Action UUID", copyField(action.action_id, actions.copy)],
        ["Source", action.source],
        ["Started", time(action.started_at)],
        ...(action.finished_at ? [["Finished", time(action.finished_at)] as [string, string]] : []),
      ]),
      action.references.length ? h("div", { class: "activity-references" }, ...action.references.map(reference =>
        h("div", {}, h("span", {}, referenceName(reference)), copyField(reference.id, actions.copy)))) : null,
      last.failure ? h("div", { class: "activity-failure", role: "alert" }, h("strong", {}, last.failure.code), h("span", {}, last.failure.message)) : null,
      h("ol", { class: "activity-events" }, ...action.events.map(event => h("li", {},
        h("time", { datetime: event.created_at }, time(event.created_at)),
        h("span", {}, event.stage ? label(event.stage) : stateLabel(event.state)),
        event.completed !== undefined && event.total !== undefined ? h("span", { class: "activity-progress-count" }, `${event.completed} / ${event.total}`) : null,
        h("button", { type: "button", class: "activity-event-id", title: "Copy event UUID", onClick: () => actions.copy(event.id) }, shortId(event.id)))))));
}

export function renderActivity(state: ActivityPageState, actions: ActivityPageActions): HTMLElement {
  const controls = h("div", { class: "page-actions" },
    button(state.exporting ? "Exporting…" : "Export JSONL", actions.export, "secondary"),
    button(state.loading ? "Refreshing…" : "Refresh", actions.refresh, "secondary", "refresh"));
  for (const control of controls.querySelectorAll<HTMLButtonElement>("button")) control.disabled = state.loading || !!state.exporting;
  return workspacePage("Activity", controls,
    state.error ? h("section", { role: "alert", class: "operation-failure" }, failureNotice(state.error)) : null,
    state.loading && !state.log ? h("div", { class: "activity-loading", role: "status" }, "Loading activity…") : null,
    state.log && !state.log.actions.length ? h("div", { class: "empty-state" }, h("h2", {}, "No activity yet")) : null,
    state.log?.actions.length ? h("div", { class: "activity-list" }, ...state.log.actions.map(action => renderAction(action, actions))) : null);
}
