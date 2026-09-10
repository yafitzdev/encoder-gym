import type { ManagedWorkspace } from "../managed-workspace.js";
import type { ManagedProviderStatus, ManagedReadiness, ProviderRole } from "../managed-control.js";
import type { Actions, ProjectActions } from "./actions.js";
import type { ProjectEntry } from "../projects.js";
import { bytesLabel, dateLabel, displayPath } from "./catalog.js";
import { button, copyField, details, empty, facts, sectionHeader, status, tag, workspacePage } from "./components.js";
import { h } from "./dom.js";

export function renderDatasets(workspace: ManagedWorkspace, actions: Actions, projects: ProjectActions, readiness?: ManagedReadiness): HTMLElement {
  const preview = readiness?.preparedOptimization?.launchPreview ?? readiness?.launchPreview;
  const authority = readiness?.preparedOptimization?.authority ?? readiness?.optimizationAuthority;
  const trainingSnapshotId = preview?.trainingSnapshotId ?? authority?.trainingSnapshotId;
  const snapshot = trainingSnapshotId ? h("section", { class: "scientific-data-card" },
      sectionHeader(preview ? "Frozen training snapshot" : "Approved training snapshot", tag(preview ? preview.existingRun ? "Used by run" : "Prepared" : "Approved", "accent")),
      authority ? h("div", { class: "snapshot-summary" },
        h("div", {}, h("strong", {}, authority.deltaRows.toLocaleString()), h("span", {}, "approved repair rows")),
        h("div", {}, h("strong", {}, authority.baseTrainingInputs.toLocaleString()), h("span", {}, authority.baseTrainingInputs === 1 ? "base input artifact" : "base input artifacts")),
        h("div", {}, h("strong", {}, String(authority.candidateCount)), h("span", {}, authority.candidateCount === 1 ? "bounded candidate" : "bounded candidates"))) : null,
      details("Snapshot authority and usage", facts([
        ["Training snapshot", copyField(trainingSnapshotId, actions.copy)],
        ...(preview ? [["Prepared run", preview.runName], ["Benchmark generation", copyField(preview.benchmarkGenerationId, actions.copy)], ["Use", preview.existingRun ? `Optimization run ${preview.existingRun.runId}` : "Prepared definition; no run reserved"]] as [string, string | HTMLElement][] : []),
        ...(authority ? [["Repair selection", copyField(authority.selectionId, actions.copy)], ["Authority valid until", dateLabel(authority.validUntil)]] as [string, string | HTMLElement][] : []),
      ])),
      button(preview?.existingRun ? "Open optimization run" : "Review optimization", actions.prepareOptimization, "secondary", "arrow")) : null;
  const dataActions = h("div", { class: "inline-group" }, tag(trainingSnapshotId ? `${preview ? "1 frozen" : "1 approved"} snapshot` : "No snapshot"), tag(`${workspace.datasets.length} ${workspace.datasets.length === 1 ? "source" : "sources"}`), workspace.datasets.length ? button("Import dataset", projects.importDataset, "primary", "project") : null);
  return workspacePage("Data", dataActions,
    snapshot,
    sectionHeader("Imported sources"),
    !workspace.datasets.length ? trainingSnapshotId ? h("section", { class: "source-empty" }, h("h3", {}, "No imported sources"), button("Import dataset", projects.importDataset, "secondary")) : empty("No imported sources", button("Import dataset", projects.importDataset, "primary")) :
      h("div", { class: "dataset-list", "aria-label": "Imported datasets" }, ...workspace.datasets.map(dataset => h("section", { class: "dataset-card", "data-dataset-id": dataset.id },
        h("div", { class: "dataset-row" }, h("div", { class: "dataset-identity" }, h("h2", {}, dataset.name),
          h("div", { class: "dataset-summary" }, `${dataset.rows.toLocaleString()} records · ${bytesLabel(dataset.artifact.bytes)} · JSONL`)),
          h("div", { class: "dataset-purpose" }, h("span", { class: "field-caption" }, "Intended use"), tag(dataset.purpose === "unassigned" ? "Not assigned" : dataset.purpose === "training" ? "Training" : dataset.purpose === "sealed" ? "Sealed holdout" : "Development"))),
        details("File details and provenance", facts([
          ["Imported", dateLabel(dataset.createdAt)],
          ["Original source", copyField(displayPath(dataset.source), actions.copy)], ["Managed copy", copyField(dataset.artifact.path, actions.copy)],
          ["Content identity", copyField(dataset.artifact.fingerprint, actions.copy)], ["Imported dataset ID", copyField(dataset.id, actions.copy)],
          ...(dataset.trainingSource ? [["Baseline", copyField(dataset.trainingSource.baselineFingerprint, actions.copy)], ["Training manifest", copyField(dataset.trainingSource.manifestFingerprint, actions.copy)]] as [string, HTMLElement][] : []),
        ])),
      ))),
  );
}

