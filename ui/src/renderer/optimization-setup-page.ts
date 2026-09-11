import type { Actions } from "./actions.js";
import type { OptimizationSetupController } from "./optimization-setup-controller.js";
import { button, failureNotice, selectControl, workspacePage } from "./components.js";
import { h } from "./dom.js";
import { inputRunStageLabel } from "./input-run-activity.js";

export function renderOptimizationSetup(controller: OptimizationSetupController, actions: Actions): HTMLElement {
  const model = controller.model, selectedDataset = controller.dataset, busy = controller.loading || controller.saving || controller.running;
  const datasets: [string, string][] = controller.data?.datasets.flatMap(entry => entry.versions.map(item => [item.version.id, `${entry.dataset.name} · v${item.version.number} · ${item.rows.toLocaleString()} ${item.rows === 1 ? "row" : "rows"}`] as [string, string])) ?? [];
  const benchmarks: [string, string][] = controller.data?.benchmarks.map(version => [version.id, `Benchmark v${version.number}`]) ?? [];
  const refresh = button("Refresh", actions.refresh, "ghost", "refresh"); refresh.disabled = busy;
  const optimize = button(controller.running ? "Optimizing…" : controller.error && controller.run ? "Retry" : "Optimize", () => { void controller.optimize(); }, "primary");
  optimize.id = "optimization-start"; optimize.disabled = !controller.canOptimize;
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
    h("div", { class: "optimization-input-actions" }, optimize),
    controller.saving ? h("div", { role: "status", class: "workspace-progress" }, controller.phase ?? "Saving…", " ", h("span", { "data-elapsed-start": String(controller.startedAt) })) : null,
    controller.run ? runState(controller, actions) : null);
}

function runState(controller: OptimizationSetupController, actions: Actions): HTMLElement {
  const run = controller.run!, phase = controller.runPhase;
  const labels = { checking_inputs: "Checking inputs", preparing_data: "Preparing data", starting: "Starting", training: "Training", saving_candidate: "Saving candidate", evaluating: "Evaluating", complete: "Done" } as const;
  const result = run.state === "candidate_accepted" ? "Candidate passed" : run.state === "candidate_rejected" ? "Candidate did not pass" : run.state === "baseline_retained" ? "No improvement" : undefined;
  const phases = ["checking_inputs", "preparing_data", "starting", "training", "saving_candidate", "evaluating"] as const;
  const active = phase === "complete" ? phases.length : Math.max(0, phases.indexOf(phase ?? "checking_inputs"));
  const activity = controller.activity, progress = activity?.progress;
  const current = result ?? (controller.running && progress ? inputRunStageLabel(progress.phase) : labels[phase ?? "checking_inputs"]);
  return h("section", { class: "optimization-progress", "aria-live": "polite", "aria-busy": String(controller.running) },
    h("div", { class: "optimization-progress-state" }, h("strong", {}, current),
      controller.running && controller.startedAt ? h("span", { "data-elapsed-start": String(controller.startedAt) }) : null),
    progress?.completed !== undefined && progress.total !== undefined ? h("div", { class: "optimization-live-progress" },
      h("progress", { value: progress.completed, max: progress.total }), h("span", {}, `${progress.completed.toLocaleString()} / ${progress.total.toLocaleString()}`)) : null,
    h("ol", { class: "optimization-progress-steps" }, ...phases.map((item, index) => h("li", { class: index < active ? "complete" : index === active ? "active" : "" }, labels[item]))),
    activity?.stages.length ? h("ol", { class: "optimization-live-stages" }, ...activity.stages.slice(-4).map(stage => h("li", {}, inputRunStageLabel(stage)))) : null,
    !controller.running ? button("Runs", () => actions.navigate({ page: "runs" }), "ghost", "arrow") : null);
}
