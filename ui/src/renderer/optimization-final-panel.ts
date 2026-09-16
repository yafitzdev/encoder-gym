import type { InputOptimizationRun } from "../input-optimization.js";
import type { ManagedWorkspace } from "../managed-workspace.js";
import type { OptimizationFinalController } from "./optimization-final-controller.js";
import type { Actions } from "./actions.js";
import { button, failureNotice, spinner } from "./components.js";
import { h } from "./dom.js";

export function optimizationFinalPanel(run: InputOptimizationRun, controller: OptimizationFinalController | undefined,
  workspace: ManagedWorkspace, actions: Actions, diagnostic: boolean, otherBusy: boolean): HTMLElement | null {
  if (!controller || !["agent_completed", "agent_budget_exhausted"].includes(run.state) || diagnostic) return null;
  const winner = run.iterations?.find(iteration => iteration.selected);
  if (!winner) return h("p", { class: "muted" }, "No development-eligible candidate. Baseline retained; final holdout was not authorized.");
  const state = controller.state(run.id), saved = state.view, receipt = saved?.execution.result;
  const busy = !!state.operation || otherBusy || !!controller.busyRun;
  const control = (id: string, label: string, action: () => void, style = "secondary") => {
    const element = button(label, action, style); element.id = `agent-final-${id}-${run.id}`; element.disabled = busy || state.loading; return element;
  };
  queueMicrotask(() => { void controller.ensure(run.id); });
  const promoted = state.promoted || workspace.modelCatalog?.baselineRevisions.some(revision => revision.change.kind === "promotion" && revision.change.decision_id === `agent-final:${run.id}`);
  const controls: HTMLElement[] = [];
  if (state.operation === "evaluate") {
    const stop = button(state.stopping ? "Stopping…" : "Stop final evaluation", () => { void controller.stop(run.id); }, "secondary");
    stop.id = `agent-final-stop-${run.id}`; stop.disabled = !!state.stopping; controls.push(stop);
  } else if (receipt) {
    if (receipt.accepted && !promoted) controls.push(control("promote", "Promote accepted candidate", () => { void controller.promote(run.id); }, "primary"));
  } else if (saved) {
    controls.push(control("recover", saved.state === "authorized" ? "Execute authorized final evaluation" : "Recover saved final result", () => { void controller.recover(run.id); }));
  } else if (state.review) {
    controls.push(control("authorize", "Authorize one final evaluation", () => { void controller.authorize(run.id); }, "primary"));
    controls.push(control("dismiss", "Not now", () => controller.dismiss(run.id), "ghost"));
  } else if (state.loaded) {
    controls.push(control("review", "Review final evaluation", () => { void controller.review(run.id); }));
  }
  controls.push(control("refresh", "Refresh final status", () => { void controller.refresh(run.id); }, "ghost"));
  return h("section", { class: "focus-report", "aria-label": "Final acceptance", "data-final-state": saved?.state ?? "not_authorized" },
    h("h3", {}, "Final acceptance"),
    h("p", {}, `Selected candidate: iteration ${winner.number}. `, winner.modelId ? button("Inspect candidate", () => actions.navigate({ page: "model", id: winner.modelId! }), "ghost small") : null),
    state.error ? h("div", { role: "alert" }, failureNotice(state.error)) : null,
    state.loading || state.operation ? h("p", { role: "status" }, spinner(`final:${run.id}`), state.operation === "evaluate" ? "Final evaluation in progress" : state.operation === "promote" ? "Updating baseline" : "Checking saved authority") : null,
    receipt ? h("p", { class: receipt.accepted ? "success" : "danger" }, promoted ? "Candidate promoted to baseline." : receipt.accepted ? "Final acceptance passed. Promotion requires your explicit action." : "Final acceptance rejected the candidate. Baseline retained.")
      : saved?.state === "outcome_unknown" ? h("p", {}, "Final allowance consumed; outcome unknown. Recovery reads saved evidence only and cannot repeat evaluation.")
      : saved ? h("p", {}, "Consent saved. The one-use evaluation has not been reserved yet.")
      : h("p", {}, "Development selection is not final acceptance. Holdout requires separate consent; promotion is never automatic."),
    state.review ? h("div", { role: "group", "aria-label": "Final evaluation consent" },
      h("p", {}, `Authorize the selected checkpoint on ${state.review.scope.finalSuite}, with a ${state.review.scope.maximumEvaluationSeconds}-second ceiling. The allowance is consumed before dispatch, even if execution is interrupted. Protected evidence cannot feed another Agent iteration.`),
      h("p", { class: "muted" }, `Checkpoint ${state.review.scope.model.id} · dataset v${state.review.scope.dataset.number}`)) : null,
    h("div", { class: "focus-controls" }, ...controls));
}
