import type { OpenedProject, ProjectEntry } from "../projects.js";
import type { Actions, ProjectActions } from "./actions.js";
import { candidateRows, dateLabel, timeLabel } from "./catalog.js";
import { button, copyField, empty, facts, failureNotice, icon, pageHeader, sectionHeader, tag } from "./components.js";
import { h, type Child } from "./dom.js";

export function renderWelcome(actions: ProjectActions): HTMLElement {
  return h("div", { class: "page-content welcome-page" },
    pageHeader("Encoder Gym"),
    h("div", { class: "welcome-action" }, icon("models"), h("h2", {}, "New project"), button("Create", actions.create, "primary")),
    h("div", { class: "welcome-action" }, icon("project"), h("h2", {}, "Open project"), button("Open", actions.openManaged, "secondary")),
    h("button", { type: "button", id: "open-recorded-example", class: "inline-link welcome-example", onClick: actions.openExample }, "Recorded example →"),
  );
}
export function renderProjectState(project: ProjectEntry, opened: OpenedProject | undefined, actions: Actions, projects: ProjectActions, page = "models"): HTMLElement {
  if (!opened) return h("div", { class: "page-content", role: "status" }, pageHeader(project.name), h("div", { class: "workspace-progress" }, "Loading…"));
  const content = opened.content;
  if (content.state === "error") return h("div", { class: "page-content" }, pageHeader(project.name),
    h("section", { class: "project-recovery", role: "alert" }, failureNotice(content.message), h("div", { class: "inline-group" }, button("Try again", actions.refresh, "primary", "refresh"), project.source.kind === "folder" ? button("Locate folder", projects.relocate, "secondary", "project") : null, button("Project settings", () => actions.navigate({ page: "project" }), "ghost"))),
    );
  if (page === "runs" || page === "benchmarks") return h("div", { class: "page-content" }, pageHeader(page === "runs" ? "Runs" : "Evaluation"),
    empty(page === "runs" ? "No runs" : "No evaluations", button("Models", () => actions.navigate({ page: "models" }), "primary")));
  return h("div", { class: "page-content" }, pageHeader("Models"),
    h("section", { class: "empty-baseline" }, icon("models"), h("div", {}, h("div", { class: "eyebrow" }, "Legacy folder"), h("h2", {}, "No baseline"))),
    h("section", { class: "setup-section" }, sectionHeader("Set up this project"),
      h("div", { class: "inline-group" }, button("New project", projects.create, "primary", "project"), button("Reload evidence", actions.refresh, "secondary", "refresh"))),
    sectionHeader("Candidates", tag("0")), empty("No candidates"));
}
export function renderProjectSettings(project: ProjectEntry, opened: OpenedProject | undefined, actions: Actions, projects: ProjectActions): HTMLElement {
  const workspace = opened?.content.state === "ready" ? opened.content.workspace : undefined;
  const managed = project.source.kind === "folder" && !!project.source.workspaceId;
  const entries: [string, Child][] = [["Name", project.name], ["Project ID", copyField(project.id, actions.copy)], ["Folder", project.source.kind === "folder" ? copyField(project.source.path, actions.copy) : "Recorded example · no connected folder"]];
  if (workspace) entries.push(["Task", workspace.task.replaceAll("_", " ")], ["Models", `${candidateRows(workspace).length} candidates + 1 baseline`], ["Runs", String(workspace.runs.length)]);
  return h("div", { class: "page-content" }, pageHeader("Project settings"),
    h("section", { class: "project-info" }, sectionHeader(project.name, tag(managed ? "Managed project" : project.source.kind === "folder" ? "Legacy connection" : "Recorded example")), facts(entries),
      h("div", { class: "inline-group settings-actions" }, button("Rename project", projects.rename, "secondary"), project.source.kind === "folder" ? button("Locate folder", projects.relocate, "secondary", "project") : null)),
    h("section", { class: "project-info" }, sectionHeader("Evidence source", tag(!workspace ? opened?.content.state === "error" ? "Unavailable" : "No records yet" : workspace.source === "local" ? "Local journals" : "Recorded snapshot")),
      workspace ? facts([["Read / captured", `${dateLabel(workspace.capturedAt)} at ${timeLabel(workspace.capturedAt)}`], ["Databases", h("div", { class: "stack" }, ...workspace.databases.map(file => h("code", {}, file)))]]): opened?.content.state === "error" ? failureNotice(opened.content.message) : tag(managed ? "Loading" : "Empty"),
      button("Reload evidence", actions.refresh, "secondary", "refresh")),
    h("section", { class: "remove-project" }, h("h2", {}, "Remove from this app"), button("Remove project entry…", projects.forget, "secondary")),
  );
}
