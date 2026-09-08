import type { OpenedProject, ProjectEntry } from "../projects.js";
import type { Actions, ProjectActions } from "./actions.js";
import { candidateRows, dateLabel, timeLabel } from "./catalog.js";
import { button, copyField, empty, facts, icon, pageHeader, sectionHeader, tag } from "./components.js";
import { h, type Child } from "./dom.js";

export function renderWelcome(actions: ProjectActions): HTMLElement {
  return h("div", { class: "page-content welcome-page" },
    h("div", { class: "eyebrow" }, "Your encoder workspace"),
    pageHeader("Start with an encoder", "Each project owns its baseline, datasets, and experiment history."),
    h("div", { class: "welcome-action" }, icon("models"), h("div", {}, h("h2", {}, "Create a new project"), h("p", {}, "Choose a local checkpoint. Gym creates a workspace and copies the baseline into it. No training starts.")), button("New project", actions.create, "primary", "project")),
    h("div", { class: "welcome-action" }, icon("project"), h("div", {}, h("h2", {}, "Continue an existing project"), h("p", {}, "Open a folder created by Encoder Gym—not a source-code repository.")), button("Open project", actions.openManaged, "secondary", "project")),
    h("p", { class: "welcome-example" }, "Want to explore first? ", h("button", { type: "button", id: "open-recorded-example", class: "inline-link", onClick: actions.openExample }, "Open a recorded example →")),
  );
}
export function renderProjectState(project: ProjectEntry, opened: OpenedProject | undefined, actions: Actions, projects: ProjectActions, page = "models"): HTMLElement {
  if (!opened) return h("div", { class: "page-content", role: "status" }, pageHeader(project.name, "Reading this project's evidence…"));
  const content = opened.content;
  if (content.state === "error") return h("div", { class: "page-content" }, pageHeader(project.name, "Project evidence is unavailable."),
    empty("Couldn't read this project", content.message, h("div", { class: "inline-group" }, button("Try again", actions.refresh, "primary", "refresh"), project.source.kind === "folder" ? button("Locate folder", projects.relocate, "secondary", "project") : null, button("Project settings", () => actions.navigate({ page: "project" }), "ghost"))),
    h("p", { class: "table-footnote" }, "No evidence from another project is displayed. Your project files have not been changed."));
  if (page === "runs" || page === "benchmarks") return h("div", { class: "page-content" }, pageHeader(page === "runs" ? "Runs" : "Benchmarks", project.name),
    empty(page === "runs" ? "No runs recorded yet" : "No benchmark results recorded yet", "This project has not recorded experiment evidence. Start with its baseline setup.", button("Open model setup", () => actions.navigate({ page: "models" }), "primary")));
  return h("div", { class: "page-content" }, pageHeader("Models", `${project.name} · baseline and candidates`),
    h("section", { class: "empty-baseline" }, icon("models"), h("div", {}, h("div", { class: "eyebrow" }, "Legacy folder connection"), h("h2", {}, "No baseline recorded yet"), h("p", {}, "This external folder is not a Gym-managed workspace. Its baseline appears only if a supported experiment journal already exists."))),
    h("section", { class: "setup-section" }, sectionHeader("Set up this project"),
      h("p", { class: "section-note" }, "Connect a folder containing supported encoder experiment journals, or prepare one through a compatible CLI adapter. Reload after preparation. Folder registration itself does not create a model, dataset, or experiment."),
      h("p", { class: "section-note" }, "To start from a local encoder, create a separate managed project. Gym will copy its checkpoint and keep this source folder untouched."),
      h("div", { class: "inline-group" }, button("New project", projects.create, "primary", "project"), button("Reload evidence", actions.refresh, "secondary", "refresh"))),
    sectionHeader("Candidates", tag("0")), h("p", { class: "section-note" }, "Candidates appear here as your experiments create and evaluate them."));
}
export function renderProjectSettings(project: ProjectEntry, opened: OpenedProject | undefined, actions: Actions, projects: ProjectActions): HTMLElement {
  const workspace = opened?.content.state === "ready" ? opened.content.workspace : undefined;
  const entries: [string, Child][] = [["Name", project.name], ["Project ID", copyField(project.id, actions.copy)], ["Folder", project.source.kind === "folder" ? copyField(project.source.path, actions.copy) : "Recorded example · no connected folder"]];
  if (workspace) entries.push(["Task", workspace.task.replaceAll("_", " ")], ["Models", `${candidateRows(workspace).length} candidates + 1 baseline`], ["Runs", String(workspace.runs.length)]);
  return h("div", { class: "page-content" }, pageHeader("Project settings", "Folder organization is separate from immutable experiment records."),
    h("section", { class: "project-info" }, sectionHeader(project.name, tag(project.source.kind === "folder" ? "Local project" : "Recorded example")), facts(entries),
      h("div", { class: "inline-group settings-actions" }, button("Rename project", projects.rename, "secondary"), project.source.kind === "folder" ? button("Locate folder", projects.relocate, "secondary", "project") : null)),
    h("section", { class: "project-info" }, sectionHeader("Evidence source", tag(!workspace ? opened?.content.state === "error" ? "Unavailable" : "No records yet" : workspace.source === "local" ? "Local journals" : "Recorded snapshot")),
      workspace ? h("div", {}, h("p", { class: "section-note" }, workspace.source === "local" ? "Read from this project's local databases. Reload to read newly persisted results." : "Historical example data, not a connection to the original workspace or its current state."), facts([["Read / captured", `${dateLabel(workspace.capturedAt)} at ${timeLabel(workspace.capturedAt)}`], ["Databases", h("div", { class: "stack" }, ...workspace.databases.map(file => h("code", {}, file)))]])) : h("p", { class: "section-note" }, opened?.content.state === "error" ? opened.content.message : "No baseline or experiment records have been found. This is a valid empty project."),
      button("Reload evidence", actions.refresh, "secondary", "refresh")),
    h("section", { class: "reading-note" }, h("h2", {}, "Read-only experiment evidence"), h("p", {}, "Renaming or reconnecting this project changes only the app's folder collection. Training, model registration, and final acceptance remain explicit CLI workflows."), h("p", {}, "The reader checks report bindings and journal continuity. It does not replace CLI Doctor's native artifact verification.")),
    h("section", { class: "remove-project" }, h("h2", {}, "Remove from this app"), h("p", { class: "section-note" }, "Forget this folder entry. No project files, models, datasets, or experiment history will be deleted."), button("Remove project entry…", projects.forget, "secondary")),
  );
}
