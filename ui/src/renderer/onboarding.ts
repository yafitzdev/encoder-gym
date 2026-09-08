import type { EncoderGymBridge } from "../preload.js";
import type { ProjectCollection, OpenedProject } from "../projects.js";
import type { DatasetChoice, DatasetPurpose, FolderChoice, ModelChoice } from "../managed-workspace.js";
import { bytesLabel, displayPath } from "./catalog.js";
import { button, details, facts, failureNotice, tag } from "./components.js";
import { h } from "./dom.js";

function dialogState(dialog: HTMLDialogElement) {
  let busy = false;
  const fields = h("fieldset", { class: "onboarding-fields" }) as HTMLFieldSetElement;
  const footer = h("fieldset", { class: "onboarding-footer" }) as HTMLFieldSetElement;
  const error = h("div", { class: "form-error", role: "alert" });
  const progress = h("p", { class: "operation-status", role: "status", "aria-live": "polite" });
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
  const destination = h("p", { id: "project-parent-path", class: "path-note" }, "No location selected.");
  const hint = h("p", { id: "confirm-new-project-hint", class: "confirmation-hint" });
  const confirm = button("Create project", () => {}, "primary"); confirm.id = "confirm-new-project"; confirm.type = "submit";
  confirm.setAttribute("aria-describedby", hint.id);
  const update = () => {
    confirm.disabled = !model || !parent || !name.value.trim() || !folderName.value.trim();
    hint.textContent = !name.value.trim() ? "Name your project to get started." : !model ? "Choose the checkpoint to copy." : !parent ? "Choose where to create the project." : !folderName.value.trim() ? "Enter a name for the new folder." : "Copies " + bytesLabel(model.model.bytes) + " into a new project. No training starts.";
    destination.textContent = parent ? displayPath(parent.path).replace(/[\\/]+$/, "") + "\\" + (folderName.value.trim() || "…") : "No location selected.";
  };
  folderName.addEventListener("input", () => { editedFolder = true; update(); });
  name.addEventListener("input", () => { if (!editedFolder) folderName.value = name.value.trim().replace(/[\\/:<>"|?*]/g, "-"); update(); });
  const chooseModel = button("Choose checkpoint…", () => { void state.run(async () => {
    try {
      const choice = await bridge.chooseLocalModel();
      if (choice) {
        model = choice;
        modelPreview.replaceChildren(h("div", { class: "inline-group" }, tag(choice.model.architecture.toUpperCase()), h("strong", {}, bytesLabel(choice.model.bytes) + " · " + choice.model.files.length + " files")),
          h("p", {}, choice.model.format), details("Checkpoint location", h("p", { class: "path-note" }, displayPath(choice.model.source))));
      }
    } catch (error) { model = null; modelPreview.textContent = "No valid checkpoint selected."; throw error; }
    finally { update(); }
  }, "Inspecting checkpoint files… Large models can take a moment."); }, "secondary", "models");
  chooseModel.id = "choose-local-model";
  const chooseParent = button("Choose location…", () => { void state.run(async () => {
    const choice = await bridge.chooseProjectParent(); if (choice) parent = choice; update();
  }, "Choosing a project location…"); }, "secondary", "project");
  chooseParent.id = "choose-project-parent";
  state.fields.append(nameField,
    h("section", { class: "onboarding-section" }, h("h3", {}, "Starting model"), h("p", {}, "Choose a local BERT-family encoder folder with safetensors weights. The original stays untouched."), chooseModel, modelPreview),
    h("section", { class: "onboarding-section" }, h("h3", {}, "Save project"), chooseParent, folderField, h("span", { class: "field-caption" }, "New project folder"), destination),
    details("Add a task description", taskField));
  state.footer.append(state.progress, state.error, hint, h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "secondary"), confirm));
  const form = h("form", { class: "onboarding-form", onSubmit: (event: Event) => {
    event.preventDefault(); if (!model || !parent) return;
    const request = { name: name.value, folderName: folderName.value, modelToken: model.token, parentToken: parent.token, task: task.value };
    void state.run(async () => { const collection = await bridge.createManagedProject(request); dialog.close(); await created(collection); }, "Copying and verifying " + bytesLabel(model.model.bytes) + ". Keep the app open…");
  } }, h("header", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "New encoder project"), h("p", {}, "A separate workspace for your baseline, datasets, and experiments.")), state.fields, state.footer);
  update(); dialog.replaceChildren(form); dialog.showModal(); name.focus();
}

export function importDatasetDialog(dialog: HTMLDialogElement, bridge: EncoderGymBridge, projectId: string, imported: (project: OpenedProject) => Promise<void>): void {
  const state = dialogState(dialog);
  let choice: DatasetChoice | null = null;
  const [nameField, name] = field("Dataset name", "import-dataset-name", "e.g. Training examples");
  const preview = h("div", { id: "dataset-preview", class: "import-preview" }, "No file selected. JSONL contains one JSON record per line.");
  const hint = h("p", { id: "confirm-dataset-hint", class: "confirmation-hint" }, "Choose a file to review before importing.");
  const confirm = button("Import dataset", () => {}, "primary"); confirm.id = "confirm-dataset-import"; confirm.type = "submit"; confirm.disabled = true;
  confirm.setAttribute("aria-describedby", hint.id);
  const purposeHelp = h("p", { id: "purpose-help", class: "field-help" });
  const purposes: [DatasetPurpose, string, string][] = [
    ["unassigned", "Decide later", "Keep a source copy without assigning it to training or evaluation."],
    ["training", "Training", "For model training. Files declaring held-out rows are rejected."],
    ["development", "Development / validation", "For measuring progress while developing the model."],
    ["sealed", "Sealed holdout", "Reserved for final acceptance, not development feedback."],
  ];
  const purpose = h("select", { id: "dataset-purpose", class: "text-input", value: "unassigned", "aria-describedby": "purpose-help", onChange: () => {
    choice = null; confirm.disabled = true; state.error.replaceChildren();
    preview.textContent = "Purpose changed. Select the file again to check it for this purpose.";
    hint.textContent = "Choose a file for the selected purpose."; explainPurpose();
  } }, ...purposes.map(([value, label]) => h("option", { value }, label))) as HTMLSelectElement;
  const explainPurpose = () => { purposeHelp.textContent = purposes.find(([value]) => value === purpose.value)![2]; };
  explainPurpose();
  name.addEventListener("input", () => { confirm.disabled = !choice || !name.value.trim(); });
  const choose = button("Choose JSONL file…", () => { void state.run(async () => {
    try {
      const result = await bridge.chooseDataset(projectId, purpose.value as DatasetPurpose);
      if (!result) return;
      choice = result; if (!name.value) name.value = result.source.split(/[\\/]/).at(-1) ?? "Imported dataset";
      preview.replaceChildren(h("div", { class: "preview-summary" }, h("strong", {}, result.rows.toLocaleString() + " records"), h("span", {}, bytesLabel(result.artifact.bytes) + " to copy")),
        facts([["Declared partitions", Object.entries(result.partitions).map(([key, count]) => key + ": " + count.toLocaleString()).join(" · ") || "Not declared"]]),
        details("Source file", h("p", { class: "path-note" }, displayPath(result.source))));
      confirm.disabled = !name.value.trim();
      hint.textContent = "Import an unchanged copy. Training preparation remains separate.";
    } catch (error) {
      choice = null; confirm.disabled = true; preview.textContent = "This file could not be selected.";
      hint.textContent = "Choose a valid file to continue."; throw error;
    }
  }, "Checking records and declared partitions… No files are changed."); }, "secondary", "project");
  choose.id = "choose-dataset-file";
  state.fields.append(h("label", { class: "form-field", for: "dataset-purpose" }, "How will you use this data?", purpose), purposeHelp,
    choose, preview, nameField, h("p", { class: "field-help" }, "Gym records the source and checksum. Importing does not approve training membership or create evaluation results."));
  state.footer.append(state.progress, state.error, hint, h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "secondary"), confirm));
  dialog.replaceChildren(h("form", { class: "onboarding-form", onSubmit: (event: Event) => {
    event.preventDefault(); if (!choice) return; const token = choice.token;
    void state.run(async () => { const result = await bridge.importDataset(projectId, token, name.value); dialog.close(); await imported(result); }, "Copying and verifying the dataset. Keep the app open…");
  } }, h("header", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "Import dataset"), h("p", {}, "Add a local source file to this project. Its original stays untouched.")), state.fields, state.footer));
  dialog.showModal(); purpose.focus();
}
