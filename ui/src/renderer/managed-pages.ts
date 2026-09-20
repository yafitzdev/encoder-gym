import type { ManagedWorkspace } from "../managed-workspace.js";
import type { ManagedProviderStatus } from "../managed-control.js";
import type { ProjectProviderConnections, ProviderAssignmentRequest, AssignableProviderRole } from "../provider-connections.js";
import type { Actions, ProjectActions } from "./actions.js";
import type { ProjectEntry } from "../projects.js";
import { bytesLabel, dateLabel, displayPath } from "./catalog.js";
import { button, copyField, details, facts, sectionHeader, status, tag, workspacePage } from "./components.js";
import { h } from "./dom.js";

export interface ProviderPageState { loading: boolean; error?: string; status?: ManagedProviderStatus; connections?: ProjectProviderConnections }
export interface ProviderPageActions {
  add(): void;
  refresh(): void;
  refreshConnection(id: string): void;
  removeConnection(id: string): void;
  assign(value: ProviderAssignmentRequest): void;
}
export interface ScientificRuntimeActions { configure(): void; upgrade(): void }

export function renderManagedSettings(project: ProjectEntry, workspace: ManagedWorkspace, providerState: ProviderPageState, providerActions: ProviderPageActions, runtimeActions: ScientificRuntimeActions, actions: Actions, projects: ProjectActions): HTMLElement {
  const binding = workspace.scientificBinding, connections = providerState.connections;
  const addProvider = button("Add provider", providerActions.add, "primary"); addProvider.disabled = providerState.loading;
  const connectionLabel = (endpoint: string, id: string): string => {
    const base = providerName(endpoint), matching = connections?.connections.filter(connection => providerName(connection.endpoint) === base) ?? [];
    return matching.length > 1 ? `${base} ${matching.findIndex(connection => connection.id === id) + 1}` : base;
  };
  const modelOptions = connections?.connections.flatMap(connection => connection.models.map(model => ({ connection, model, value: `${connection.id}:${encodeURIComponent(model)}` }))) ?? [];
  const assignmentValue = (role: AssignableProviderRole): string => {
    const selected = connections?.assignments[role];
    return selected ? `${selected.connectionId}:${encodeURIComponent(selected.model)}` : "";
  };
  let advisorValue = assignmentValue("advisor"), generationValue = assignmentValue("generation");
  const parseAssignment = (value: string) => {
    const split = value.indexOf(":");
    if (split < 0) throw new Error("Choose a discovered provider model.");
    return { connectionId: value.slice(0, split), model: decodeURIComponent(value.slice(split + 1)) };
  };
  const assignmentSelect = (role: AssignableProviderRole, label: string, value: string, change: (next: string) => void) => h("label", { class: "provider-assignment", for: `provider-assignment-${role}` }, h("span", {}, label),
    h("select", { id: `provider-assignment-${role}`, value, disabled: providerState.loading || !modelOptions.length, onChange: (event: Event) => change((event.target as HTMLSelectElement).value) },
      h("option", { value: "" }, "Choose model"), ...modelOptions.map(option => h("option", { value: option.value }, `${connectionLabel(option.connection.endpoint, option.connection.id)} · ${option.model}`))));
  const saveAssignments = button("Save assignments", () => providerActions.assign({ advisor: parseAssignment(advisorValue), generation: parseAssignment(generationValue) }), "primary");
  saveAssignments.disabled = providerState.loading || !advisorValue || !generationValue;
  return workspacePage("Project settings", null,
    h("section", { class: "project-info" }, sectionHeader("Scientific runtime", tag(binding ? binding.runtime.kind === "managed" ? "Managed" : binding.store.snapshotFingerprint ? "History connected" : "External (migration needed)" : "Not connected", binding ? "accent" : undefined)),
      binding ? h("div", {}, facts([["Task adapter", binding.adapter.key], ["Protocol", binding.adapter.protocol], ["Baseline revision", binding.baselineRevisionId], ["Custody", binding.runtime.kind === "managed" ? "Contained in this project" : "Depends on an external checkout"], ...(binding.store.snapshotBytes ? [["Imported history", bytesLabel(binding.store.snapshotBytes)]] as [string, string][] : [])]),
        details("Runtime and store identity", facts([["Runtime", copyField(displayPath(binding.runtime.location), actions.copy)], ...(binding.runtime.package ? [["Package", copyField(binding.runtime.package.fingerprint, actions.copy)]] as [string, HTMLElement][] : []), ["Scientific store", binding.store.databasePath], ...(binding.store.snapshotFingerprint ? [["History snapshot", copyField(binding.store.snapshotFingerprint, actions.copy)]] as [string, HTMLElement][] : []), ["Binding", copyField(binding.id, actions.copy)]])), h("div", { class: "inline-group settings-actions" }, button(binding.runtime.kind === "managed" ? "Reverify or replace package" : "Copy runtime into project", runtimeActions.configure, "secondary"))) :
        workspace.modelCatalog ? button("Connect scientific runtime", runtimeActions.configure, "primary") : button("Initialize model history", runtimeActions.upgrade, "primary")),
    h("section", { class: "project-info" }, sectionHeader("LLM connections", tag(`${connections?.connections.length ?? 0}`)),
      providerState.error ? h("p", { class: "form-error", role: "alert" }, providerState.error) : null,
      connections?.connections.length ? h("div", { class: "provider-connections" }, ...connections.connections.map(connection => {
        const assigned = Object.values(connections.assignments).some(value => value?.connectionId === connection.id);
        const remove = button("Remove", () => providerActions.removeConnection(connection.id), "ghost small"); remove.disabled = providerState.loading || assigned;
        return h("div", { class: "provider-summary" }, h("div", {}, h("strong", {}, connectionLabel(connection.endpoint, connection.id)), h("small", {}, connection.endpoint)),
          h("div", { class: "provider-state" }, tag(`${connection.models.length} models`), status(connection.availability === "available" ? "Key saved" : "Key unavailable", connection.availability === "available" ? "success" : "warning"),
            button("Refresh models", () => providerActions.refreshConnection(connection.id), "ghost small", "refresh"), remove));
      })) : null,
      h("div", { class: "inline-group settings-actions" }, addProvider, button(providerState.loading ? "Loading…" : "Refresh", providerActions.refresh, "ghost", "refresh")),
      modelOptions.length ? h("div", { class: "provider-assignments" }, sectionHeader("Use for optimization"), assignmentSelect("advisor", "Agent", advisorValue, value => { advisorValue = value; saveAssignments.disabled = !advisorValue || !generationValue; }), assignmentSelect("generation", "Data generation", generationValue, value => { generationValue = value; saveAssignments.disabled = !advisorValue || !generationValue; }), h("div", { class: "provider-assignment-save" }, saveAssignments)) : null),
    h("section", { class: "project-info" }, sectionHeader(project.name, tag("Managed project", "accent")), facts([
      ["Workspace folder", copyField(displayPath(workspace.folder), actions.copy)], ["Task", workspace.manifest.task ?? "No task description"],
    ]), h("div", { class: "inline-group settings-actions" }, button("Rename project", projects.rename, "secondary"), button("Locate folder", projects.relocate, "secondary", "project")),
      details("Project identity", facts([["Created as", workspace.manifest.name], ["Created", dateLabel(workspace.manifest.createdAt)], ["Project ID", copyField(workspace.manifest.id, actions.copy)]]))),
    h("section", { class: "project-info" }, sectionHeader("Check stored files", tag(workspace.verified ? "Checksums verified" : "File sizes checked")),
      button("Verify project files", projects.verify, "secondary", "check")),
    h("section", { class: "remove-project" }, h("h2", {}, "Remove from this app"), button("Remove project entry…", projects.forget, "secondary")),
  );
}

function providerName(endpoint: string): string {
  try { const host = new URL(endpoint).hostname; return host === "api.deepseek.com" ? "DeepSeek" : host === "yan.tail85512d.ts.net" ? "Yan" : host; }
  catch { return "Provider"; }
}
