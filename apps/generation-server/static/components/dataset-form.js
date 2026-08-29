import { api } from "/assets/api.js";
import { setupDimensionEditor } from "/assets/components/dimension-editor.js";

export function setupDatasetForm({ onCreated, onLoaded }) {
  const form = document.querySelector("#dataset-form");
  const status = document.querySelector("#dataset-status");
  const select = document.querySelector("#dataset-select");
  const dimensions = setupDimensionEditor(
    document.querySelector("#dimension-editor"),
    document.querySelector("#add-dimension"),
  );

  async function refreshDatasets(selectedId) {
    const datasets = await api("/api/datasets");
    select.replaceChildren();
    for (const dataset of datasets) {
      const option = new Option(dataset.name, dataset.id, false, dataset.id === selectedId);
      select.add(option);
    }
  }

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const data = new FormData(form);
    const labels = data.get("labels").split(/[\n,]/).map((value) => value.trim()).filter(Boolean);
    const dataset = await api("/api/datasets", {
      method: "POST",
      body: JSON.stringify({
        name: data.get("name"),
        task_description: data.get("task_description"),
        labels,
        dimensions: dimensions.values(),
      }),
    });
    status.textContent = `Created ${dataset.name} (${dataset.id})`;
    await refreshDatasets(dataset.id);
    await onCreated(dataset);
  });

  document.querySelector("#load-dataset").addEventListener("click", async () => {
    if (!select.value) return;
    const dataset = await api(`/api/datasets/${select.value}`);
    status.textContent = `Loaded ${dataset.name} (${dataset.id})`;
    await onLoaded(dataset);
  });

  return { refreshDatasets };
}
