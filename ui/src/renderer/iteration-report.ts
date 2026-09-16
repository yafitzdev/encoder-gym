import type { OptimizationIteration } from "../optimization-history.js";
import type { Actions, Location } from "./actions.js";
import { h } from "./dom.js";
import { button } from "./components.js";
import { comparisonTone } from "./overview-records.js";
import { delta, metricInfo, score, suiteName } from "./catalog.js";

/** These are recorded development verdicts, never promotion or holdout approval. */
export function iterationReport(iteration: OptimizationIteration | undefined, actions: Actions): HTMLElement {
  if (!iteration) return h("p", { class: "muted" }, "Select an iteration to inspect its report.");
  const decision = iteration.developmentPassed === null ? undefined : iteration.developmentPassed ? "KEEP" : "REJECT";
  const modelId = iteration.modelId, training = iteration.trainingDatasetVersionId, qualified = iteration.qualifiedDatasetVersionId;
  const open = (location: Location): void => actions.navigate({ ...location, refreshProject: true });
  return h("div", { class: "focus-report", "data-iteration-report": String(iteration.number) },
    h("h2", { class: decision === "KEEP" ? "success" : decision === "REJECT" ? "danger" : "muted" }, decision ? `Development · ${decision}` : iteration.noChange ? "No change proposed" : "Development report pending"),
    h("p", { class: "muted" }, iteration.selected ? "Selected best development candidate so far. Final approval and promotion are separate."
      : decision === "KEEP" ? "Passed the recorded development gates; another iteration is selected."
      : decision === "REJECT" ? "Did not pass the recorded development gates. The trained model is retained."
      : iteration.noChange ? "This iteration produced no new candidate." : "No complete development decision is recorded yet."),
    h("div", { class: "focus-controls" },
      modelId ? button("View model", () => open({ page: "model", id: modelId }), "secondary") : null,
      button("Starting model", () => open({ page: "model", id: iteration.startingModelId }), "ghost"),
      button("Input dataset", () => open({ page: "dataset", id: iteration.inputDatasetVersionId }), "ghost"),
      qualified ? button("Dataset changes", () => open({ page: "dataset", id: qualified, tab: "changes" }), "secondary") : null,
      training ? button("Training dataset", () => open({ page: "dataset", id: training, tab: "rows" }), "secondary") : null,
      button("Evaluation reports", () => open({ page: "benchmarks", id: iteration.benchmarkVersionId, tab: "results" }), "secondary")),
    iteration.checks.length ? h("p", { class: "muted" }, "Recorded comparisons against this run’s original baseline") : null,
    iteration.checks.length ? h("div", { class: "focus-table-scroll" }, h("table", { class: "focus-comparison", "aria-label": "Original baseline comparison" },
      h("thead", {}, h("tr", {}, ...["Benchmark", "Baseline", "Candidate", "Change", "Gate"].map(label => h("th", { scope: "col" }, label)))),
      h("tbody", {}, ...iteration.checks.map(check => {
        const tone = comparisonTone(check.baseline, check.candidate, check.direction);
        return h("tr", { "data-report-id": check.reportId }, h("th", { scope: "row" }, suiteName(check.suite), h("small", {}, metricInfo(check.metric).label)),
          h("td", {}, score(check.baseline, check.metric)), h("td", { class: tone }, score(check.candidate, check.metric)),
          h("td", { class: tone }, delta(check.candidate - check.baseline, check.metric)), h("td", {}, check.passed ? "Pass" : "Fail"));
      })))) : null);
}
