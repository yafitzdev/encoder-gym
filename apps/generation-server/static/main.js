import { api } from "/assets/api.js";
import { setupBackendConfig } from "/assets/components/backend-config.js";
import { renderCoverage } from "/assets/components/coverage-table.js";
import { setupDatasetForm } from "/assets/components/dataset-form.js";
import { setupGeneratedRowsTable } from "/assets/components/generated-rows-table.js";
import { setupGenerationControls } from "/assets/components/generation-controls.js";
import { renderGenerationProgress } from "/assets/components/generation-progress.js";

const state = { dataset: null, cells: [], plan: null, job: null };
const error = document.querySelector("#global-error");
window.addEventListener("unhandledrejection", (event) => {
  error.textContent = event.reason?.message || String(event.reason);
});

const datasetForm = setupDatasetForm({ onCreated: loadDataset, onLoaded: loadDataset });
const loadBackend = setupBackendConfig();
const rowsTable = setupGeneratedRowsTable({ getDatasetId: () => state.dataset?.id });
setupGenerationControls({
  getPlanId: () => state.plan?.id,
  onJob: async (job) => {
    state.job = job;
    renderGenerationProgress(document.querySelector("#generation-progress"), job);
    await refreshCoverage();
  },
  onFinished: async () => {
    await refreshCoverage();
    await rowsTable.refresh();
  },
});

document.querySelector("#apply-default-target").addEventListener("click", () => {
  const target = document.querySelector("#default-target").value;
  document.querySelectorAll(".cell-target").forEach((input) => { input.value = target; });
  updatePlanSummary();
});
document.querySelector("#plan-table").addEventListener("input", updatePlanSummary);
document.querySelector("#save-plan").addEventListener("click", savePlan);
document.querySelector("#export-jsonl").addEventListener("click", () => exportDataset("jsonl"));
document.querySelector("#export-csv").addEventListener("click", () => exportDataset("csv"));

async function loadDataset(dataset) {
  state.dataset = dataset;
  state.plan = null;
  state.job = null;
  state.cells = await api(`/api/datasets/${dataset.id}/cells`);
  renderPlanCells();
  renderCoverage(document.querySelector("#coverage-table"), []);
  await rowsTable.refresh();
}

function renderPlanCells() {
  const table = document.querySelector("#plan-table");
  const head = document.createElement("thead");
  const header = document.createElement("tr");
  for (const label of ["Label", "Dimensions", "Target"]) {
    const th = document.createElement("th");
    th.textContent = label;
    header.append(th);
  }
  head.append(header);
  const body = document.createElement("tbody");
  const target = document.querySelector("#default-target").value;
  state.cells.forEach((cell, index) => {
    const row = document.createElement("tr");
    const label = document.createElement("td");
    label.textContent = cell.label;
    const dimensions = document.createElement("td");
    dimensions.textContent = Object.entries(cell.dimensions)
      .map(([name, value]) => `${name}=${value}`).join(", ") || "—";
    const count = document.createElement("td");
    const input = document.createElement("input");
    input.type = "number";
    input.min = "0";
    input.value = target;
    input.dataset.index = index;
    input.className = "cell-target";
    count.append(input);
    row.append(label, dimensions, count);
    body.append(row);
  });
  table.replaceChildren(head, body);
  updatePlanSummary();
}

function plannedCells() {
  return [...document.querySelectorAll(".cell-target")].map((input) => ({
    cell: state.cells[Number(input.dataset.index)],
    target_count: Number(input.value),
  }));
}

function updatePlanSummary() {
  const targets = [...document.querySelectorAll(".cell-target")]
    .map((input) => Number(input.value || 0));
  const total = targets.reduce((sum, value) => sum + value, 0);
  document.querySelector("#plan-summary").textContent = `${targets.length} cells, ${total} total target rows`;
}

async function savePlan() {
  if (!state.dataset) throw new Error("Create or load a dataset first.");
  state.plan = await api("/api/plans", {
    method: "POST",
    body: JSON.stringify({ dataset_id: state.dataset.id, cells: plannedCells() }),
  });
  document.querySelector("#plan-summary").textContent += ` — saved as ${state.plan.id}`;
  await refreshCoverage();
}

async function refreshCoverage() {
  if (!state.plan) return;
  const coverage = await api(`/api/plans/${state.plan.id}/coverage`);
  renderCoverage(document.querySelector("#coverage-table"), coverage);
}

function exportDataset(format) {
  if (!state.dataset) throw new Error("Create or load a dataset first.");
  window.location.assign(`/api/datasets/${state.dataset.id}/export?format=${format}`);
}

Promise.all([datasetForm.refreshDatasets(), loadBackend()]).catch((failure) => {
  error.textContent = failure.message;
});
