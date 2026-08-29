import { api } from "/assets/api.js";

export function setupBackendConfig() {
  const form = document.querySelector("#backend-form");
  const status = document.querySelector("#backend-status");
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const data = new FormData(form);
    const optionalNumber = (name) => data.get(name) === "" ? null : Number(data.get(name));
    const configuration = await api("/api/backend", {
      method: "PUT",
      body: JSON.stringify({
        base_url: data.get("base_url"),
        model: data.get("model"),
        api_key: data.get("api_key") || null,
        temperature: optionalNumber("temperature"),
        max_tokens: optionalNumber("max_tokens"),
        seed: optionalNumber("seed"),
      }),
    });
    form.elements.api_key.value = "";
    status.textContent = `Saved ${configuration.model}; API keys remain in memory only.`;
  });

  return async function loadBackend() {
    const configuration = await api("/api/backend");
    if (!configuration) return;
    form.elements.base_url.value = configuration.base_url || "";
    form.elements.model.value = configuration.model;
    form.elements.temperature.value = configuration.parameters.temperature ?? "";
    form.elements.max_tokens.value = configuration.parameters.max_tokens ?? "";
    form.elements.seed.value = configuration.parameters.seed ?? "";
  };
}
