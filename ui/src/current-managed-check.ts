import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { BrowserWindow } from "electron";
import type { ManagedBackend } from "./managed-backend.js";
import type { ProjectRegistry } from "./project-registry.js";

/** Explicit opt-in: actual saved library, temporary Chromium profile, read-only navigation. */
export async function checkCurrentManaged(window: BrowserWindow, output: string, registry: ProjectRegistry, backend: ManagedBackend): Promise<void> {
  const before = readFileSync(registry.file), collection = registry.read();
  if (!collection.selectedId) throw new Error("Select the real managed project to verify first.");
  const expected = await backend.openRegistered(collection.selectedId, true);
  const evaluate = (code: string) => window.webContents.executeJavaScript(code, true);
  await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(document.querySelector('.baseline-name')&&!document.getElementById('source-state').textContent.includes('Reading'))resolve(true);else if(n++>1000)reject(new Error('Real managed baseline did not load'));else setTimeout(poll,20)};poll()})");
  const check = async (expression: string) => { if (!await evaluate(expression)) throw new Error("Real project UI verification failed: " + expression); };
  await check("document.querySelector('[data-project-id=" + JSON.stringify(expected.manifest.id) + "]') !== null");
  await check("document.getElementById('source-state').textContent.includes('Managed workspace') && document.querySelectorAll('[data-candidate-id]').length === 0");
  mkdirSync(output, { recursive: true });
  const capture = async (name: string) => {
    await evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
    writeFileSync(join(output, name + ".png"), (await window.webContents.capturePage()).toPNG());
  };
  await capture("managed-current-models");
  await evaluate("document.querySelector('[data-page=datasets]').click()");
  await check(`document.querySelectorAll('[data-dataset-id]').length === ${expected.datasets.length}`);
  for (const dataset of expected.datasets) {
    await check("document.querySelector('[data-dataset-id=" + JSON.stringify(dataset.id) + "]').textContent.includes(" + JSON.stringify(dataset.rows.toLocaleString() + " records") + ")");
  }
  await capture("managed-current-datasets");
  await evaluate("document.querySelector('[data-page=project]').click()");
  await capture("managed-current-settings");
  if (!before.equals(readFileSync(registry.file))) throw new Error("Verification changed the saved library.");
  console.log(JSON.stringify({ projectId: expected.manifest.id, folder: expected.folder, baseline: expected.manifest.baseline.fingerprint, datasets: expected.datasets.length, rows: expected.datasets.reduce((n, d) => n + d.rows, 0), verified: true, savedLibraryUnchanged: true }));
}
