import type { ManagedWorkspace } from "../managed-workspace.js";
import type { ManagedProviderStatus, ProviderRole } from "../managed-control.js";
import type { Actions, ProjectActions } from "./actions.js";
import type { ProjectEntry } from "../projects.js";
import { bytesLabel, dateLabel, displayPath } from "./catalog.js";
import { button, copyField, details, facts, sectionHeader, status, tag, workspacePage } from "./components.js";
import { h } from "./dom.js";

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
