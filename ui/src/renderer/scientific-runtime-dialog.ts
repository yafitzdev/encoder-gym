import type { NomosBindingPreview, NativePathChoice } from "../managed-control.js";
import type { EncoderGymBridge } from "../preload.js";
import type { OpenedProject } from "../projects.js";
import { bytesLabel, displayPath } from "./catalog.js";
import { button, details, failureNotice, facts, status } from "./components.js";
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
      h("div", { class: "runtime-preview-heading" }, h("div", {}, h("h3", {}, value.ready ? "Runtime is compatible" : "Runtime needs attention"), h("p", {}, value.ready ? "Every required execution capability is available." : "Resolve the items below, then choose and verify the interpreter again.")), status(value.ready ? "Ready to connect" : "Blocked", value.ready ? "success" : "warning")),
      facts([["Baseline", `${value.activeModel.name} · ${bytesLabel(value.activeModel.bytes)}`], ["Adapter", `${value.adapter.key} · ${value.adapter.protocol}`], ["Runtime revision", value.sourceRevision], ["Python", `${value.python.version}${value.python.compatibleVersion ? "" : " · requires 3.11 or 3.12"}`], ["Scientific store", value.store.action === "import_verified_history" ? "Copy verified history into this project" : value.store.action === "initialize_new_store" ? "Create a new contained store" : value.store.action === "extend_existing_store_for_promoted_baseline" ? "Extend the existing history for this promoted baseline" : "Verify the existing contained store"]]),
      imported ? h("section", { class: "history-preview" }, h("strong", {}, "Existing history matches this runtime"), h("p", {}, `${imported.inventory.optimizationRuns} optimization run, ${imported.inventory.proposals} repair proposals, ${imported.inventory.approvedDeltaSelections} approved delta selections, and ${imported.inventory.trainingSnapshots} training snapshot will remain available in the contained copy.`), details("History inventory", facts([["Source", imported.sourceName], ["Projects", String(imported.inventory.projects)], ["Experiment runs", String(imported.inventory.experimentRuns)], ["Benchmark generations", String(imported.inventory.benchmarkGenerations)], ["Diagnoses", String(imported.inventory.diagnoses)], ["Protocols", String(imported.inventory.protocols)]]))) : null,
      h("div", { class: "capability-list" }, ...value.python.capabilities.map(capability => h("div", { class: "capability-row" }, status(capability.ready ? "Ready" : "Missing", capability.ready ? "success" : "warning"), h("div", {}, h("strong", {}, capability.label), capability.ready ? h("small", {}, "Available") : h("small", {}, capability.missingModules.join(", ")))))),
      !value.ready && value.python.compatibleVersion && missing.length ? h("div", { class: "runtime-repair" }, confirmInstall ? h("section", { class: "installation-confirm", role: "alert" }, h("strong", {}, "Change this Python environment?"), h("p", {}, `Encoder Gym will run pip for its fixed package mapping (${missing.join(", ")}) through the selected interpreter. Pip may download packages and dependencies from its configured index. It cannot install renderer-supplied package names.`), h("div", { class: "inline-group" }, button("Keep environment unchanged", () => { confirmInstall = false; draw(); }, "ghost small"), button("Install missing packages", () => { void install(); }, "primary"))) : button("Install missing packages…", () => { confirmInstall = true; draw(); }, "secondary")) : null,
      details("Verified identities", facts([["Runtime", displayPath(value.runtimeLocation)], ["Python executable", displayPath(value.python.executable)], ["Model fingerprint", value.activeModel.fingerprint], ["Project snapshot", value.projectSnapshot.id], ["Contained store", value.store.databasePath]])),
    );
  }
  function draw(): void {
    const cancel = button("Cancel", () => dialog.close(), "secondary"), verifyButton = button(busy ? "Verifying…" : "Verify setup", () => { void verify(); }, "secondary"), confirm = button(busy ? "Connecting…" : "Connect runtime", () => { void bind(); }, "primary");
    cancel.disabled = busy; verifyButton.disabled = busy || !runtime || !python; confirm.disabled = busy || !preview?.ready;
    dialog.oncancel = event => { if (busy) event.preventDefault(); };
    dialog.replaceChildren(h("div", { class: "onboarding-form runtime-form", "aria-busy": String(busy) },
      h("div", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "Connect scientific runtime"), h("p", {}, "Connect the task adapter that can train and evaluate this project's exact baseline. This is an execution dependency, not a dataset or model import.")),
      h("div", { class: "runtime-notice" }, h("strong", {}, "Use the isolated experiment checkout."), h("p", {}, "It must be committed, clean, have no Git remote, and contain the same baseline. The original source repository is deliberately rejected.")),
      h("div", { class: "onboarding-fields" }, selection("Isolated Nomos runtime", runtime, () => { void chooseRuntime(); }, runtime ? "Choose another folder" : "Choose folder"), selection("Python environment", python, () => { void choosePython(); }, python ? "Choose another executable" : "Choose executable"),
        h("section", { class: "runtime-selection history-selection" }, h("div", {}, h("strong", {}, "Existing Encoder Gym history"), h("span", {}, history ? displayPath(history.path) : "Optional · start with an empty scientific store"), h("small", {}, "Recommended when continuing earlier experiments. The selected database is verified and copied; it is never attached or changed in place.")), h("div", { class: "inline-group" }, history ? button("Start fresh instead", () => { history = undefined; preview = undefined; draw(); }, "ghost small") : null, button(history ? "Choose another database" : "Bring existing history", () => { void chooseHistory(); }, "secondary"))),
        busy ? h("div", { class: "operation-status", role: "status" }, activity === "install" ? "Installing the fixed missing packages into the selected Python environment…" : activity === "connect" ? "Connecting the verified runtime…" : "Checking local files and Python capabilities…") : null,
        error ? h("div", { class: "form-error", role: "alert" }, failureNotice(error)) : null,
        preview ? previewContent(preview) : h("p", { class: "section-note" }, "Verification is offline and read-only. It makes no provider call and does not create a scientific store.")),
      h("footer", { class: "onboarding-footer" }, h("p", { class: "section-note" }, history ? "Connecting creates a transactionally consistent, content-addressed copy of the selected history and records an immutable binding. The source database is unchanged; no model is trained or evaluated." : "Connecting creates or verifies a new contained scientific store and records an immutable binding. Existing experiments are not included; no model is trained or evaluated."), h("div", { class: "dialog-actions" }, cancel, verifyButton, confirm)),
    ));
  }
  draw(); dialog.showModal();
}
