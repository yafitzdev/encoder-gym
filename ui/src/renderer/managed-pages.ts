import type { ManagedWorkspace } from "../managed-workspace.js";
import type { Actions, ProjectActions } from "./actions.js";
import type { ProjectEntry } from "../projects.js";
import { bytesLabel, dateLabel, displayPath } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tag } from "./components.js";
import { h } from "./dom.js";

export function renderDatasets(workspace: ManagedWorkspace, actions: Actions, projects: ProjectActions): HTMLElement {
  const rows = workspace.datasets.reduce((total, dataset) => total + dataset.rows, 0);
  return h("div", { class: "page-content" }, pageHeader("Datasets", `${workspace.datasets.length} imported ${workspace.datasets.length === 1 ? "source" : "sources"} · ${rows.toLocaleString()} records`, workspace.datasets.length ? button("Import dataset", projects.importDataset, "primary", "project") : null),
    !workspace.datasets.length ? empty("Add your first dataset", "Import a local JSONL file. Gym preserves its original structure and stores an independent copy inside this project.", button("Import dataset", projects.importDataset, "primary")) :
      h("div", { class: "dataset-list", "aria-label": "Imported datasets" }, ...workspace.datasets.map(dataset => h("section", { class: "dataset-card", "data-dataset-id": dataset.id },
        h("div", { class: "dataset-row" }, h("div", { class: "dataset-identity" }, h("h2", {}, dataset.name),
          h("p", { class: "dataset-summary" }, `${dataset.rows.toLocaleString()} records · ${bytesLabel(dataset.artifact.bytes)} · JSONL`)),
          h("div", { class: "dataset-purpose" }, h("span", { class: "field-caption" }, "Intended use"), tag(dataset.purpose === "unassigned" ? "Not assigned" : dataset.purpose === "training" ? "Training" : dataset.purpose === "sealed" ? "Sealed holdout" : "Development"))),
        details("File details and provenance", h("div", {},
          dataset.trainingSource ? h("p", { class: "section-note" }, "Named in the baseline's final-stage training manifest. This records the files available at import time, not the model's full pretraining history or proof of historical file contents.") : null,
          facts([
          ["Imported", dateLabel(dataset.createdAt)],
          ["Original source", copyField(displayPath(dataset.source), actions.copy)], ["Managed copy", copyField(dataset.artifact.path, actions.copy)],
          ["Content identity", copyField(dataset.artifact.fingerprint, actions.copy)], ["Imported dataset ID", copyField(dataset.id, actions.copy)],
          ...(dataset.trainingSource ? [["Baseline", copyField(dataset.trainingSource.baselineFingerprint, actions.copy)], ["Training manifest", copyField(dataset.trainingSource.manifestFingerprint, actions.copy)]] as [string, HTMLElement][] : []),
        ]))),
      ))),
    h("section", { class: "preparation-note" }, h("h2", {}, "Source files, not training-ready datasets"),
      h("p", {}, "Import keeps a local copy. Approving records and creating a reproducible training snapshot are separate CLI steps, not available in this desktop yet."),
      details("What preparation involves", h("p", {}, "Use the existing task-compatible dataset admission, snapshot, and evaluation contracts before training. Declaring a file's purpose does not approve its records, create splits, or configure a trainer. This page never runs training or evaluation."))),
  );
}

export function renderManagedSettings(project: ProjectEntry, workspace: ManagedWorkspace, actions: Actions, projects: ProjectActions): HTMLElement {
  return h("div", { class: "page-content settings-page" }, pageHeader("Project settings", "Manage this project's name, location, and stored files."),
    h("section", { class: "project-info" }, sectionHeader(project.name, tag("Managed project", "accent")), facts([
      ["Workspace folder", copyField(displayPath(workspace.folder), actions.copy)], ["Task", workspace.manifest.task ?? "No task description"],
    ]), h("div", { class: "inline-group settings-actions" }, button("Rename project", projects.rename, "secondary"), button("Locate folder", projects.relocate, "secondary", "project")),
      h("p", { class: "field-help" }, "Moved the workspace? Locate its new folder. Original import sources are not needed to reopen it."),
      details("Project identity", facts([["Created as", workspace.manifest.name], ["Created", dateLabel(workspace.manifest.createdAt)], ["Project ID", copyField(workspace.manifest.id, actions.copy)]]))),
    h("section", { class: "project-info" }, sectionHeader("Check stored files", tag(workspace.verified ? "Checksums verified" : "File sizes checked")),
      h("p", { class: "section-note" }, workspace.verified ? "Baseline and dataset files matched their recorded checksums and counts during this verification." : "Opening checked file sizes and project metadata. Verify to check the contents of every baseline and dataset file."),
      button("Verify project files", projects.verify, "secondary", "check")),
    details("Training and execution availability", h("div", { class: "reading-note" }, h("p", {}, "Importing a checkpoint does not configure a trainer. Dataset admission, snapshots, training, and evaluation remain explicit task-compatible CLI workflows; this desktop does not launch them or import their history automatically."),
      h("p", {}, "project.sqlite stores this workspace's file registry. It is not a training-run database and must not be used as the legacy CLI's --database-url."))),
    h("section", { class: "remove-project" }, h("h2", {}, "Remove from this app"), h("p", { class: "section-note" }, "Only the library entry is removed. The workspace, checkpoint, datasets and run folders stay on disk."), button("Remove project entry…", projects.forget, "secondary")),
  );
}
