import type { EncoderGymBridge } from "../preload.js";
import type { ProjectCollection } from "../projects.js";
import type { FolderChoice, ModelChoice } from "../managed-workspace.js";
import { bytesLabel, displayPath } from "./catalog.js";
import { button, details, failureNotice, tag } from "./components.js";
import { h } from "./dom.js";

function dialogState(dialog: HTMLDialogElement) {
  let busy = false;
  const fields = h("fieldset", { class: "onboarding-fields" }) as HTMLFieldSetElement;
  const footer = h("fieldset", { class: "onboarding-footer" }) as HTMLFieldSetElement;
  const error = h("div", { class: "form-error", role: "alert" });
  const progress = h("div", { class: "operation-status", role: "status", "aria-live": "polite" });
  dialog.oncancel = event => { if (busy) event.preventDefault(); };
  return { fields, footer, error, progress, async run(operation: () => Promise<void>, description: string) {
    if (busy) return;
    const focused = document.activeElement as HTMLElement | null;
    busy = true; fields.disabled = true; footer.disabled = true;
    error.replaceChildren(); progress.textContent = description;
    dialog.setAttribute("aria-busy", "true");
    try { await operation(); } catch (failure) { error.replaceChildren(failureNotice(failure)); }
    finally {
      busy = false; fields.disabled = false; footer.disabled = false; progress.textContent = "";
      dialog.removeAttribute("aria-busy");
      if (dialog.open && focused?.isConnected) focused.focus({ preventScroll: true });
    }
  } };
}
function field(label: string, id: string, placeholder: string, required = true): [HTMLElement, HTMLInputElement] {
  const input = h("input", { class: "text-input", id, maxlength: "120", required, placeholder }) as HTMLInputElement;
  return [h("label", { class: "form-field", for: id }, label, input), input];
}

export function newProjectDialog(dialog: HTMLDialogElement, bridge: EncoderGymBridge, created: (collection: ProjectCollection) => Promise<void>): void {
  const state = dialogState(dialog);
  let model: ModelChoice | null = null, parent: FolderChoice | null = null, editedFolder = false;
  const [nameField, name] = field("Project name", "new-project-name", "e.g. Support encoder");
  const [folderField, folderName] = field("New folder name", "new-project-folder", "support-encoder");
  const [taskField, task] = field("Task description (optional)", "new-project-task", "e.g. Rank support articles", false);
  const modelPreview = h("div", { id: "model-preview", class: "import-preview" }, "No checkpoint selected.");
  const destination = h("div", { id: "project-parent-path", class: "path-note" }, "No location selected");
  const hint = h("div", { id: "confirm-new-project-hint", class: "confirmation-hint" });
  const confirm = button("Create project", () => {}, "primary"); confirm.id = "confirm-new-project"; confirm.type = "submit";
  confirm.setAttribute("aria-describedby", hint.id);
  const update = () => {
    confirm.disabled = !model || !parent || !name.value.trim() || !folderName.value.trim();
    hint.textContent = !name.value.trim() ? "Project name required" : !model ? "Checkpoint required" : !parent ? "Location required" : !folderName.value.trim() ? "Folder name required" : bytesLabel(model.model.bytes);
    destination.textContent = parent ? displayPath(parent.path).replace(/[\\/]+$/, "") + "\\" + (folderName.value.trim() || "…") : "No location selected.";
  };
  folderName.addEventListener("input", () => { editedFolder = true; update(); });
  name.addEventListener("input", () => { if (!editedFolder) folderName.value = name.value.trim().replace(/[\\/:<>"|?*]/g, "-"); update(); });
  const chooseModel = button("Choose checkpoint…", () => { void state.run(async () => {
    try {
      const choice = await bridge.chooseLocalModel();
      if (choice) {
        model = choice;
        modelPreview.replaceChildren(h("div", { class: "inline-group" }, tag(choice.model.architecture.toUpperCase()), tag(choice.model.format), h("strong", {}, bytesLabel(choice.model.bytes) + " · " + choice.model.files.length + " files")),
          details("Checkpoint location", h("div", { class: "path-note" }, displayPath(choice.model.source))));
      }
    } catch (error) { model = null; modelPreview.textContent = "No valid checkpoint selected."; throw error; }
    finally { update(); }
  }, "Inspecting checkpoint…"); }, "secondary", "models");
  chooseModel.id = "choose-local-model";
  const chooseParent = button("Choose location…", () => { void state.run(async () => {
    const choice = await bridge.chooseProjectParent(); if (choice) parent = choice; update();
  }, "Choosing location…"); }, "secondary", "project");
  chooseParent.id = "choose-project-parent";
  state.fields.append(nameField,
    h("section", { class: "onboarding-section" }, h("h3", {}, "Starting model"), chooseModel, modelPreview),
    h("section", { class: "onboarding-section" }, h("h3", {}, "Save project"), chooseParent, folderField, h("span", { class: "field-caption" }, "New project folder"), destination),
    details("Add a task description", taskField));
  state.footer.append(state.progress, state.error, hint, h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "secondary"), confirm));
  const form = h("form", { class: "onboarding-form", onSubmit: (event: Event) => {
    event.preventDefault(); if (!model || !parent) return;
    const request = { name: name.value, folderName: folderName.value, modelToken: model.token, parentToken: parent.token, task: task.value };
    void state.run(async () => { const collection = await bridge.createManagedProject(request); dialog.close(); await created(collection); }, "Creating project…");
  } }, h("header", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "New encoder project")), state.fields, state.footer);
  update(); dialog.replaceChildren(form); dialog.showModal(); name.focus();
}
