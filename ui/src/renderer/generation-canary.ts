import type { GenerationCanary } from "../optimization-repair.js";
import { h } from "./dom.js";

export function generationCanaryPanel(canary: GenerationCanary | undefined): HTMLElement {
  const states = {
    disabled: "No canary was authorized for this historical run.",
    not_required: "No generation targets; no canary needed.",
    pending: "Waiting for a saved canary result. This does not imply a live worker.",
    interrupted: "The sample attempt was interrupted or its outcome is unknown. Its reservation remains recorded.",
    passed: "Native admission passed. Remaining batches may proceed within the saved limits.",
    rejected: "Sample rejected. Remaining batches were not dispatched. Resume reuses this result; it does not retry rejected rows.",
  };
  return h("section", { class: "generation-canary", "aria-label": "Generation canary" }, h("h4", {}, "Generation canary"),
    h("p", {}, canary ? states[canary.status] : "Canary details are unavailable for this history."),
    canary?.policy && canary.requested ? h("p", {}, `${canary.requested} sample rows requested · ${canary.admitted} admitted · ${canary.rejected.length} rejected`) : null,
    canary?.policy ? h("p", { class: "muted" }, "The first batch only—not every target. This checks native format and local duplicate admission, not semantic correctness or complete benchmark isolation. The full dataset still passes the training firewall.") : null,
    ...canary?.rows.map(row => h("article", { class: "repair-target", "data-key": row.fingerprint }, h("h5", {}, `Sample row ${row.index + 1} · ${row.taskKind ?? "task kind not recorded"}`), h("p", {}, row.question))) ?? [],
    ...canary?.rejected.map(row => h("p", { class: "repair-rejection" }, `Sample row ${row.index + 1}: ${row.reason}`)) ?? [],
    canary?.callId ? h("p", { class: "muted repair-source" }, "Saved generator call ", h("code", {}, canary.callId)) : null);
}
