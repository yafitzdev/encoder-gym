import type { ManagedProviderStatus, ProviderInput, ProviderRole, ProviderSettingsRequest } from "../managed-control.js";
import { button, failureNotice } from "./components.js";
import { h } from "./dom.js";

export interface ProviderDialogSubmission { settings: ProviderSettingsRequest; credentials: Partial<Record<ProviderRole, string>> }

const presets = [
  { label: "DeepSeek", endpoint: "https://api.deepseek.com", generation: "deepseek-v4-flash", advisor: "deepseek-v4-pro" },
  { label: "Yan", endpoint: "https://yan.tail85512d.ts.net", generation: "deepseek-v4-flash", advisor: "deepseek-v4-pro" },
] as const;
const defaults = {
  generation: { requests: 100, input: 1_000_000, output: 200_000, cost: 5 },
  advisor: { requests: 20, input: 200_000, output: 50_000, cost: 2 },
};
const byId = <T extends HTMLElement>(id: string): T => {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing #${id}`);
  return value as T;
};
const normalizedEndpoint = (value: string): string => value.trim().replace(/\/+$/, "").toLowerCase();
const matchingPreset = (endpoint: string) => presets.find(preset => normalizedEndpoint(preset.endpoint) === normalizedEndpoint(endpoint));
function providerFields(role: "generation" | "advisor", status?: ManagedProviderStatus): HTMLElement {
  const existing = status?.catalog?.providers.find(provider => provider.role === role);
  const availability = status?.credentialAvailability.find(item => item.role === role)?.availability;
  const endpoint = existing?.endpoint ?? presets[0].endpoint;
  const url = h("input", { id: `${role}-endpoint`, class: "text-input", type: "url", value: endpoint, required: true, autocomplete: "url" }) as HTMLInputElement;
  const key = h("input", {
    id: `${role}-credential`, class: "text-input", type: "password", required: availability !== "available",
    autocomplete: "new-password", placeholder: availability === "available" ? "Saved" : "",
  }) as HTMLInputElement;
  const presetButtons = presets.map(preset => {
    const control = button(preset.label, () => {
      url.value = preset.endpoint;
      url.dispatchEvent(new Event("input", { bubbles: true }));
    }, "ghost small");
    control.dataset.endpoint = preset.endpoint;
    return control;
  });
  const updatePreset = () => {
    for (const control of presetButtons) control.classList.toggle("selected", normalizedEndpoint(control.dataset.endpoint ?? "") === normalizedEndpoint(url.value));
    const reusesSavedKey = availability === "available" && existing?.endpoint && normalizedEndpoint(existing.endpoint) === normalizedEndpoint(url.value);
    key.required = !reusesSavedKey;
    key.placeholder = reusesSavedKey ? "Saved" : "";
  };
  url.addEventListener("input", updatePreset);
  updatePreset();
  return h("section", { class: "provider-form-section" }, h("h3", {}, role === "generation" ? "Data generation" : "Agentic work"),
    h("div", { class: "provider-presets", "aria-label": `${role} provider presets` }, ...presetButtons),
    h("div", { class: "provider-form-grid" },
      h("label", { class: "form-field", for: url.id }, "URL", url),
      h("label", { class: "form-field", for: key.id }, "API key", key)),
  );
}
function readProvider(role: "generation" | "advisor", status?: ManagedProviderStatus): ProviderInput {
  const endpoint = byId<HTMLInputElement>(`${role}-endpoint`).value.trim();
  if (!endpoint) throw new Error(`Enter the ${role} URL.`);
  const existing = status?.catalog?.providers.find(provider => provider.role === role);
  const sameEndpoint = existing?.endpoint && normalizedEndpoint(existing.endpoint) === normalizedEndpoint(endpoint);
  const preset = matchingPreset(endpoint), fallback = defaults[role];
  const model = preset?.[role] ?? (sameEndpoint ? existing.model : "default");
  const limits = sameEndpoint ? existing.limits : {
    maximumRequests: fallback.requests,
    maximumInputTokens: fallback.input,
    maximumOutputTokens: fallback.output,
    maximumCostMicrousd: fallback.cost * 1_000_000,
  };
  return {
    kind: "openai-compatible", endpoint, model, authentication: "bearer",
    environmentFallback: role === "generation" ? "SYNTH_OPENAI_API_KEY" : "SYNTH_ADVISOR_API_KEY",
    limits,
  };
}

export function providerDialog(dialog: HTMLDialogElement, status: ManagedProviderStatus | undefined, submit: (value: ProviderDialogSubmission) => Promise<void>): void {
  const error = h("div", { class: "form-error", role: "alert" }), progress = h("div", { class: "operation-status", role: "status" });
  let saving = false;
  const cancel = button("Cancel", () => dialog.close(), "secondary"), save = button("Save provider setup", () => {}, "primary"); save.type = "submit";
  const form = h("form", { class: "onboarding-form provider-form", onSubmit: async (event: Event) => {
    event.preventDefault(); if (saving) return;
    error.replaceChildren();
    try {
      const generation = readProvider("generation", status), advisor = readProvider("advisor", status);
      const credentials: Partial<Record<ProviderRole, string>> = {};
      for (const role of ["generation", "advisor"] as const) { const value = byId<HTMLInputElement>(`${role}-credential`).value; if (value) credentials[role] = value; }
      saving = true; dialog.setAttribute("aria-busy", "true"); save.disabled = true; cancel.disabled = true; progress.textContent = "Saving…";
      await submit({ settings: { version: 1, generation, advisor, actor: "local-operator", reason: "Configure separate desktop provider authorities" }, credentials });
      dialog.close();
    } catch (failure) { error.replaceChildren(failureNotice(failure)); }
    finally { saving = false; dialog.removeAttribute("aria-busy"); save.disabled = false; cancel.disabled = false; progress.textContent = ""; }
  } },
    h("div", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "Providers")),
    h("div", { class: "onboarding-fields" }, providerFields("generation", status), providerFields("advisor", status)),
    h("footer", { class: "onboarding-footer" }, progress, error, h("div", { class: "dialog-actions" }, cancel, save)));
  dialog.replaceChildren(form); dialog.showModal(); byId<HTMLInputElement>("generation-endpoint").focus();
}