export interface ProviderPageState { loading: boolean; error?: string; status?: ManagedProviderStatus }
export interface ProviderPageActions { configure(): void; refresh(): void; remove(role: ProviderRole): void }
export interface ScientificRuntimeActions { configure(): void; upgrade(): void }

export function renderManagedSettings(project: ProjectEntry, workspace: ManagedWorkspace, providerState: ProviderPageState, providerActions: ProviderPageActions, runtimeActions: ScientificRuntimeActions, actions: Actions, projects: ProjectActions): HTMLElement {
  const binding = workspace.scientificBinding, providers = providerState.status?.catalog ?? workspace.providerCatalog;
  const configureProviders = button(providers ? "Edit provider setup" : "Configure providers", providerActions.configure, "secondary");
  configureProviders.disabled = providerState.loading;
  return workspacePage("Project settings", null,
    h("section", { class: "project-info" }, sectionHeader("Scientific runtime", tag(binding ? binding.store.snapshotFingerprint ? "History connected" : "Connected" : "Not connected", binding ? "accent" : undefined)),
      binding ? h("div", {}, facts([["Task adapter", binding.adapter.key], ["Protocol", binding.adapter.protocol], ["Baseline revision", binding.baselineRevisionId], ...(binding.store.snapshotBytes ? [["Imported history", bytesLabel(binding.store.snapshotBytes)]] as [string, string][] : [])]),
        details("Runtime and store identity", facts([["Runtime", copyField(displayPath(binding.runtime.location), actions.copy)], ["Scientific store", binding.store.databasePath], ...(binding.store.snapshotFingerprint ? [["History snapshot", copyField(binding.store.snapshotFingerprint, actions.copy)]] as [string, HTMLElement][] : []), ["Binding", copyField(binding.id, actions.copy)]])), h("div", { class: "inline-group settings-actions" }, button("Reverify or change runtime", runtimeActions.configure, "secondary"))) :
        workspace.modelCatalog ? button("Connect scientific runtime", runtimeActions.configure, "primary") : button("Initialize model history", runtimeActions.upgrade, "primary")),
    h("section", { class: "project-info" }, sectionHeader("Provider authorities", tag(providers ? `${providers.providers.length} configured` : "Not configured")),
      providerState.error ? h("p", { class: "form-error", role: "alert" }, providerState.error) : null,
      providers ? h("div", {}, ...providers.providers.map(provider => {
        const credential = providerState.status?.credentialAvailability.find(item => item.role === provider.role);
        const availability = credential?.availability ?? (provider.authentication === "none" ? "available" : "unavailable");
        return h("div", { class: "provider-summary" }, h("div", {}, h("strong", {}, provider.role === "generation" ? "Data generation" : provider.role === "advisor" ? "Agentic work" : "Evaluation"), h("small", {}, provider.endpoint ?? provider.kind)),
          h("div", { class: "provider-state" }, status(availability === "available" ? credential?.source === "environment" ? "Available from environment" : "Credential available" : availability === "missing" ? "Credential missing" : "Availability unknown", availability === "available" ? "success" : availability === "missing" ? "warning" : "neutral"),
            credential?.source === "credential_store" ? button("Remove saved key", () => providerActions.remove(provider.role), "ghost small") : null));
      })) : null,
      h("div", { class: "inline-group settings-actions" }, configureProviders, button(providerState.loading ? "Checking…" : "Refresh availability", providerActions.refresh, "ghost", "refresh"))),
    h("section", { class: "project-info" }, sectionHeader(project.name, tag("Managed project", "accent")), facts([
      ["Workspace folder", copyField(displayPath(workspace.folder), actions.copy)], ["Task", workspace.manifest.task ?? "No task description"],
    ]), h("div", { class: "inline-group settings-actions" }, button("Rename project", projects.rename, "secondary"), button("Locate folder", projects.relocate, "secondary", "project")),
      details("Project identity", facts([["Created as", workspace.manifest.name], ["Created", dateLabel(workspace.manifest.createdAt)], ["Project ID", copyField(workspace.manifest.id, actions.copy)]]))),
    h("section", { class: "project-info" }, sectionHeader("Check stored files", tag(workspace.verified ? "Checksums verified" : "File sizes checked")),
      button("Verify project files", projects.verify, "secondary", "check")),
    h("section", { class: "remove-project" }, h("h2", {}, "Remove from this app"), button("Remove project entry…", projects.forget, "secondary")),
  );
}
