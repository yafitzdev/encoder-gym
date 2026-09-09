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
  let runtime: NativePathChoice | undefined, python: NativePathChoice | undefined, preview: NomosBindingPreview | undefined;
  let busy = false, error: unknown;

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
  const verify = async () => {
    if (busy || !runtime || !python) return;
    busy = true; error = undefined; preview = undefined; draw();
    try { preview = await bridge.previewNomosBinding(projectId, runtime.token, python.token); }
    catch (failure) { error = failure; }
    finally { busy = false; draw(); }
  };
  const bind = async () => {
    if (busy || !preview?.ready) return;
    busy = true; error = undefined; draw();
    try { const result = await bridge.bindNomos(projectId, preview.token); dialog.close(); onBound(result); }
    catch (failure) { error = failure; busy = false; draw(); }
  };

  function selection(label: string, choice: NativePathChoice | undefined, action: () => void, actionLabel: string): HTMLElement {
    return h("section", { class: "runtime-selection" }, h("div", {}, h("strong", {}, label), h("span", {}, choice ? displayPath(choice.path) : "Not selected")), button(actionLabel, action, "secondary"));
  }
  function previewContent(value: NomosBindingPreview): HTMLElement {
    return h("section", { class: "runtime-preview", "data-runtime-ready": String(value.ready) },
      h("div", { class: "runtime-preview-heading" }, h("div", {}, h("h3", {}, value.ready ? "Runtime is compatible" : "Runtime needs attention"), h("p", {}, value.ready ? "Every required execution capability is available." : "Resolve the items below, then choose and verify the interpreter again.")), status(value.ready ? "Ready to connect" : "Blocked", value.ready ? "success" : "warning")),
      facts([["Baseline", `${value.activeModel.name} · ${bytesLabel(value.activeModel.bytes)}`], ["Adapter", `${value.adapter.key} · ${value.adapter.protocol}`], ["Runtime revision", value.sourceRevision], ["Python", `${value.python.version}${value.python.compatibleVersion ? "" : " · requires 3.11 or 3.12"}`], ["Scientific store", value.store.action === "initialize_new_store" ? "Create a new contained store" : "Verify the existing contained store"]]),
      h("div", { class: "capability-list" }, ...value.python.capabilities.map(capability => h("div", { class: "capability-row" }, status(capability.ready ? "Ready" : "Missing", capability.ready ? "success" : "warning"), h("div", {}, h("strong", {}, capability.label), capability.ready ? h("small", {}, "Available") : h("small", {}, capability.missingModules.join(", ")))))),
      details("Verified identities", facts([["Runtime", displayPath(value.runtimeLocation)], ["Python executable", displayPath(value.python.executable)], ["Model fingerprint", value.activeModel.fingerprint], ["Project snapshot", value.projectSnapshot.id]])),
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
        busy ? h("div", { class: "operation-status", role: "status" }, preview ? "Connecting the verified runtime…" : "Checking local files and Python capabilities…") : null,
        error ? h("div", { class: "form-error", role: "alert" }, failureNotice(error)) : null,
        preview ? previewContent(preview) : h("p", { class: "section-note" }, "Verification is offline and read-only. It makes no provider call and does not create a scientific store.")),
      h("footer", { class: "onboarding-footer" }, h("p", { class: "section-note" }, "Connecting creates or verifies runs/scientific.sqlite and records an immutable binding. It does not train or evaluate a model."), h("div", { class: "dialog-actions" }, cancel, verifyButton, confirm)),
    ));
  }
  draw(); dialog.showModal();
}
