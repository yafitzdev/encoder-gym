import type { NomosBindingPreview, NativePathChoice } from "../managed-control.js";
import type { EncoderGymBridge } from "../preload.js";
import type { OpenedProject } from "../projects.js";
import { bytesLabel, displayPath } from "./catalog.js";
import { button, details, failureNotice, facts, status, tag } from "./components.js";
import { h } from "./dom.js";

export function scientificRuntimeDialog(
  dialog: HTMLDialogElement,
  bridge: EncoderGymBridge,
  projectId: string,
  onBound: (result: OpenedProject) => void,
): void {
  let runtime: NativePathChoice | undefined, python: NativePathChoice | undefined, history: NativePathChoice | undefined, preview: NomosBindingPreview | undefined;
  let busy = false, confirmInstall = false, activity: "verify" | "install" | "connect" = "verify", error: unknown;

  const chooseRuntime = async () => {
    if (busy) return;
    busy = true; error = undefined; draw();
    try { const selected = await bridge.chooseNomosRuntime(projectId); if (selected) { runtime = selected; preview = undefined; } }
    catch (failure) { error = failure; }
    finally { busy = false; draw(); }
  };
  const choosePython = async () => {
    if (busy) return;
    busy = true; error = undefined; draw();
    try { const selected = await bridge.chooseNomosPython(projectId); if (selected) { python = selected; preview = undefined; } }
    catch (failure) { error = failure; }
    finally { busy = false; draw(); }
  };
  const chooseHistory = async () => {
    if (busy) return;
    busy = true; error = undefined; draw();
    try { const selected = await bridge.chooseNomosHistory(projectId); if (selected) { history = selected; preview = undefined; } }
    catch (failure) { error = failure; }
    finally { busy = false; draw(); }
  };
  const verify = async () => {
    if (busy || !runtime || !python) return;
    busy = true; activity = "verify"; error = undefined; preview = undefined; confirmInstall = false; draw();
    try { preview = await bridge.previewNomosBinding(projectId, runtime.token, python.token, history?.token); }
    catch (failure) { error = failure; }
    finally { busy = false; draw(); }
  };
  const bind = async () => {
    if (busy || !preview?.ready) return;
    busy = true; activity = "connect"; error = undefined; draw();
    try { const result = await bridge.bindNomos(projectId, preview.token); dialog.close(); onBound(result); }
    catch (failure) { error = failure; busy = false; draw(); }
  };
  const install = async () => {
    if (busy || !preview || preview.ready || !runtime || !python) return;
    busy = true; activity = "install"; error = undefined; draw();
    try {
      await bridge.prepareNomosPython(projectId, preview.token);
      preview = await bridge.previewNomosBinding(projectId, runtime.token, python.token, history?.token);
      confirmInstall = false;
    } catch (failure) { error = failure; }
    finally { busy = false; draw(); }
  };

  function selection(label: string, choice: NativePathChoice | undefined, action: () => void, actionLabel: string): HTMLElement {
    return h("section", { class: "runtime-selection" }, h("div", {}, h("strong", {}, label), h("span", {}, choice ? displayPath(choice.path) : "Not selected")), button(actionLabel, action, "secondary"));
  }
  function previewContent(value: NomosBindingPreview): HTMLElement {
    const imported = value.store.importedHistory;
    const missing = [...new Set(value.python.capabilities.flatMap(capability => capability.missingModules))];
    return h("section", { class: "runtime-preview", "data-runtime-ready": String(value.ready) },
      h("div", { class: "runtime-preview-heading" }, h("h3", {}, value.ready ? "Compatible runtime" : "Runtime blocked"), status(value.ready ? "Ready" : "Blocked", value.ready ? "success" : "warning")),
      facts([["Baseline", `${value.activeModel.name} · ${bytesLabel(value.activeModel.bytes)}`], ["Adapter", `${value.adapter.key} · ${value.adapter.protocol}`], ["Runtime revision", value.sourceRevision], ["Python", `${value.python.version}${value.python.compatibleVersion ? "" : " · requires 3.11 or 3.12"}`], ["Managed package", `${bytesLabel(value.package.totalBytes)} · ${value.package.runtimeFiles + value.package.pythonFiles} files copied into this project`], ["Scientific store", value.store.action === "import_verified_history" ? "Copy verified history into this project" : value.store.action === "initialize_new_store" ? "Create a new contained store" : value.store.action === "extend_existing_store_for_promoted_baseline" ? "Extend the existing history for this promoted baseline" : "Verify the existing contained store"]]),
      imported ? h("section", { class: "history-preview" }, h("strong", {}, "Verified history"), h("div", { class: "inline-group" }, status(`${imported.inventory.optimizationRuns} optimization runs`, "success"), tag(`${imported.inventory.proposals} proposals`), tag(`${imported.inventory.trainingSnapshots} snapshots`)), details("Inventory", facts([["Source", imported.sourceName], ["Projects", String(imported.inventory.projects)], ["Experiment runs", String(imported.inventory.experimentRuns)], ["Benchmark generations", String(imported.inventory.benchmarkGenerations)], ["Diagnoses", String(imported.inventory.diagnoses)], ["Protocols", String(imported.inventory.protocols)]]))) : null,
      h("div", { class: "capability-list" }, ...value.python.capabilities.map(capability => h("div", { class: "capability-row" }, status(capability.ready ? "Ready" : "Missing", capability.ready ? "success" : "warning"), h("div", {}, h("strong", {}, capability.label), capability.ready ? h("small", {}, "Available") : h("small", {}, capability.missingModules.join(", ")))))),
      !value.ready && value.python.compatibleVersion && missing.length ? h("div", { class: "runtime-repair" }, confirmInstall ? h("section", { class: "installation-confirm", role: "alert" }, h("strong", {}, "Change this Python environment?"), h("p", {}, `Encoder Gym will run pip for its fixed package mapping (${missing.join(", ")}) through the selected interpreter. Pip may download packages and dependencies from its configured index. It cannot install renderer-supplied package names.`), h("div", { class: "inline-group" }, button("Keep environment unchanged", () => { confirmInstall = false; draw(); }, "ghost small"), button("Install missing packages", () => { void install(); }, "primary"))) : button("Install missing packages…", () => { confirmInstall = true; draw(); }, "secondary")) : null,
      details("Verified identities", facts([["Source runtime", displayPath(value.runtimeLocation)], ["Source Python", displayPath(value.python.executable)], ["Managed destination", value.package.destination], ["Model fingerprint", value.activeModel.fingerprint], ["Project snapshot", value.projectSnapshot.id], ["Contained store", value.store.databasePath]])),
    );
  }
  function draw(): void {
    const cancel = button("Cancel", () => dialog.close(), "secondary"), verifyButton = button(busy ? "Verifying…" : "Verify setup", () => { void verify(); }, "secondary"), confirm = button(busy ? "Copying…" : "Copy into project", () => { void bind(); }, "primary");
    cancel.disabled = busy; verifyButton.disabled = busy || !runtime || !python; confirm.disabled = busy || !preview?.ready;
    dialog.oncancel = event => { if (busy) event.preventDefault(); };
    dialog.replaceChildren(h("div", { class: "onboarding-form runtime-form", "aria-busy": String(busy) },
      h("div", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "Scientific runtime")),
      h("div", { class: "runtime-notice" }, status("Source checkout is copied; future runs use only the managed project package", "success")),
      h("div", { class: "onboarding-fields" }, selection("Isolated Nomos runtime", runtime, () => { void chooseRuntime(); }, runtime ? "Choose another folder" : "Choose folder"), selection("Python environment", python, () => { void choosePython(); }, python ? "Choose another executable" : "Choose executable"),
        h("section", { class: "runtime-selection history-selection" }, h("div", {}, h("strong", {}, "Existing history"), h("span", {}, history ? displayPath(history.path) : "None")), h("div", { class: "inline-group" }, history ? button("Remove", () => { history = undefined; preview = undefined; draw(); }, "ghost small") : null, button(history ? "Choose another database" : "Import history", () => { void chooseHistory(); }, "secondary"))),
        busy ? h("div", { class: "operation-status", role: "status" }, activity === "install" ? "Installing…" : activity === "connect" ? "Copying and verifying managed package…" : "Verifying…") : null,
        error ? h("div", { class: "form-error", role: "alert" }, failureNotice(error)) : null,
        preview ? previewContent(preview) : null),
      h("footer", { class: "onboarding-footer" }, h("div", { class: "dialog-actions" }, cancel, verifyButton, confirm)),
    ));
  }
  draw(); dialog.showModal();
}
