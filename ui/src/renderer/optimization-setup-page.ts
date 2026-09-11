import type { Actions } from "./actions.js";
import type { OptimizationSetupController } from "./optimization-setup-controller.js";
import { button, failureNotice, selectControl, workspacePage } from "./components.js";
import { h } from "./dom.js";
import { inputRunProgress, pendingInputRunProgress } from "./input-run-progress.js";

export function renderOptimizationSetup(controller: OptimizationSetupController, actions: Actions): HTMLElement {
  const model = controller.model, selectedDataset = controller.dataset, busy = controller.loading || controller.saving || controller.running || controller.cancelling || controller.initializingEvaluation;
  const datasets: [string, string][] = controller.data?.datasets.flatMap(entry => entry.versions.map(item => [item.version.id, `${entry.dataset.name} · v${item.version.number} · ${item.rows.toLocaleString()} ${item.rows === 1 ? "row" : "rows"}`] as [string, string])) ?? [];
  const optimize = button(controller.error && controller.run ? "Retry" : "Optimize", () => { void controller.optimize(); }, "primary");
  optimize.id = "optimization-start"; optimize.disabled = !controller.canOptimize;
  const datasetSelect = selectControl("optimization-dataset", "Dataset version", [["", "Choose dataset"], ...datasets], controller.datasetId, value => controller.select("dataset", value));
  datasetSelect.querySelector("select")!.disabled = busy;
  return workspacePage("Optimize", null,
    controller.error ? h("div", { role: "alert", class: "operation-failure" }, failureNotice(controller.error)) : null,
    controller.loading && !controller.saving && !controller.initializingEvaluation ? h("div", { role: "status", class: "workspace-progress" }, "Loading inputs…") : null,
    h("div", { class: "optimization-inputs", "aria-label": "Optimization inputs" },
      h("section", { class: "optimization-input" }, h("h2", {}, "Baseline"),
        model ? h("div", { class: "optimization-input-selection" }, h("strong", {}, model.name), button("Inspect model", () => actions.navigate({ page: "model", id: model.id }), "ghost", "arrow"))
          : button("Choose baseline", () => actions.navigate({ page: "models" }), "secondary")),
      h("section", { class: "optimization-input" }, h("h2", {}, "Starting dataset"),
        datasets.length ? datasetSelect : button("Set up dataset", () => actions.navigate({ page: "datasets" }), "secondary"),
        selectedDataset ? button("Inspect dataset", () => actions.navigate({ page: "dataset", id: selectedDataset.version.version.id, tab: "rows" }), "ghost", "arrow") : null),
      h("section", { class: "optimization-input" }, h("h2", {}, "Evaluation"),
        controller.benchmark || controller.workspace.scientificBinding
          ? h("div", { class: "optimization-input-selection" }, h("strong", {}, controller.benchmark ? `Version ${controller.benchmark.number}` : "Project evaluation"),
            controller.benchmark ? button("Inspect evaluation", () => actions.navigate({ page: "benchmarks", id: controller.benchmark!.id, tab: "protocol" }), "ghost", "arrow") : null)
          : button("Connect evaluation", () => actions.navigate({ page: "project" }), "secondary"))),
    h("div", { class: "optimization-input-actions" }, optimize,
      controller.canCancel || controller.cancelling ? button(controller.cancelling ? "Stopping…" : "Stop", () => { void controller.cancel(); }, "secondary danger") : null),
    controller.saving || controller.initializingEvaluation ? h("div", { role: "status", class: "workspace-progress" }, controller.phase ?? "Saving…", " ", h("span", { "data-elapsed-start": String(controller.startedAt) })) : null,
    controller.run ? runState(controller, actions) : controller.running ? pendingInputRunProgress() : null);
}

function runState(controller: OptimizationSetupController, actions: Actions): HTMLElement {
  const selectedDataset = controller.dataset;
  return h("div", {}, inputRunProgress({
    run: controller.run!, running: controller.running, startedAt: controller.startedAt, activity: controller.activity,
    context: {
      model: controller.model?.name ?? "Baseline model",
      dataset: selectedDataset ? `${selectedDataset.entry.dataset.name} · v${selectedDataset.version.version.number}` : "Starting dataset",
      datasetRows: selectedDataset?.version.rows ?? 0,
      evaluation: controller.benchmark ? `Evaluation · Version ${controller.benchmark.number}` : "Evaluation",
      developmentSuites: controller.benchmark?.definition.suites.filter(suite => suite.role === "development").map(suite => suite.key.replaceAll("_", " ")) ?? [],
      finalSuite: controller.benchmark?.definition.suites.find(suite => suite.role === "sealed_acceptance")?.key.replaceAll("_", " "),
      finalEvaluation: controller.run!.state === "evaluating_final" || controller.run!.state === "final_evaluation_failed",
    },
  }), !controller.running ? button("Runs", () => actions.navigate({ page: "runs" }), "ghost", "arrow") : null);
}
