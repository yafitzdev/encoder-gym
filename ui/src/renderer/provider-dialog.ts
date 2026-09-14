import { button, failureNotice } from "./components.js";
import { h } from "./dom.js";

export interface ProviderDialogSubmission { endpoint: string; apiKey: string }

const presets = [
  { label: "DeepSeek", endpoint: "https://api.deepseek.com" },
  { label: "Yan", endpoint: "https://yan.tail85512d.ts.net" },
] as const;

export function providerDialog(dialog: HTMLDialogElement, submit: (value: ProviderDialogSubmission) => Promise<void>): void {
  const error = h("div", { class: "form-error", role: "alert" }), progress = h("div", { class: "operation-status", role: "status" });
  const url = h("input", { id: "provider-endpoint", class: "text-input", type: "url", required: true, autocomplete: "url" }) as HTMLInputElement;
  const key = h("input", { id: "provider-api-key", class: "text-input", type: "password", required: true, autocomplete: "new-password" }) as HTMLInputElement;
  const presetButtons = presets.map(preset => { const control = button(preset.label, () => { url.value = preset.endpoint; }, "ghost small"); control.dataset.endpoint = preset.endpoint; return control; });
  let saving = false;
  const cancel = button("Cancel", () => dialog.close(), "secondary"), save = button("Add connection", () => {}, "primary"); save.type = "submit";
  const form = h("form", { class: "onboarding-form provider-form", onSubmit: async (event: Event) => {
    event.preventDefault(); if (saving) return; error.replaceChildren();
    try {
      saving = true; dialog.setAttribute("aria-busy", "true"); save.disabled = true; cancel.disabled = true; progress.textContent = "Discovering models…";
      await submit({ endpoint: url.value.trim(), apiKey: key.value });
      key.value = ""; dialog.close();
    } catch (failure) { error.replaceChildren(failureNotice(failure)); }
    finally { saving = false; dialog.removeAttribute("aria-busy"); save.disabled = false; cancel.disabled = false; progress.textContent = ""; }
  } },
    h("div", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "Add provider")),
    h("div", { class: "provider-presets", "aria-label": "Provider presets" }, ...presetButtons),
    h("div", { class: "provider-form-grid" },
      h("label", { class: "form-field", for: url.id }, "URL", url),
      h("label", { class: "form-field", for: key.id }, "API key", key)),
    h("footer", { class: "onboarding-footer" }, progress, error, h("div", { class: "dialog-actions" }, cancel, save)));
  dialog.replaceChildren(form); dialog.showModal(); url.focus();
}
