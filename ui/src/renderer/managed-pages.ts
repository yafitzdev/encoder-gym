import type { ManagedWorkspace } from "../managed-workspace.js";
import type { ManagedProviderStatus, ProviderRole } from "../managed-control.js";
import type { Actions, ProjectActions } from "./actions.js";
import type { ProjectEntry } from "../projects.js";
import { bytesLabel, dateLabel, displayPath } from "./catalog.js";
import { button, copyField, details, empty, facts, pageHeader, sectionHeader, status, tag } from "./components.js";
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

export interface ProviderPageState { loading: boolean; error?: string; status?: ManagedProviderStatus }
export interface ProviderPageActions { configure(): void; refresh(): void; remove(role: ProviderRole): void }
export interface ScientificRuntimeActions { configure(): void; upgrade(): void }

export function renderManagedSettings(project: ProjectEntry, workspace: ManagedWorkspace, providerState: ProviderPageState, providerActions: ProviderPageActions, runtimeActions: ScientificRuntimeActions, actions: Actions, projects: ProjectActions): HTMLElement {
  const binding = workspace.scientificBinding, providers = providerState.status?.catalog ?? workspace.providerCatalog;
  const configureProviders = button(providers ? "Edit provider setup" : "Configure providers", providerActions.configure, "secondary");
  configureProviders.disabled = providerState.loading;
  return h("div", { class: "page-content settings-page" }, pageHeader("Project settings", "Runtime, provider authorities, workspace identity, and integrity."),
    h("section", { class: "project-info" }, sectionHeader("Scientific runtime", tag(binding ? binding.store.snapshotFingerprint ? "History connected" : "Connected" : "Not connected", binding ? "accent" : undefined)),
      binding ? h("div", {}, h("p", { class: "section-note" }, binding.store.snapshotFingerprint ? "The verified task runtime uses a contained, content-addressed copy of explicitly selected Encoder Gym history. The original database remains separate and unchanged." : "The task adapter and scientific store are bound to the current baseline revision. Readiness re-verifies their exact identities before every launch."), facts([["Task adapter", binding.adapter.key], ["Protocol", binding.adapter.protocol], ["Baseline revision", binding.baselineRevisionId], ...(binding.store.snapshotBytes ? [["Imported history", bytesLabel(binding.store.snapshotBytes)]] as [string, string][] : [])]),
        details("Runtime and store identity", facts([["Runtime", copyField(displayPath(binding.runtime.location), actions.copy)], ["Scientific store", binding.store.databasePath], ...(binding.store.snapshotFingerprint ? [["History snapshot", copyField(binding.store.snapshotFingerprint, actions.copy)]] as [string, HTMLElement][] : []), ["Binding", copyField(binding.id, actions.copy)]])), h("div", { class: "inline-group settings-actions" }, button("Reverify or change runtime", runtimeActions.configure, "secondary"))) :
        workspace.modelCatalog ? h("div", {}, h("p", { class: "section-note" }, "This project currently owns model and dataset custody only. Connect a verified task runtime before training snapshots, evaluation authority, or optimization runs can exist."), button("Connect scientific runtime", runtimeActions.configure, "primary")) :
          h("div", {}, h("p", { class: "section-note" }, "This older workspace has no immutable model history yet. Initialize it from the already-verified imported baseline before connecting a runtime. Models and datasets are not copied or changed."), button("Initialize model history", runtimeActions.upgrade, "primary"))),
    h("section", { class: "project-info" }, sectionHeader("Provider authorities", tag(providers ? `${providers.providers.length} configured` : "Not configured")),
      providerState.error ? h("p", { class: "form-error", role: "alert" }, providerState.error) : null,
      providers ? h("div", {}, h("p", { class: "section-note" }, "Generation, advisor, and optional evaluator settings are separate. Secret values are never part of this project record."), ...providers.providers.map(provider => {
        const credential = providerState.status?.credentialAvailability.find(item => item.role === provider.role);
        const availability = credential?.availability ?? (provider.authentication === "none" ? "available" : "unavailable");
        return h("div", { class: "provider-summary" }, h("div", {}, h("strong", {}, provider.role[0]!.toUpperCase() + provider.role.slice(1)), h("small", {}, `${provider.kind} · ${provider.model}`)),
          h("div", { class: "provider-state" }, status(availability === "available" ? credential?.source === "environment" ? "Available from environment" : "Credential available" : availability === "missing" ? "Credential missing" : "Availability unknown", availability === "available" ? "success" : availability === "missing" ? "warning" : "neutral"),
            credential?.source === "credential_store" ? button("Remove saved key", () => providerActions.remove(provider.role), "ghost small") : null));
      })) : h("p", { class: "section-note" }, "Generation and advisor providers have not been selected. Configure them separately; checking setup makes no external call."),
      h("div", { class: "inline-group settings-actions" }, configureProviders, button(providerState.loading ? "Checking…" : "Refresh availability", providerActions.refresh, "ghost", "refresh"))),
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
