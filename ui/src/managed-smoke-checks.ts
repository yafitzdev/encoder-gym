import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import type { BrowserWindow } from "electron";
import type { ProjectRegistry } from "./project-registry.js";
import type { ManagedBackend } from "./managed-backend.js";

interface Harness { registry: ProjectRegistry; backend: ManagedBackend; chooseFolder(path?: string): void; restart: boolean }
export async function runManagedSmokeChecks(window: BrowserWindow, output: string, harness: Harness): Promise<void> {
  mkdirSync(output, { recursive: true });
  const web = window.webContents;
  const evaluate = (code: string) => web.executeJavaScript(code, true);
  const until = async (expression: string) => evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(" + expression + ")resolve(true);else if(n++>1000)reject(new Error('Timed out: '+" + JSON.stringify(expression) + "));else setTimeout(poll,20)};poll()})");
  const check = async (name: string, expression: string) => { if (!await evaluate(expression)) throw new Error("Managed renderer check failed: " + name); console.log("PASS " + name); };
  const click = async (selector: string) => evaluate("(()=>{const control=document.querySelector(" + JSON.stringify(selector) + ");control.focus();control.click()})()");
  // Chromium input works in hidden QA windows without stealing the user's OS focus.
  const key = async (key: "Enter" | "Tab" | "Escape") => {
    if (!web.debugger.isAttached()) web.debugger.attach("1.3");
    await web.debugger.sendCommand("Emulation.setFocusEmulationEnabled", { enabled: true });
    const input = { key, code: key, windowsVirtualKeyCode: { Enter: 13, Tab: 9, Escape: 27 }[key] };
    await web.debugger.sendCommand("Input.dispatchKeyEvent", { ...input, type: "keyDown", text: key === "Enter" ? "\r" : "" });
    await web.debugger.sendCommand("Input.dispatchKeyEvent", { ...input, type: "keyUp" });
    await evaluate("new Promise(r=>requestAnimationFrame(r))");
  };
  const textButton = async (text: string) => evaluate("[...document.querySelectorAll('#page button, dialog[open] button')].find(b=>b.textContent === " + JSON.stringify(text) + ").click()");
  const type = async (id: string, value: string) => evaluate("document.getElementById(" + JSON.stringify(id) + ").value=" + JSON.stringify(value) + ";document.getElementById(" + JSON.stringify(id) + ").dispatchEvent(new Event('input',{bubbles:true}))");
  const nav = async (page: string) => click('[data-page="' + page + '"]');
  const loaded = async () => until("!document.getElementById('source-state').textContent.includes('Reading') && document.querySelector('#page h1')");
  const screenshot = async (name: string, width = 1440, height = 960) => {
    window.setContentSize(width, height);
    await evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
    writeFileSync(join(output, name + ".png"), (await web.capturePage()).toPNG());
  };
  await until("document.querySelector('#page h1')");
  if (harness.restart) {
    await loaded();
    await check("managed projects and selected identity survive another Electron process", "document.querySelectorAll('[data-project-id]').length === 2 && document.getElementById('breadcrumb').textContent.includes('Routing encoder') && document.getElementById('source-state').textContent.includes('Managed workspace')");
    await nav("project"); await until("document.querySelectorAll('.provider-summary').length === 2 && !document.querySelector('.provider-state .neutral')");
    await check("provider settings and encrypted credential availability survive restart", "document.querySelectorAll('.provider-state .success').length === 2 && !document.getElementById('page').textContent.includes('smoke-secret')");
    await nav("datasets");
    await check("imported dataset metadata survives restart", "document.querySelectorAll('[data-dataset-id]').length === 1 && document.querySelector('.dataset-summary').textContent.includes('2 records')");
    await screenshot("managed-restarted"); return;
  }
  const root = join(harness.registry.file, "..", "managed-fixtures"); mkdirSync(root, { recursive: true });
  const { writeLocalModel } = await import(pathToFileURL(join(output, "..", "tests", "fixtures", "local-model.mjs")).href);
  const one = join(root, "model-one"), two = join(root, "model-two"); writeLocalModel(one, 1); writeLocalModel(two, 2);
  const before = readFileSync(join(one, "model.safetensors"));
  await check("onboarding starts with model-based New and managed Open", "document.querySelector('.welcome-page').textContent.includes('New project') && document.querySelector('.welcome-page').textContent.includes('Open project') && !document.querySelector('.welcome-page').textContent.includes('Nomos')");
  await click("#add-project");
  await check("creation requires a checkpoint preview and destination", "document.getElementById('confirm-new-project').disabled");
  harness.chooseFolder(); await click("#choose-local-model");
  await until("!document.querySelector('.onboarding-fields').disabled");
  await check("cancelled checkpoint picker does not create a project", "document.getElementById('confirm-new-project').disabled && document.querySelectorAll('[data-project-id]').length === 0");
  await textButton("Cancel");
  await check("cancelling onboarding restores keyboard focus to New project", "document.activeElement.id === 'add-project'");
  await key("Enter"); await until("document.querySelector('#project-dialog[open]')");
  await check("keyboard activation opens onboarding at its first input", "document.activeElement.id === 'new-project-name'");
  await key("Tab");
  await check("Tab reaches checkpoint selection in form order", "document.activeElement.id === 'choose-local-model'");
  for (let i = 0; i < 9; i++) { await key("Tab"); await check("dialog contains keyboard focus " + i, "document.getElementById('project-dialog').contains(document.activeElement)"); }
  await key("Escape"); await until("!document.querySelector('#project-dialog[open]')");
  await check("Escape restores the onboarding opener", "document.activeElement.id === 'add-project'");
  const create = async (name: string, source: string) => {
    await click("#add-project"); await type("new-project-name", name); await type("new-project-task", "Local encoder fixture");
    harness.chooseFolder(source); await click("#choose-local-model"); await until("document.getElementById('model-preview').textContent.includes('files') && !document.querySelector('.onboarding-fields').disabled");
    harness.chooseFolder(root); await click("#choose-project-parent"); await until("!document.getElementById('confirm-new-project').disabled && !document.querySelector('.onboarding-fields').disabled");
    if (name === "Routing encoder") {
      await screenshot("managed-create-preview"); await screenshot("managed-create-760", 760, 800);
      await check("creation confirmation stays visible in the small desktop dialog", "document.getElementById('confirm-new-project').getBoundingClientRect().bottom <= document.getElementById('project-dialog').getBoundingClientRect().bottom");
      await check("dialog fields never overlap fixed actions", "document.querySelector('.onboarding-fields').getBoundingClientRect().bottom <= document.querySelector('.onboarding-footer').getBoundingClientRect().top + 1");
      await check("creation preview shows the complete destination", "document.getElementById('project-parent-path').textContent.endsWith('\\\\Routing encoder') && document.getElementById('confirm-new-project-hint').textContent.includes('No training starts')");
      await screenshot("managed-create-760x560", 760, 560);
      await check("minimum desktop keeps fields scrollable and both creation actions visible", "document.querySelector('.onboarding-fields').clientHeight > 100 && document.querySelector('.onboarding-fields').scrollHeight > document.querySelector('.onboarding-fields').clientHeight && [...document.querySelectorAll('.dialog-actions button')].every(b=>b.getBoundingClientRect().bottom < innerHeight)");
      await screenshot("managed-create-390", 390, 700);
      await check("narrow dialog keeps confirmation inside viewport", "document.getElementById('confirm-new-project').getBoundingClientRect().bottom < innerHeight && document.getElementById('project-dialog').scrollWidth <= document.getElementById('project-dialog').clientWidth + 1");
      window.setContentSize(1440, 960);
    }
    await click("#confirm-new-project"); await until("!document.querySelector('#project-dialog[open]')"); await loaded();
    return harness.registry.read().selectedId!;
  };
  const firstId = await create("Routing encoder", one);
  await check("new project shows its baseline without fabricated runs or candidates", "document.querySelector('.baseline-name').textContent.includes('Routing encoder') && document.querySelectorAll('[data-candidate-id]').length === 0 && document.getElementById('source-state').textContent.includes('Managed workspace')");
  await screenshot("managed-baseline");
  await check("Models leads to the real optimization journey without dataset filler", "document.querySelector('.next-step').textContent.includes('Prepare the first bounded run') && [...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Start optimization') && !document.querySelector('.candidate-empty').textContent.includes('imported dataset')");
  await textButton("Start optimization"); await until("document.querySelector('.launch-summary') && document.querySelectorAll('.readiness-row').length > 0");
  await check("readiness distinguishes ready foundations from missing scientific authority", "document.querySelector('.readiness-list').textContent.includes('Connect the scientific runtime') && document.querySelector('.readiness-list > details').textContent.includes('foundations are ready') && document.querySelector('.readiness-list').textContent.includes('Choose the reviewed run definition')");
  await check("readiness exposes no managed paths as editable command input", "!document.querySelector('.optimization-page input') && !document.querySelector('.optimization-page').textContent.includes('project.sqlite')");
  await screenshot("managed-readiness");
  await nav("project"); await until("[...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Configure providers' && !b.disabled)");
  await check("scientific runtime setup is a real project-settings action", "[...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Connect scientific runtime') && !document.getElementById('page').textContent.includes('not implemented')");
  await textButton("Connect scientific runtime"); await until("document.querySelector('#project-dialog[open] .runtime-form')");
  await check("runtime setup explains isolation, explicit history import, and offline preview", "document.querySelector('.runtime-form').textContent.includes('original source repository is deliberately rejected') && document.querySelector('.runtime-form').textContent.includes('no provider call') && document.querySelector('.runtime-form').textContent.includes('Existing Encoder Gym history') && [...document.querySelectorAll('.runtime-form button')].find(b=>b.textContent === 'Bring existing history') && [...document.querySelectorAll('.runtime-form button')].find(b=>b.textContent === 'Connect runtime').disabled");
  await screenshot("managed-runtime-setup", 760, 760); window.setContentSize(1440, 960);
  await textButton("Cancel"); await until("!document.querySelector('#project-dialog[open]')");
  await textButton("Configure providers"); await until("document.querySelector('#project-dialog[open]')");
  await type("generation-model", "generation-smoke-model"); await type("advisor-model", "advisor-smoke-model");
  await type("generation-credential", "generation-smoke-secret-123"); await type("advisor-credential", "advisor-smoke-secret-456");
  await textButton("Save provider setup"); await until("!document.querySelector('#project-dialog[open]') && document.querySelectorAll('.provider-summary').length === 2");
  await check("separate provider authorities expose availability without secret values", "document.querySelectorAll('.provider-state .success').length === 2 && !document.getElementById('page').textContent.includes('generation-smoke-secret-123') && !document.getElementById('page').textContent.includes('advisor-smoke-secret-456')");
  const credentialIndex = readFileSync(join(harness.registry.file, "..", "credentials.json"), "utf8");
  if (credentialIndex.includes("generation-smoke-secret-123") || credentialIndex.includes("advisor-smoke-secret-456")) throw new Error("Credential plaintext reached the desktop profile");
  await screenshot("managed-provider-settings");
  await nav("models");
  await nav("runs");
  await check("managed runs point to explicit scientific history import", "document.querySelector('.empty-state').textContent.includes('Project settings') && document.querySelector('.empty-state').textContent.includes('verify and copy')");
  await nav("benchmarks");
  await check("managed benchmarks distinguish imports from evaluation", "document.querySelector('.empty-state').textContent.includes('does not create evaluation results')");
  await nav("datasets"); await textButton("Import dataset");
  const sealed = join(root, "held-out.jsonl"); writeFileSync(sealed, '{"evaluation_partition":"sealed"}\n');
  await evaluate("document.getElementById('dataset-purpose').value='training';document.getElementById('dataset-purpose').dispatchEvent(new Event('change',{bubbles:true}))");
  harness.chooseFolder(sealed); await click("#choose-dataset-file"); await until("document.querySelector('dialog .form-error').textContent.includes('non-training partition')");
  await check("training import rejects held-out rows visibly before copying", "document.getElementById('confirm-dataset-import').disabled && document.querySelectorAll('[data-dataset-id]').length === 0");
  await check("import error leads with recovery and keeps diagnostics collapsed", "document.querySelector('.form-error strong').textContent === 'This file contains held-out data' && !document.querySelector('.form-error details').open && document.querySelector('.form-error').getBoundingClientRect().bottom < innerHeight");
  await screenshot("managed-import-rejection", 760, 800); window.setContentSize(1440, 960);
  await screenshot("managed-import-error-760x560", 760, 560);
  await check("minimum desktop import error leaves input and actions reachable", "document.querySelector('.onboarding-fields').clientHeight > 80 && document.querySelector('.form-error').getBoundingClientRect().bottom <= document.querySelector('.dialog-actions').getBoundingClientRect().top && document.getElementById('confirm-dataset-import').getBoundingClientRect().bottom < innerHeight");
  window.setContentSize(1440, 960);
  const sourceData = join(root, "train.jsonl"); writeFileSync(sourceData, '{"text":"source only one","split":"train"}\n{"text":"source only two","split":"train"}\n');
  harness.chooseFolder(sourceData); await click("#choose-dataset-file"); await until("!document.getElementById('confirm-dataset-import').disabled && !document.querySelector('.onboarding-fields').disabled");
  await type("import-dataset-name", "Routing training source"); await screenshot("managed-import-preview");
  const sourceBytes = readFileSync(sourceData);
  writeFileSync(sourceData, sourceBytes.toString() + '{"text":"changed after preview","split":"train"}\n');
  await click("#confirm-dataset-import"); await until("document.querySelector('dialog .form-error').textContent.includes('Dataset changed after preview')");
  await check("failed import preserves input and creates no dataset", "document.getElementById('import-dataset-name').value === 'Routing training source' && document.querySelectorAll('[data-dataset-id]').length === 0 && document.querySelector('#project-dialog[open]')");
  writeFileSync(sourceData, sourceBytes);
  harness.chooseFolder(sourceData); await click("#choose-dataset-file"); await until("!document.querySelector('.onboarding-fields').disabled && !document.getElementById('confirm-dataset-import').disabled");
  const importData = harness.backend.importDataset.bind(harness.backend);
  let releaseImport!: () => void, importCalls = 0;
  const pendingImport = new Promise<void>(resolve => { releaseImport = resolve; });
  harness.backend.importDataset = async (...args) => { importCalls++; await pendingImport; return importData(...args); };
  try {
    await click("#confirm-dataset-import"); await until("document.getElementById('project-dialog').getAttribute('aria-busy') === 'true'");
    await screenshot("managed-import-busy-760x560", 760, 560);
    await key("Escape");
    await evaluate("document.querySelector('.onboarding-form').dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))");
    await check("slow import keeps progress visible and cannot be dismissed or resubmitted", "document.querySelector('#project-dialog[open]') && document.querySelector('.onboarding-fields').disabled && document.querySelector('.onboarding-footer').disabled && document.querySelector('.operation-status').textContent.includes('Copying and verifying') && document.querySelector('.operation-status').getBoundingClientRect().bottom < innerHeight");
    if (importCalls !== 1) throw new Error('Duplicate import reached backend');
    releaseImport(); await until("!document.querySelector('#project-dialog[open]') && document.querySelectorAll('[data-dataset-id]').length === 1");
  } finally { releaseImport(); harness.backend.importDataset = importData; }
  window.setContentSize(1440, 960);
  await check("dataset view exposes provenance and counts, not native payloads", "document.querySelector('.dataset-summary').textContent.includes('2 records') && !document.getElementById('page').textContent.includes('source only one')");
  await check("dataset details are optional and source readiness stays explicit", "!document.querySelector('[data-dataset-id] details').open && document.querySelector('.preparation-note').textContent.includes('not training-ready')");
  await click("[data-dataset-id] summary");
  await check("expanded dataset retains source identities and copy actions", "document.querySelector('[data-dataset-id] details').open && document.querySelector('[data-dataset-id] details').textContent.includes('Content identity') && document.querySelector('[data-dataset-id] .copy-field button')");
  await click("[data-dataset-id] summary");
  await screenshot("managed-datasets");
  await click("#theme-toggle"); await screenshot("managed-datasets-light"); await click("#theme-toggle");
  for (const width of [760, 390]) {
    await screenshot("managed-datasets-" + width, width, 820);
    await check("managed dataset page fits width " + width, "document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth");
  }
  window.setContentSize(1440, 960);
  const secondId = await create("Support encoder", two);
  await nav("datasets"); await check("second project never inherits first project's data", "document.querySelectorAll('[data-dataset-id]').length === 0 && document.getElementById('page').textContent.includes('Add your first dataset')");
  await click('[data-project-id="' + firstId + '"]'); await loaded();
  await check("switching back restores only this project's imports", "document.querySelectorAll('[data-dataset-id]').length === 1 && document.getElementById('page').textContent.includes('Routing training source')");
  await check("switching projects restores the previous dataset page", "document.querySelector('#nav-datasets').getAttribute('aria-current') === 'page'");
  harness.chooseFolder(root); await click("#open-project"); await until("document.querySelector('#operation-error strong')?.textContent === 'This folder is not a Gym project'");
  await check("Open rejects an arbitrary folder and keeps the collection", "document.querySelectorAll('[data-project-id]').length === 2");
  await evaluate("new Promise(resolve=>setTimeout(resolve,6100))");
  await check("Open failure remains available after the toast lifetime", "document.querySelector('#operation-error') && !document.querySelector('#operation-error details').open && document.querySelectorAll('[data-dataset-id]').length === 1");
  await screenshot("managed-open-error"); await textButton("Dismiss");
  const original = join(root, "Routing encoder"), moved = join(root, "routing-moved"); renameSync(original, moved);
  await click("#reload-evidence"); await loaded();
  await check("missing managed project hides its old baseline", "document.querySelector('.project-recovery').textContent.includes('Cannot read') && !document.querySelector('.baseline-name')");
  await nav("project");
  await check("unavailable managed settings never pretend to be legacy journals", "document.getElementById('page').textContent.includes('Managed project') && !document.getElementById('page').textContent.includes('Read-only experiment evidence')");
  await nav("datasets");
  harness.chooseFolder(join(root, "Support encoder")); await textButton("Locate folder"); await until("document.querySelector('#operation-error')?.textContent.includes('different project identity')");
  if (harness.registry.get(firstId).source.kind !== "folder") throw new Error("Managed source lost");
  harness.chooseFolder(moved); await textButton("Locate folder"); await until("document.querySelectorAll('[data-dataset-id]').length === 1"); await loaded();
  await nav("project");
  // Hold an actual verified response, not fabricated evidence, to exercise a slow operation.
  const read = harness.backend.openRegistered.bind(harness.backend);
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let delivered!: () => void;
  const delivery = new Promise<void>(resolve => { delivered = resolve; });
  harness.backend.openRegistered = async (id, verify) => { const result = await read(id, verify); if (verify) { await held; delivered(); } return result; };
  try {
    await textButton("Verify project files"); await until("document.getElementById('page').getAttribute('aria-busy') === 'true'");
    await check("verification shows progress and prevents duplicate content actions", "document.querySelector('.workspace-progress').textContent.includes('Verifying') && document.querySelector('#page .page-content').inert && document.getElementById('reload-evidence').disabled");
    await screenshot("managed-verifying");
    await click('[data-project-id="' + secondId + '"]'); await loaded();
    await click('[data-project-id="' + firstId + '"]'); await loaded();
    await check("project switching restores its settings page", "document.querySelector('#nav-project').getAttribute('aria-current') === 'page'");
    release(); await delivery; await evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
    await check("late verification never overwrites a newer read of the same project", "document.getElementById('page').textContent.includes('File sizes checked') && !document.getElementById('page').textContent.includes('Checksums verified')");
  } finally { release(); harness.backend.openRegistered = read; }
  await textButton("Verify project files"); await until("document.getElementById('page').textContent.includes('Checksums verified')");
  await screenshot("managed-settings");
  await check("reconnect and verification preserve the original identity", "document.querySelector('[data-project-id=" + JSON.stringify(firstId) + "]').textContent.includes('Routing encoder')");
  const longName = "Encoder-" + "long-project-identity".repeat(5);
  await textButton("Rename project"); await type("project-name-input", longName); await textButton("Save name"); await until("!document.querySelector('#project-dialog[open]')");
  for (const width of [760, 390]) {
    await screenshot("managed-long-name-" + width, width, 700);
    await check("long names and workspace paths remain readable at " + width, "[...document.querySelectorAll('.project-info h2')].some(heading=>heading.textContent.length > 100) && document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth");
  }
  window.setContentSize(1440, 960);
  await textButton("Rename project"); await type("project-name-input", "Routing encoder"); await textButton("Save name"); await until("!document.querySelector('#project-dialog[open]')");
  await textButton("Remove project entry…"); await textButton("Remove entry"); await until("!document.querySelector('#project-dialog[open]') && document.querySelectorAll('[data-project-id]').length === 1"); await loaded();
  if (!existsSync(join(moved, "encoder-gym.json"))) throw new Error("Forget deleted a workspace");
  harness.chooseFolder(moved); await click("#open-project"); await until("document.querySelectorAll('[data-project-id]').length === 2"); await loaded();
  if (harness.registry.read().selectedId !== firstId || !harness.registry.get(secondId)) throw new Error("Reopen changed identities");
  if (!before.equals(readFileSync(join(one, "model.safetensors")))) throw new Error("Onboarding changed source model bytes");
  await check("reopened project still has no invented run evidence", "document.querySelectorAll('[data-candidate-id]').length === 0");
  console.log("Managed-workspace Electron acceptance passed.");
}
