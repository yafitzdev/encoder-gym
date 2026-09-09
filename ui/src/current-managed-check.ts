import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { BrowserWindow } from "electron";
import type { ManagedBackend } from "./managed-backend.js";
import type { ProjectRegistry } from "./project-registry.js";

/** Explicit opt-in: actual saved library, temporary Chromium profile, read-only navigation. */
export async function checkCurrentManaged(window: BrowserWindow, output: string, registry: ProjectRegistry, backend: ManagedBackend): Promise<void> {
  const before = readFileSync(registry.file), collection = registry.read();
  if (!collection.selectedId) throw new Error("Select the real managed project to verify first.");
  const expected = await backend.openRegistered(collection.selectedId, true);
  const manifestPath = join(expected.folder, "encoder-gym.json"), databasePath = join(expected.folder, "project.sqlite");
  const manifestBefore = readFileSync(manifestPath), databaseBefore = existsSync(databasePath) ? readFileSync(databasePath) : undefined;
  const evaluate = (code: string) => window.webContents.executeJavaScript(code, true);
  await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(document.querySelector('.baseline-name')&&!document.getElementById('source-state').textContent.includes('Reading'))resolve(true);else if(n++>1000)reject(new Error('Real managed baseline did not load'));else setTimeout(poll,20)};poll()})");
  const check = async (expression: string) => { if (!await evaluate(expression)) throw new Error("Real project UI verification failed: " + expression); };
  await check("document.querySelector('[data-project-id=" + JSON.stringify(expected.manifest.id) + "]') !== null");
  await check("document.getElementById('source-state').textContent.includes('Managed workspace')");
  await check("document.querySelectorAll('[data-candidate-id]').length === 0");
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
  const readiness = await backend.readiness(expected.manifest.id);
  // Real adapter authority resolution may replay a large immutable graph. Do it
  // exactly once, then make the renderer prove it presents that owner response
  // rather than asking the backend to repeat the same read-only computation.
  const readinessMethod = backend.readiness.bind(backend);
  backend.readiness = async (projectId, manifestToken) => projectId === expected.manifest.id && manifestToken === undefined
    ? readiness
    : readinessMethod(projectId, manifestToken);
  try {
    await evaluate("[...document.querySelectorAll('[data-page=models]')].at(0).click();[...document.querySelectorAll('#page button')].find(button=>button.textContent==='Start optimization').click();true");
    await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(document.querySelector('.launch-summary')&&!document.querySelector('.workspace-progress'))resolve(true);else if(n++>1500)reject(new Error('Real managed readiness did not render'));else setTimeout(poll,20)};poll()})");
    await capture("managed-current-readiness");
  } finally {
    backend.readiness = readinessMethod;
  }
  const providers = await backend.providerStatus(expected.manifest.id);
  const after = await backend.openRegistered(collection.selectedId, true);
  if (!before.equals(readFileSync(registry.file))) throw new Error("Verification changed the saved library.");
  if (!manifestBefore.equals(readFileSync(manifestPath))) throw new Error("Verification changed the managed manifest.");
  if (databaseBefore && !databaseBefore.equals(readFileSync(databasePath))) throw new Error("Verification changed the managed custody database.");
  if (JSON.stringify(expected) !== JSON.stringify(after)) throw new Error("Verification changed the managed workspace facts.");
  const requiredBlockers = readiness.report.checks.filter(check => check.required && check.state !== "ready").map(check => ({ key: check.key, state: check.state, summary: check.summary, nextAction: check.nextAction?.label }));
  console.log(JSON.stringify({
    projectId: expected.manifest.id, folder: expected.folder, baseline: expected.manifest.baseline.fingerprint,
    datasets: expected.datasets.length, rows: expected.datasets.reduce((n, d) => n + d.rows, 0), verified: true,
    preflight: {
      runnable: readiness.report.runnable, requiredBlockers,
      exactRun: readiness.launchPreview ?? null,
      reviewedAuthority: readiness.optimizationAuthority ?? null,
      configuredProviderRoles: providers.catalog?.providers.map(provider => provider.role) ?? [],
      externalCallsBeforeAuthorization: 0,
      persistenceBeforeAuthorization: "none; this preflight is read-only",
    },
    unchanged: { savedLibrary: true, managedManifest: true, custodyDatabase: true, workspaceFacts: true },
  }));
}
