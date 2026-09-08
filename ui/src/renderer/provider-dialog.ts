import type { ManagedProviderStatus, ProviderInput, ProviderRole, ProviderSettingsRequest } from "../managed-control.js";
import { button, failureNotice } from "./components.js";
import { h } from "./dom.js";

export interface ProviderDialogSubmission { settings: ProviderSettingsRequest; credentials: Partial<Record<ProviderRole, string>> }

const defaults = {
  generation: { requests: 100, input: 1_000_000, output: 200_000, cost: 5 },
  advisor: { requests: 20, input: 200_000, output: 50_000, cost: 2 },
};
const byId = <T extends HTMLElement>(id: string): T => {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing #${id}`);
  return value as T;
};
function number(id: string, label: string, minimum: number): number {
  const value = Number(byId<HTMLInputElement>(id).value);
  if (!Number.isSafeInteger(value) || value < minimum) throw new Error(`${label} must be ${minimum ? "a positive" : "a non-negative"} whole number.`);
  return value;
}
function providerFields(role: "generation" | "advisor", status?: ManagedProviderStatus): HTMLElement {
  const existing = status?.catalog?.providers.find(provider => provider.role === role);
  const limit = existing?.limits, fallback = defaults[role];
  const field = (id: string, label: string, value: string | number, type = "text", help?: string) => h("label", { class: "form-field", for: id }, label,
    h("input", { id, class: "text-input", type, value: String(value), required: type !== "password", autocomplete: type === "password" ? "new-password" : "off", ...(type === "number" ? { min: "0", step: "any" } : {}) }), help ? h("span", { class: "field-help" }, help) : null);
  return h("section", { class: "provider-form-section" }, h("h3", {}, role === "generation" ? "Data generation" : "Advisor / agentic work"),
    h("p", {}, role === "generation" ? "Creates new source evidence within an authorized generation budget." : "Proposes bounded research or repair work; it is a separate authority."),
    h("div", { class: "provider-form-grid" },
      field(`${role}-endpoint`, "OpenAI-compatible endpoint", existing?.endpoint ?? "https://api.openai.com/v1", "url"),
      field(`${role}-model`, "Model", existing?.model ?? ""),
      field(`${role}-requests`, "Maximum requests", limit?.maximumRequests ?? fallback.requests, "number"),
      field(`${role}-input`, "Maximum input tokens", limit?.maximumInputTokens ?? fallback.input, "number"),
      field(`${role}-output`, "Maximum output tokens", limit?.maximumOutputTokens ?? fallback.output, "number"),
      field(`${role}-cost`, "Maximum spend (USD)", (limit?.maximumCostMicrousd ?? fallback.cost * 1_000_000) / 1_000_000, "number"),
      field(`${role}-credential`, existing ? "Replace saved credential (optional)" : "Credential (optional)", "", "password", existing ? "Leave blank to keep the current credential or environment fallback." : "Leave blank to use the environment fallback.")),
  );
}
function readProvider(role: "generation" | "advisor"): ProviderInput {
  const endpoint = byId<HTMLInputElement>(`${role}-endpoint`).value.trim(), model = byId<HTMLInputElement>(`${role}-model`).value.trim();
  const cost = Number(byId<HTMLInputElement>(`${role}-cost`).value);
  if (!endpoint || !model) throw new Error(`Complete the ${role} endpoint and model.`);
  if (!Number.isFinite(cost) || cost < 0 || cost > Number.MAX_SAFE_INTEGER / 1_000_000) throw new Error(`Enter a valid finite ${role} spend ceiling.`);
  return {
    kind: "openai-compatible", endpoint, model, authentication: "bearer",
    environmentFallback: role === "generation" ? "SYNTH_OPENAI_API_KEY" : "SYNTH_ADVISOR_API_KEY",
    limits: {
      maximumRequests: number(`${role}-requests`, `${role} request limit`, 1),
      maximumInputTokens: number(`${role}-input`, `${role} input-token limit`, 1),
      maximumOutputTokens: number(`${role}-output`, `${role} output-token limit`, 1),
      maximumCostMicrousd: Math.round(cost * 1_000_000),
    },
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
      const generation = readProvider("generation"), advisor = readProvider("advisor");
      const credentials: Partial<Record<ProviderRole, string>> = {};
      for (const role of ["generation", "advisor"] as const) { const value = byId<HTMLInputElement>(`${role}-credential`).value; if (value) credentials[role] = value; }
      saving = true; dialog.setAttribute("aria-busy", "true"); save.disabled = true; cancel.disabled = true; progress.textContent = "Saving non-secret settings, then storing credentials with operating-system encryption…";
      await submit({ settings: { version: 1, generation, advisor, actor: "local-operator", reason: "Configure separate desktop provider authorities" }, credentials });
      dialog.close();
    } catch (failure) { error.replaceChildren(failureNotice(failure)); }
    finally { saving = false; dialog.removeAttribute("aria-busy"); save.disabled = false; cancel.disabled = false; progress.textContent = ""; }
  } },
    h("div", { class: "onboarding-heading" }, h("h2", { id: "project-dialog-title" }, "Provider authorities"), h("p", {}, "Settings are project records. Credentials are encrypted separately and can never be read back by this screen.")),
    h("div", { class: "onboarding-fields" }, providerFields("generation", status), providerFields("advisor", status)),
    h("footer", { class: "onboarding-footer" }, progress, error, h("div", { class: "dialog-actions" }, cancel, save)));
  dialog.replaceChildren(form); dialog.showModal(); byId<HTMLInputElement>("generation-model").focus();
}
