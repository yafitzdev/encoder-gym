import type { Actions } from "./actions.js";
import type { OptimizationSetupController } from "./optimization-setup-controller.js";
import { button, failureNotice, selectControl, status, workspacePage } from "./components.js";
import { h } from "./dom.js";

export function renderOptimizationSetup(controller: OptimizationSetupController, actions: Actions): HTMLElement {
  const model = controller.model, selectedDataset = controller.dataset, busy = controller.loading || controller.saving;
  const datasets: [string, string][] = controller.data?.datasets.flatMap(entry => entry.versions.map(item => [item.version.id, `${entry.dataset.name} · v${item.version.number} · ${item.rows.toLocaleString()} ${item.rows === 1 ? "row" : "rows"}`] as [string, string])) ?? [];
  const benchmarks: [string, string][] = controller.data?.benchmarks.map(version => [version.id, `Benchmark v${version.number}`]) ?? [];
  const refresh = button("Refresh", actions.refresh, "ghost", "refresh"); refresh.disabled = busy;
  const save = button(controller.saving ? "Saving…" : controller.saved ? "Inputs saved" : controller.error ? "Retry save" : "Save inputs", () => { void controller.save(); }, "primary");
  save.id = "optimization-save-inputs"; save.disabled = !controller.canSave;
  const datasetSelect = selectControl("optimization-dataset", "Dataset version", [["", "Choose dataset"], ...datasets], controller.datasetId, value => controller.select("dataset", value));
  const benchmarkSelect = selectControl("optimization-benchmark", "Benchmark version", [["", "Choose benchmark"], ...benchmarks], controller.benchmarkId, value => controller.select("benchmark", value));
  for (const control of [datasetSelect, benchmarkSelect]) control.querySelector("select")!.disabled = busy;
  return workspacePage("Optimize", refresh,
    controller.error ? h("div", { role: "alert", class: "operation-failure" }, failureNotice(controller.error), button("Refresh inputs", actions.refresh, "secondary")) : null,
    controller.loading && !controller.saving ? h("div", { role: "status", class: "workspace-progress" }, "Loading inputs…") : null,
    h("div", { class: "optimization-inputs", "aria-label": "Optimization inputs" },
      h("section", { class: "optimization-input" }, h("h2", {}, "Baseline"),
        model ? h("div", { class: "optimization-input-selection" }, h("strong", {}, model.name), button("Inspect model", () => actions.navigate({ page: "model", id: model.id }), "ghost", "arrow"))
          : button("Choose baseline", () => actions.navigate({ page: "models" }), "secondary")),
      h("section", { class: "optimization-input" }, h("h2", {}, "Starting dataset"),
        datasets.length ? datasetSelect : button("Set up dataset", () => actions.navigate({ page: "datasets" }), "secondary"),
        selectedDataset ? button("Inspect dataset", () => actions.navigate({ page: "dataset", id: selectedDataset.version.version.id, tab: "rows" }), "ghost", "arrow") : null),
      h("section", { class: "optimization-input" }, h("h2", {}, "Evaluation"),
        benchmarks.length ? benchmarkSelect : button("Set up evaluation", () => actions.navigate({ page: "benchmarks" }), "secondary"),
        controller.benchmark ? button("Inspect benchmark", () => actions.navigate({ page: "benchmarks", id: controller.benchmark!.id, tab: "protocol" }), "ghost", "arrow") : null)),
    h("div", { class: "optimization-input-actions" }, save, controller.saved ? status("No run started") : null),
    controller.saving ? h("div", { role: "status", class: "workspace-progress" }, controller.phase ?? "Saving…", " ", h("span", { "data-elapsed-start": String(controller.startedAt) })) : null,
    h("div", { class: "optimization-execution-state" }, status("Automatic execution not connected", "warning"), button("View existing runs", () => actions.navigate({ page: "runs" }), "ghost", "arrow")));
}
