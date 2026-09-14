import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { discoverProviderModels, ProviderConnectionStore, providerConnectionSecretId } from "../dist/evidence/provider-connections.js";

test("one project can keep many discovered connections and assign their models independently", () => {
  const root = mkdtempSync(join(tmpdir(), "gym-provider-connections-")), projectId = randomUUID();
  const store = new ProviderConnectionStore(join(root, "connections.json"));
  const deepseek = store.add(projectId, "https://api.deepseek.com/", ["deepseek-pro", "deepseek-flash", "deepseek-flash"]);
  const local = store.add(projectId, "https://yan.example.test/v1", ["qwen-large"]);
  const assigned = store.assign(projectId, {
    advisor: { connectionId: deepseek.id, model: "deepseek-flash" },
    generation: { connectionId: deepseek.id, model: "deepseek-pro" },
  });
  assert.equal(store.get(projectId).connections.length, 2);
  assert.equal(assigned.advisor.model, "deepseek-flash");
  assert.equal(assigned.generation.model, "deepseek-pro");
  assert.equal(store.get(projectId).assignments.advisor.connectionId, deepseek.id);
  assert.throws(() => store.remove(projectId, deepseek.id), /Assign another model/);
  store.update(projectId, deepseek.id, ["deepseek-pro"]);
  assert.equal(store.get(projectId).assignments.advisor, undefined, "a removed provider model cannot remain assigned");
  assert.equal(store.get(projectId).assignments.generation.model, "deepseek-pro");
  store.assign(projectId, {
    advisor: { connectionId: local.id, model: "qwen-large" },
    generation: { connectionId: local.id, model: "qwen-large" },
  });
  store.remove(projectId, deepseek.id);
  assert.deepEqual(store.get(projectId).connections.map(item => item.id), [local.id]);
  assert.equal(readFileSync(store.file, "utf8").includes("api-key"), false);
});

test("model discovery queries the OpenAI-compatible models endpoint with bearer auth", async () => {
  let request;
  const discovered = await discoverProviderModels("https://provider.example.test/v1/", "valid-api-key", async (url, init) => {
    request = { url: String(url), init };
    return new Response(JSON.stringify({ data: [{ id: "model-b" }, { id: "model-a" }] }), { status: 200, headers: { "content-type": "application/json" } });
  });
  assert.equal(request.url, "https://provider.example.test/v1/models");
  assert.equal(request.init.headers.authorization, "Bearer valid-api-key");
  assert.deepEqual(discovered.models, ["model-a", "model-b"]);
});

test("model discovery never includes a provider response body in its error", async () => {
  await assert.rejects(() => discoverProviderModels("https://provider.example.test", "valid-api-key", async () =>
    new Response("secret provider diagnostic", { status: 401 })), error => {
      assert.match(error.message, /HTTP 401/);
      assert.equal(error.message.includes("secret provider diagnostic"), false);
      return true;
    });
});
