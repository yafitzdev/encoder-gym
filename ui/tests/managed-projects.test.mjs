import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, renameSync, readFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { ManagedBackend, managedSnapshot } from "../dist/evidence/managed-backend.js";
import { ProjectRegistry } from "../dist/evidence/project-registry.js";
import { writeLocalModel } from "./fixtures/local-model.mjs";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "gym-managed-ui-"));
  const registry = new ProjectRegistry(join(root, "profile", "projects.json"));
  const backend = new ManagedBackend(resolve("../target/debug/synth" + (process.platform === "win32" ? ".exe" : "")), registry);
  const source = join(root, "model"); writeLocalModel(source);
  return { root, registry, backend, source };
}
async function create(f, name) {
  const model = await f.backend.chooseModel(f.source), parent = f.backend.chooseParent(f.root);
  const collection = await f.backend.create({ modelToken: model.token, parentToken: parent.token, name, folderName: name, task: "" });
  return collection.selectedId;
}

test("managed project identity survives independent profiles, move, rename, forget and reopen", async () => {
  const f = fixture(), id = await create(f, "Managed one");
  const workspace = await f.backend.openRegistered(id, true);
  assert.equal(managedSnapshot(workspace).runs.length, 0);
  assert.equal(managedSnapshot(workspace).baseline.fingerprint, workspace.manifest.baseline.fingerprint);
  assert.equal(f.registry.get(id).source.workspaceId, id);
  f.registry.rename(id, "My nickname");
  const moved = join(f.root, "moved"); renameSync(join(f.root, "Managed one"), moved);
  await assert.rejects(() => f.backend.openRegistered(id));
  f.registry.addManaged(await f.backend.open(moved, true));
  assert.equal(f.registry.get(id).name, "My nickname");
  const differentProfile = new ProjectRegistry(join(f.root, "another-profile.json"));
  assert.equal(differentProfile.addManaged(await f.backend.open(moved)).selectedId, id);
  f.registry.remove(id);
  assert.ok(existsSync(join(moved, "encoder-gym.json")));
  assert.equal(f.registry.addManaged(await f.backend.open(moved)).selectedId, id);
});

test("model confirmation and dataset choices are fenced to native-picked inputs and their own project", async () => {
  const f = fixture(), first = await create(f, "First"), second = await create(f, "Second");
  await assert.rejects(() => f.backend.create({ modelToken: "forged", parentToken: "forged", name: "Third", folderName: "Third", task: "" }), /Choose the checkpoint/);
  const data = join(f.root, "input.jsonl"); writeFileSync(data, '{"text":"source only"}\n');
  const before = readFileSync(data);
  const choice = await f.backend.chooseDataset(first, data, "training");
  await assert.rejects(() => f.backend.importDataset(second, choice.token, "Wrong project"), /for this project/);
  const result = await f.backend.importDataset(first, choice.token, "Training source");
  assert.equal(result.datasets[0].rows, 1);
  assert.equal((await f.backend.openRegistered(second)).datasets.length, 0);
  assert.deepEqual(readFileSync(data), before);
  const sealed = join(f.root, "held-out.jsonl"); writeFileSync(sealed, '{"evaluation_partition":"sealed"}\n');
  await assert.rejects(() => f.backend.chooseDataset(first, sealed, "training"), /non-training partition/);
  assert.equal((await f.backend.openRegistered(first)).datasets.length, 1);
});

test("opening arbitrary repositories fails without mutation and explicit legacy replacement is metadata only", async () => {
  const f = fixture();
  const repo = join(f.root, "source-repo"); mkdirSync(repo); writeFileSync(join(repo, "README.md"), "original");
  const legacy = f.registry.addFolder(repo, "Legacy source").selectedId;
  await assert.rejects(() => f.backend.open(repo), /not an Encoder Gym workspace/);
  const id = await create(f, "Clean project");
  f.registry.addManaged(await f.backend.openRegistered(id), legacy);
  assert.equal(f.registry.read().projects.length, 1);
  assert.equal(readFileSync(join(repo, "README.md"), "utf8"), "original");
  assert.ok(!existsSync(join(repo, "encoder-gym.json")));
  const project = f.registry.get(id);
  // A copied folder from another identity must not replace this project on read.
  const second = await create(f, "Another");
  const collection = f.registry.read();
  collection.projects.find(p => p.id === id).source.path = f.registry.get(second).source.path;
  collection.projects = collection.projects.filter(p => p.id !== second);
  collection.selectedId = id;
  writeFileSync(f.registry.file, JSON.stringify(collection));
  await assert.rejects(() => f.backend.openRegistered(id), /different Gym project/);
  assert.equal(project.source.workspaceId, id);
});
