import type { EncoderGymBridge } from "../preload.js";
import type { ProjectCollection, OpenedProject } from "../projects.js";
import type { DatasetChoice, DatasetPurpose, FolderChoice, ModelChoice } from "../managed-workspace.js";
import { bytesLabel, displayPath } from "./catalog.js";
import { button, facts, tag } from "./components.js";
import { h } from "./dom.js";

function dialogState(dialog: HTMLDialogElement) {
  let busy = false;
  const fields = h("fieldset", { class: "onboarding-fields" }) as HTMLFieldSetElement;
  const error = h("p", { class: "form-error", role: "alert" });
  const progress = h("p", { class: "operation-status", role: "status", "aria-live": "polite" });
  dialog.oncancel = event => { if (busy) event.preventDefault(); };
  return { fields, error, progress, async run(operation: () => Promise<void>, description: string) {
    if (busy) return;
    busy = true; fields.disabled = true; error.textContent = ""; progress.textContent = description;
    try { await operation(); } catch (failure) { error.textContent = failure instanceof Error ? failure.message : String(failure); }
    finally { busy = false; fields.disabled = false; progress.textContent = ""; }
  } };
}
function field(label: string, id: string, placeholder: string, required = true): [HTMLElement, HTMLInputElement] {
  const input = h("input", { class: "text-input", id, maxlength: "120", required, placeholder }) as HTMLInputElement;
  return [h("label", { class: "form-field", for: id }, label, input), input];
}

