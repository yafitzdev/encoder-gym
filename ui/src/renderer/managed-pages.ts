import type { ManagedWorkspace } from "../managed-workspace.js";
import type { Actions, ProjectActions } from "./actions.js";
import type { ProjectEntry } from "../projects.js";
import { bytesLabel, dateLabel, displayPath } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, tag } from "./components.js";
import { h } from "./dom.js";

export function renderDatasets(workspace: ManagedWorkspace, actions: Actions, projects: ProjectActions): HTMLElement {
  const rows = workspace.datasets.reduce((total, dataset) => total + dataset.rows, 0);
  return h("div", { class: "page-content" }, pageHeader("Datasets", `${workspace.datasets.length} imported ${workspace.datasets.length === 1 ? "source" : "sources"} · ${rows.toLocaleString()} records`, button("Import dataset", projects.importDataset, "primary", "project")),
    !workspace.datasets.length ? empty("Add your first dataset", "Import a local JSONL file. Gym preserves its original structure and stores an independent copy inside this project.", button("Choose a dataset", projects.importDataset, "primary")) :
      h("div", { class: "dataset-list" }, ...workspace.datasets.map(dataset => h("section", { class: "dataset-card", "data-dataset-id": dataset.id },
        sectionHeader(dataset.name, tag(dataset.purpose === "unassigned" ? "Purpose unassigned" : dataset.purpose === "training" ? "Training source" : dataset.purpose)),
        h("p", { class: "dataset-summary" }, `${dataset.rows.toLocaleString()} records · ${bytesLabel(dataset.artifact.bytes)} · JSONL · imported ${dateLabel(dataset.createdAt)}`),
        dataset.trainingSource ? h("p", { class: "section-note" }, "Final-stage input named by this baseline's training manifest. Native rows preserved; not the model's complete pretraining history.") : null,
        details("Source and provenance", facts([
          ["Original source", copyField(displayPath(dataset.source), actions.copy)], ["Managed copy", copyField(dataset.artifact.path, actions.copy)],
          ["Content identity", copyField(dataset.artifact.fingerprint, actions.copy)], ["Imported dataset ID", copyField(dataset.id, actions.copy)],
          ...(dataset.trainingSource ? [["Baseline", copyField(dataset.trainingSource.baselineFingerprint, actions.copy)], ["Training manifest", copyField(dataset.trainingSource.manifestFingerprint, actions.copy)]] as [string, HTMLElement][] : []),
        ])),
      ))),
    h("section", { class: "reading-note" }, h("h2", {}, "Imported data is not a training snapshot"), h("p", {}, "These are source files with recorded provenance. Dataset admission, reproducible splits, and evaluation contracts remain separate setup steps. No training, evaluation, or sealed-evidence disclosure happens on this page.")),
  );
}

export function renderManagedSettings(project: ProjectEntry, workspace: ManagedWorkspace, actions: Actions, projects: ProjectActions): HTMLElement {
  return h("div", { class: "page-content" }, pageHeader("Project settings", "A Gym-owned workspace. Original import sources are not required to reopen it."),
    h("section", { class: "project-info" }, sectionHeader(project.name, tag("Managed project", "accent")), facts([
      ["Library name", project.name], ["Created as", workspace.manifest.name], ["Project ID", copyField(workspace.manifest.id, actions.copy)],
      ["Workspace folder", copyField(displayPath(workspace.folder), actions.copy)], ["Task description", workspace.manifest.task ?? "Not specified at creation"],
      ["Created", dateLabel(workspace.manifest.createdAt)],
    ]), h("div", { class: "inline-group settings-actions" }, button("Rename project", projects.rename, "secondary"), button("Locate folder", projects.relocate, "secondary", "project"))),
    h("section", { class: "project-info" }, sectionHeader("Artifact integrity", tag(workspace.verified ? "Checksums verified" : "Inventory checked")),
      h("p", { class: "section-note" }, workspace.verified ? "Baseline and dataset copies matched their content identities during this verification." : "Opening checks the project/database binding and artifact sizes. Verify files to recalculate every checksum and dataset record count."),
      button("Verify project files", projects.verify, "secondary", "check")),
    h("section", { class: "reading-note" }, h("h2", {}, "Setup before training"), h("p", {}, "Your baseline is stored locally. Imported sources still need admission into a task-compatible training snapshot, and experiments need explicit evaluation rules. Model import alone does not configure a trainer or authorize execution."),
      h("p", {}, "The existing slice CLIs own those scientific contracts. project.sqlite is the workspace registry—not a training-run database. No run controls are shown until that execution integration is configured.")),
    h("section", { class: "remove-project" }, h("h2", {}, "Remove from this app"), h("p", { class: "section-note" }, "Only the library entry is removed. The workspace, checkpoint, datasets and run folders stay on disk."), button("Remove project entry…", projects.forget, "secondary")),
  );
}
