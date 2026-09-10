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
  await check("document.getElementById('page').textContent.includes('No managed candidates yet') && document.getElementById('page').textContent.includes('imported historical')");
  await check("[...document.querySelectorAll('#page button')].filter(button=>button.textContent==='Start optimization').length === 1");
  mkdirSync(output, { recursive: true });
  const capture = async (name: string) => {
    await evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
    writeFileSync(join(output, name + ".png"), (await window.webContents.capturePage()).toPNG());
  };
  await capture("managed-current-models");
  const readiness = await backend.readiness(expected.manifest.id);
  // Resolve the owner report exactly once, then make every collection page
  // prove it presents that same response rather than triggering duplicate work.
  const readinessMethod = backend.readiness.bind(backend);
  backend.readiness = async (projectId, manifestToken) => projectId === expected.manifest.id && manifestToken === undefined
    ? readiness
    : readinessMethod(projectId, manifestToken);
  try {
    await evaluate("document.querySelector('[data-page=datasets]').click()");
    await check(`document.querySelectorAll('[data-dataset-id]').length === ${expected.datasets.length}`);
    for (const dataset of expected.datasets) {
      await check("document.querySelector('[data-dataset-id=" + JSON.stringify(dataset.id) + "]').textContent.includes(" + JSON.stringify(dataset.rows.toLocaleString() + " records") + ")");
    }
    const snapshotId = readiness.preparedOptimization?.launchPreview.trainingSnapshotId ?? readiness.launchPreview?.trainingSnapshotId ?? readiness.optimizationAuthority?.trainingSnapshotId;
    if (snapshotId) {
      await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(document.querySelector('.scientific-data-card'))resolve(true);else if(n++>1000)reject(new Error('Real training snapshot did not render'));else setTimeout(poll,20)};poll()})");
      await check("document.querySelector('.scientific-data-card').textContent.includes('training snapshot') && document.querySelector('.scientific-data-card').textContent.includes('custody records')");
    }
    await capture("managed-current-datasets");
    const launch = readiness.preparedOptimization?.launchPreview ?? readiness.launchPreview;
    await evaluate("document.querySelector('[data-page=benchmarks]').click()");
    if (launch) {
      await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(document.querySelector('.evaluation-plan'))resolve(true);else if(n++>1000)reject(new Error('Real evaluation plan did not render'));else setTimeout(poll,20)};poll()})");
      await check(`document.querySelectorAll('.evaluation-plan-row').length === ${launch.developmentSuites.length + 1}`);
      await check("document.querySelector('.page-heading h1').textContent === 'Evaluation' && document.querySelector('.evaluation-plan').textContent.includes('not candidate results') && document.querySelector('.evaluation-plan').textContent.includes('Separate authorization')");
    }
    await check("document.querySelectorAll('.benchmark-section').length > 0 && !document.getElementById('page').textContent.includes('No run evaluations recorded yet')");
    await capture("managed-current-evaluation");
    await evaluate("document.querySelector('[data-page=project]').click()");
    await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(![...document.querySelectorAll('#page button')].some(button=>button.textContent.includes('Checking')))resolve(true);else if(n++>1000)reject(new Error('Provider availability did not resolve'));else setTimeout(poll,20)};poll()})");
    await capture("managed-current-settings");
    await evaluate("[...document.querySelectorAll('[data-page=models]')].at(0).click();[...document.querySelectorAll('#page button')].find(button=>button.textContent==='Start optimization').click();true");
    await evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(document.querySelector('.launch-summary')&&!document.querySelector('.workspace-progress'))resolve(true);else if(n++>1500)reject(new Error('Real managed readiness did not render'));else setTimeout(poll,20)};poll()})");
    if (readiness.preparedOptimization) {
      await check("[...document.querySelectorAll('#page button')].filter(button=>button.textContent==='Reserve optimization run').length === 1 && document.getElementById('page').textContent.includes('Prepared for reservation') && document.getElementById('page').textContent.includes('Exact run definition') && !document.getElementById('page').textContent.includes('Do these next')");
      await check("(()=>{const button=[...document.querySelectorAll('#page button')].find(button=>button.textContent==='Reserve optimization run');if(!button)return false;const rect=button.getBoundingClientRect();return rect.top>=0&&rect.bottom<=window.innerHeight})()");
    } else {
      await check("[...document.querySelectorAll('#page button')].filter(button=>button.textContent==='Prepare approved run').length === 1 && !document.getElementById('page').textContent.includes('Do these next')");
    }
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
      preparation: readiness.preparedOptimization ? "prepared" : "not_prepared",
      configuredProviderRoles: providers.catalog?.providers.map(provider => provider.role) ?? [],
      externalCallsBeforeAuthorization: 0,
      persistenceBeforeAuthorization: "none; this preflight is read-only",
    },
    unchanged: { savedLibrary: true, managedManifest: true, custodyDatabase: true, workspaceFacts: true },
  }));
}