export function newProjectDialog(dialog: HTMLDialogElement, bridge: EncoderGymBridge, created: (collection: ProjectCollection) => Promise<void>): void {
  const state = dialogState(dialog);
  let model: ModelChoice | null = null, parent: FolderChoice | null = null;
  const [nameField, name] = field("Project name", "new-project-name", "e.g. Support encoder");
  const [folderField, folderName] = field("New folder name", "new-project-folder", "support-encoder");
  const [taskField, task] = field("What will this encoder do? (optional)", "new-project-task", "e.g. Rank support articles", false);
  let editedFolder = false;
  folderName.addEventListener("input", () => { editedFolder = true; });
  name.addEventListener("input", () => { if (!editedFolder) folderName.value = name.value.trim().replace(/[\\/:<>"|?*]/g, "-"); });
  const modelPreview = h("div", { id: "model-preview", class: "import-preview" }, "No checkpoint selected.");
  const destination = h("p", { id: "project-parent-path", class: "path-note" }, "Choose a parent folder. Gym will create a new folder inside it.");
  const confirm = button("Create project", () => {}, "primary"); confirm.id = "confirm-new-project"; confirm.type = "submit"; confirm.disabled = true;
  const update = () => { confirm.disabled = !model || !parent; };
  const chooseModel = button("Choose checkpoint…", () => { void state.run(async () => {
    const choice = await bridge.chooseLocalModel();
    if (choice) {
      model = choice;
      modelPreview.replaceChildren(h("div", { class: "inline-group" }, tag(choice.model.architecture.toUpperCase()), h("span", {}, `${bytesLabel(choice.model.bytes)} · ${choice.model.files.length} files · ${choice.model.format}`)), h("p", { class: "path-note" }, displayPath(choice.model.source)));
    }
    update();
  }, "Inspecting the local checkpoint and calculating its content identity…"); }, "secondary", "models");
  chooseModel.id = "choose-local-model";
  const chooseParent = button("Choose location…", () => { void state.run(async () => {
    const choice = await bridge.chooseProjectParent(); if (choice) { parent = choice; destination.textContent = choice.path; } update();
  }, "Choosing the project location…"); }, "secondary", "project");
  chooseParent.id = "choose-project-parent";
  state.fields.append(nameField,
    h("section", { class: "onboarding-section" }, h("h3", {}, "1 · Starting checkpoint"), h("p", {}, "A local safetensors encoder folder. The original stays untouched; this project gets its own copy."), chooseModel, modelPreview),
    h("section", { class: "onboarding-section" }, h("h3", {}, "2 · Project location"), chooseParent, destination, folderField), taskField,
    h("p", { class: "section-note" }, "Creates the baseline, dataset storage, and empty run folders. No training or evaluation starts. You can add datasets after creation."),
    h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "secondary"), confirm));
  const form = h("form", { onSubmit: (event: Event) => {
    event.preventDefault(); if (!model || !parent) return;
    const request = { name: name.value, folderName: folderName.value, modelToken: model.token, parentToken: parent.token, task: task.value };
    void state.run(async () => { const collection = await bridge.createManagedProject(request); dialog.close(); await created(collection); }, `Copying and verifying ${bytesLabel(model.model.bytes)} of checkpoint files. Keep the app open…`);
  } }, h("h2", { id: "project-dialog-title" }, "New encoder project"), h("p", { class: "section-note" }, "Start with a model. Gym owns everything you build from it."), state.fields, state.progress, state.error);
  dialog.replaceChildren(form); dialog.showModal(); name.focus();
}

export function importDatasetDialog(dialog: HTMLDialogElement, bridge: EncoderGymBridge, projectId: string, imported: (project: OpenedProject) => Promise<void>): void {
  const state = dialogState(dialog);
  let choice: DatasetChoice | null = null;
  const [nameField, name] = field("Dataset name", "import-dataset-name", "e.g. Training examples");
  const preview = h("div", { id: "dataset-preview", class: "import-preview" }, "Choose a JSONL file to inspect its size and record count.");
  const confirm = button("Import dataset", () => {}, "primary"); confirm.id = "confirm-dataset-import"; confirm.type = "submit"; confirm.disabled = true;
  const purpose = h("select", { id: "dataset-purpose", class: "text-input", value: "unassigned", onChange: () => { choice = null; confirm.disabled = true; preview.textContent = "Purpose changed. Choose the file again to validate it for this purpose."; } },
    ...[["unassigned", "Not assigned yet"], ["training", "Training source"], ["development", "Development / validation"], ["sealed", "Sealed holdout"]].map(([value, label]) => h("option", { value }, label))) as HTMLSelectElement;
  const choose = button("Choose JSONL file…", () => { void state.run(async () => {
    const result = await bridge.chooseDataset(projectId, purpose.value as DatasetPurpose);
    if (!result) return;
    choice = result; if (!name.value) name.value = result.source.split(/[\\/]/).at(-1) ?? "Imported dataset";
    preview.replaceChildren(facts([["Records", result.rows.toLocaleString()], ["Copy size", bytesLabel(result.artifact.bytes)], ["Declared partitions", Object.entries(result.partitions).map(([key, count]) => `${key}: ${count.toLocaleString()}`).join(" · ")]]), h("p", { class: "path-note" }, displayPath(result.source)));
    confirm.disabled = false;
  }, "Reading JSONL records and checking their declared partition. No data is changed…"); }, "secondary", "project");
  choose.id = "choose-dataset-file";
  state.fields.append(h("label", { class: "form-field", for: "dataset-purpose" }, "Intended purpose", purpose), choose, preview, nameField,
    h("p", { class: "section-note" }, "The file is copied unchanged, with its source and checksum recorded. Importing does not approve training membership or create evaluation results."),
    h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "secondary"), confirm));
  dialog.replaceChildren(h("form", { onSubmit: (event: Event) => {
    event.preventDefault(); if (!choice) return; const token = choice.token;
    void state.run(async () => { const result = await bridge.importDataset(projectId, token, name.value); dialog.close(); await imported(result); }, "Copying and verifying the dataset. Keep the app open…");
  } }, h("h2", { id: "project-dialog-title" }, "Import dataset"), state.fields, state.progress, state.error));
  dialog.showModal(); purpose.focus();
}
