export function setupDimensionEditor(container, addButton) {
  const addRow = (name = "", values = "") => {
    const row = document.createElement("div");
    row.className = "dimension-row";
    row.innerHTML = `
      <input class="dimension-name" placeholder="dimension name" value="${escapeAttribute(name)}">
      <input class="dimension-values" placeholder="value1, value2" value="${escapeAttribute(values)}">
      <button type="button">Remove</button>`;
    row.querySelector("button").addEventListener("click", () => row.remove());
    container.append(row);
  };
  addButton.addEventListener("click", () => addRow());
  addRow();

  return {
    values() {
      return [...container.querySelectorAll(".dimension-row")]
        .map((row) => ({
          name: row.querySelector(".dimension-name").value.trim(),
          values: row.querySelector(".dimension-values").value
            .split(",")
            .map((value) => value.trim())
            .filter(Boolean),
        }))
        .filter((dimension) => dimension.name || dimension.values.length);
    },
  };
}

function escapeAttribute(value) {
  return value.replaceAll("&", "&amp;").replaceAll('"', "&quot;");
}
