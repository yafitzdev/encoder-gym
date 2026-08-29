export function renderCoverage(table, coverage) {
  const head = document.createElement("thead");
  const header = document.createElement("tr");
  for (const label of ["Label", "Dimensions", "Target", "Attempted", "Accepted", "Rejected", "Remaining"]) {
    const th = document.createElement("th");
    th.textContent = label;
    header.append(th);
  }
  head.append(header);
  const body = document.createElement("tbody");
  for (const item of coverage) {
    const row = document.createElement("tr");
    const values = [
      item.cell.label,
      formatDimensions(item.cell.dimensions),
      item.target,
      item.attempted,
      item.accepted,
      item.rejected,
      item.remaining,
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

function formatDimensions(dimensions) {
  return Object.entries(dimensions).map(([name, value]) => `${name}=${value}`).join(", ") || "—";
}
