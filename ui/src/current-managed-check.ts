import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { BrowserWindow } from "electron";
import { managedSnapshot, type ManagedBackend } from "./managed-backend.js";
import type { ProjectRegistry } from "./project-registry.js";
import { modelInventory } from "./renderer/model-inventory.js";

/** Actual saved library, temporary Chromium profile; observation only. */
export async function checkCurrentManaged(window: BrowserWindow, output: string, registry: ProjectRegistry, backend: ManagedBackend): Promise<void> {
  const before = readFileSync(registry.file), collection = registry.read();
  if (!collection.selectedId) throw new Error("Select a managed project first.");
  const expected = await backend.openRegistered(collection.selectedId, true);
  const models = modelInventory(managedSnapshot(expected));
  const manifestPath = join(expected.folder, "encoder-gym.json"), databasePath = join(expected.folder, "project.sqlite");
  const manifestBefore = readFileSync(manifestPath), databaseBefore = existsSync(databasePath) ? readFileSync(databasePath) : undefined;
  const evaluate = (code: string) => window.webContents.executeJavaScript(code, true);
  const until = async (expression: string) => evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(" + expression + ")resolve(true);else if(n++>1500)reject(new Error('Timed out: '+" + JSON.stringify(expression) + "));else setTimeout(poll,20)};poll()})");
  const check = async (expression: string) => { if (!await evaluate(expression)) throw new Error("Real project UI verification failed: " + expression); };
  await until("document.querySelector('.artifact-row') && !document.getElementById('source-state').textContent.includes('Reading')");
  await check(`document.querySelectorAll('.artifact-row').length === ${models.length}`);
  await check("document.querySelectorAll('.artifact-row .tag.accent').length === 1 && !document.getElementById('project-optimize').hidden");
  mkdirSync(output, { recursive: true });
  const capture = async (name: string) => {
    await evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
    writeFileSync(join(output, name + ".png"), (await window.webContents.capturePage()).toPNG());
  };
  await capture("managed-current-models");
  for (const model of models) {
    await evaluate("document.getElementById(" + JSON.stringify("model-" + model.id) + ").click()");
    await check("document.querySelector('.model-view') && document.querySelectorAll('.tabs button').length === 4");
    await check("document.querySelector('.model-view h1').textContent === " + JSON.stringify(model.name));
    await capture(model.role === "Baseline" ? "managed-current-baseline" : "managed-current-candidate");
    if (model.evidence) {
      await evaluate("document.getElementById('tab-results').click()");
      await check("document.querySelector('.result-banner') && document.querySelector('.evidence-table')");
      await capture("managed-current-candidate-results");
    }
    await evaluate("document.getElementById('nav-models').click()");
  }
  for (const page of ["datasets", "benchmarks", "runs", "project"]) {
    await evaluate("document.getElementById(" + JSON.stringify("nav-" + page) + ").click()");
    await until("document.querySelector('.workspace-page') && !document.querySelector('.workspace-progress')");
    await check("!document.getElementById('project-optimize').hidden");
    await capture("managed-current-" + page);
  }
  await evaluate("document.getElementById('project-optimize').click()");
  await until("document.querySelector('.optimization-page') && !document.querySelector('.workspace-progress') && !document.querySelector('.optimization-page [role=status]')");
  await capture("managed-current-readiness");
  const after = await backend.openRegistered(collection.selectedId, true);
  if (!before.equals(readFileSync(registry.file))) throw new Error("Verification changed the saved library.");
  if (!manifestBefore.equals(readFileSync(manifestPath))) throw new Error("Verification changed the managed manifest.");
  if (databaseBefore && !databaseBefore.equals(readFileSync(databasePath))) throw new Error("Verification changed the project database.");
  if (JSON.stringify(expected) !== JSON.stringify(after)) throw new Error("Verification changed workspace facts.");
  console.log(JSON.stringify({ projectId: expected.manifest.id, models: models.map(m => ({ id: m.id, name: m.name, role: m.role, hasEvaluation: Boolean(m.evidence) })), unchanged: true }));
}
