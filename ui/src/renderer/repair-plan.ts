import type { DatasetRepairPlan, RepairEvidence } from "../optimization-repair.js";
import { button } from "./components.js";
import { h } from "./dom.js";
import { generationCanaryPanel } from "./generation-canary.js";

// Presentation-only pagination, keyed by immutable decision identity.
const pages = new Map<string, number>();
const pageSize = 20;

export function repairPlanPanel(plan: DatasetRepairPlan | null | undefined, render: () => void): HTMLElement {
  if (!plan) return h("section", { class: "repair-plan", "aria-label": "Dataset repair plan" }, h("h3", {}, "Dataset repair plan"),
    h("p", { class: "muted" }, plan === null ? "No accepted dataset repair plan is recorded yet." : "Saved repair-plan details are unavailable for this history."));
  const requested = plan.proposal.additions.reduce((sum, target) => sum + target.count, 0);
  const published = plan.publication;
  const count = (value: number) => value.toLocaleString();
  const referenceLabels = (ids: string[]) => ids.map(id => plan.evidence.find(item => item.id === id)?.label ?? id).join("; ");
  const metric = (label: string, value: string) => h("div", {}, h("dt", {}, label), h("dd", {}, value));
  const paged = <T,>(name: string, values: T[], row: (item: T, index: number) => HTMLElement) => {
    const key = `${plan.decisionCallId}:${name}`;
    const page = Math.min(pages.get(key) ?? 0, Math.max(0, Math.ceil(values.length / pageSize) - 1));
    const change = (next: number) => { if (pages.size >= 100 && !pages.has(key)) pages.delete(pages.keys().next().value!); pages.set(key, next); render(); };
    const previous = button("Previous", () => change(page - 1), "ghost small"); previous.disabled = page === 0;
    const next = button("Next", () => change(page + 1), "ghost small"); next.disabled = (page + 1) * pageSize >= values.length;
    return h("section", { class: "repair-plan-group", "aria-label": name }, h("h4", {}, `${name} · ${count(values.length)}`),
      ...values.slice(page * pageSize, (page + 1) * pageSize).map((item, index) => row(item, page * pageSize + index)),
      values.length > pageSize ? h("div", { class: "focus-controls" }, previous, h("span", { class: "muted" }, `${page * pageSize + 1}–${Math.min((page + 1) * pageSize, values.length)} of ${count(values.length)}`), next) : null);
  };
  return h("section", { class: "repair-plan", "aria-label": "Dataset repair plan", "data-repair-decision": plan.decisionCallId },
    h("h3", {}, "Dataset repair plan"),
    h("p", { class: "repair-plan-state" }, plan.proposal.stop ? "Decision: no dataset change" : published ? "Dataset changes published" : "Plan recorded · dataset not yet published"),
    h("p", {}, plan.proposal.summary),
    h("dl", { class: "repair-plan-totals" }, metric("Input rows", count(plan.inputRows)), metric("Requested additions", count(requested)),
      metric("Requested removals", count(plan.proposal.removals.length)), metric("Remaining edit ceiling at decision", count(plan.maximumRowChanges))),
    published ? h("p", { class: "repair-plan-outcome" }, `Published +${count(published.added)} / −${count(published.removed)} rows · ${count(published.rows)} rows total. ${count(published.crossBatchDuplicates)} cross-batch duplicates excluded.`) : null,
    !plan.proposal.stop ? h("p", { class: "muted" }, "Generation changes questions only; template context, registry and labels are retained. Admission is not full-dataset qualification or permission to train.") : null,
    generationCanaryPanel(plan.canary),
    plan.proposal.additions.length ? paged("Addition targets", plan.proposal.additions, (target, index) => {
      const observed = plan.generation[index]!;
      return h("article", { class: "repair-target", "data-key": `addition-${index}` }, h("h5", {}, `Target ${index + 1} · ${count(target.count)} requested`),
        h("p", {}, target.instruction), h("p", { class: "muted" }, "Template ", h("code", {}, target.templateRowId)),
        h("p", { class: "repair-evidence-reference" }, "Evidence: ", referenceLabels(target.evidenceIds)),
        h("p", {}, `${count(observed.admitted)} admitted · ${count(observed.rejected)} rejected · ${count(observed.unresolved)} unresolved · ${count(observed.attempts)} call attempts`),
        observed.unresolved ? h("p", { class: "muted" }, "Unresolved includes undispatched and interrupted slots; it does not mean a worker is running.") : null,
        ...Object.entries(observed.rejectionReasons).map(([reason, rows]) => h("p", { class: "repair-rejection" }, `${count(rows)} rejected: ${reason}`)));
    }) : null,
    plan.proposal.removals.length ? paged("Removal targets", plan.proposal.removals, target => h("article", { class: "repair-target", "data-key": target.rowId },
      h("p", {}, target.reason), h("p", { class: "muted" }, "Row ", h("code", {}, target.rowId)),
      h("p", { class: "repair-evidence-reference" }, "Evidence: ", referenceLabels(target.evidenceIds)))) : null,
    plan.evidence.length ? paged("Saved evidence", plan.evidence, evidenceCard) : null,
    h("p", { class: "muted" }, "This is the agent’s recorded hypothesis, not proof of improvement. Coverage is descriptive; overlapping clusters must not be added together. Development results decide whether the candidate improved."));
}

function evidenceCard(item: RepairEvidence): HTMLElement {
  const percent = (value: number) => `${(value * 100).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`;
  return h("article", { class: "repair-evidence", "data-key": item.id }, h("h5", {}, item.label),
    item.trainingRows !== null ? h("p", {}, `Training coverage: ${item.trainingRows.toLocaleString()} of ${item.totalTrainingRows!.toLocaleString()} rows (${percent(item.trainingRows / item.totalTrainingRows!)}).${item.overlapping ? " Overlaps other capability clusters." : ""}`) : null,
    ...item.development.map(metric => h("p", {}, `${metric.suite}: top-1 recall ${percent(metric.recallAt1)} across ${metric.support.toLocaleString()} development cases`,
      metric.originalBaselineRecallAt1 === null ? ". Original-baseline comparison not recorded." : `; original baseline ${percent(metric.originalBaselineRecallAt1)}.`)),
    h("p", { class: "muted repair-source" }, "Saved evidence ", h("code", {}, item.id)));
}
