import { api } from "/assets/api.js";

export function setupGeneratedRowsTable({ getDatasetId }) {
  const table = document.querySelector("#rows-table");
  const filter = document.querySelector("#row-status-filter");

  async function refresh() {
    const datasetId = getDatasetId();
    if (!datasetId) return;
    const status = filter.value ? `&status=${filter.value}` : "";
    const rows = await api(`/api/rows?dataset_id=${datasetId}&limit=200${status}`);
    render(table, rows);
  }

  document.querySelector("#refresh-rows").addEventListener("click", refresh);
  filter.addEventListener("change", refresh);
  return { refresh };
}

function render(table, rows) {
  const head = document.createElement("thead");
  const header = document.createElement("tr");
  for (const label of ["Status", "Text", "Label", "Dimensions", "Backend", "Validation errors"]) {
    const th = document.createElement("th");
    th.textContent = label;
    header.append(th);
  }
  head.append(header);
  const body = document.createElement("tbody");
  for (const item of rows) {
    const row = document.createElement("tr");
    const values = [
      item.validation_status,
      item.text,
      item.label,
      Object.entries(item.dimensions).map(([name, value]) => `${name}=${value}`).join(", "),
      `${item.generator_backend} / ${item.generator_model}`,
      item.validation_errors.join("; "),
    ];
    for (const value of values) {
      const cell = document.createElement("td");
      cell.textContent = value;
      row.append(cell);
    }
    body.append(row);
  }
  table.replaceChildren(head, body);
}
