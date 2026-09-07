import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, renameSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { ProjectRegistry } from "../dist/evidence/project-registry.js";
import { ProjectSelection } from "../dist/evidence/projects.js";

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "encoder-gym-projects-"));
  const file = join(root, "preferences", "projects.json");
  const one = join(root, "one"), two = join(root, "two");
  mkdirSync(one); mkdirSync(two);
  return { root, file, one, two, registry: new ProjectRegistry(file) };
}
test("empty collection and two independent project folders survive a fresh registry instance", () => {
  const f = fixture();
  assert.deepEqual(f.registry.read(), { version: 1, selectedId: null, projects: [] });
  const a = f.registry.addFolder(f.one, "First encoder").selectedId;
  const b = f.registry.addFolder(f.two, "Second encoder").selectedId;
  assert.notEqual(a, b);
  f.registry.select(a);
  const restarted = new ProjectRegistry(f.file);
  assert.equal(restarted.read().selectedId, a);
  assert.deepEqual(restarted.read().projects.map(p => p.name), ["First encoder", "Second encoder"]);
  assert.equal(restarted.addFolder(f.one).projects.length, 2);
  assert.equal(restarted.read().selectedId, a);
});
test("renaming and reconnecting preserve project identity; removing never deletes project data", () => {
  const f = fixture();
  const a = f.registry.addFolder(f.one).selectedId;
  writeFileSync(join(f.one, "model.txt"), "immutable-model");
  const moved = join(f.root, "moved");
  renameSync(f.one, moved);
  assert.equal(f.registry.select(a).selectedId, a, "missing folders remain registered for recovery");
  f.registry.rename(a, "Renamed encoder");
  f.registry.relocate(a, moved);
  assert.equal(f.registry.get(a).source.path, moved);
  assert.equal(f.registry.get(a).name, "Renamed encoder");
  f.registry.addFolder(f.two);
  const before = readFileSync(f.file, "utf8");
  assert.throws(() => f.registry.relocate(a, f.two), /already belongs/);
  assert.equal(readFileSync(f.file, "utf8"), before);
  f.registry.remove(a);
  assert.equal(readFileSync(join(moved, "model.txt"), "utf8"), "immutable-model");
  assert.equal(existsSync(moved), true);
});
test("invalid preferences and invalid names never erase existing metadata", () => {
  const f = fixture();
  const a = f.registry.addFolder(f.one).selectedId;
  const valid = readFileSync(f.file, "utf8");
  for (const name of ["", "  ", "x".repeat(121), "bad\nname"]) assert.throws(() => f.registry.rename(a, name));
  assert.equal(readFileSync(f.file, "utf8"), valid);
  writeFileSync(f.file, "broken json");
  assert.throws(() => f.registry.addFolder(f.two), /Cannot read project collection/);
  assert.equal(readFileSync(f.file, "utf8"), "broken json");
});
test("examples are explicit, persistent, and not mistaken for connected folders", () => {
  const f = fixture();
  assert.equal(f.registry.read().projects.length, 0);
  const c = f.registry.addExample("recorded-demo", "Recorded example");
  assert.equal(c.projects[0].source.kind, "example");
  assert.equal(f.registry.addExample("recorded-demo", "Recorded example").projects.length, 1);
  assert.throws(() => f.registry.relocate(c.selectedId, f.one), /Only a local/);
});
test("out-of-order project responses and errors cannot leak across a selection", async () => {
  const selection = new ProjectSelection();
  let finishA;
  const a = selection.open("a", () => new Promise(resolve => { finishA = resolve; }));
  const b = await selection.open("b", async id => ({ project: { id }, content: { state: "empty", databases: [] } }));
  finishA({ project: { id: "a" }, content: { state: "ready", workspace: { name: "Wrong project" } } });
  assert.equal(await a, undefined);
  assert.equal(b.project.id, "b");
  assert.equal(selection.selectedId, "b");
  let fail;
  const old = selection.open("a", () => new Promise((_, reject) => { fail = reject; }));
  selection.invalidate(null);
  fail(new Error("Old folder unreadable"));
  assert.equal(await old, undefined);
  await assert.rejects(() => selection.open("b", async () => ({ project: { id: "a" } })), /does not match/);
});
