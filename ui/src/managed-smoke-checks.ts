import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import type { BrowserWindow } from "electron";
import type { ProjectRegistry } from "./project-registry.js";

interface Harness { registry: ProjectRegistry; chooseFolder(path?: string): void; restart: boolean }
export async function runManagedSmokeChecks(window: BrowserWindow, output: string, harness: Harness): Promise<void> {
  mkdirSync(output, { recursive: true });
  const web = window.webContents;
  const evaluate = (code: string) => web.executeJavaScript(code, true);
  const until = async (expression: string) => evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(" + expression + ")resolve(true);else if(n++>1000)reject(new Error('Timed out: '+" + JSON.stringify(expression) + "));else setTimeout(poll,20)};poll()})");
  const check = async (name: string, expression: string) => { if (!await evaluate(expression)) throw new Error("Managed renderer check failed: " + name); console.log("PASS " + name); };
  const click = async (selector: string) => evaluate("document.querySelector(" + JSON.stringify(selector) + ").click()");
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
  const create = async (name: string, source: string) => {
    await click("#add-project"); await type("new-project-name", name); await type("new-project-task", "Local encoder fixture");
    harness.chooseFolder(source); await click("#choose-local-model"); await until("document.getElementById('model-preview').textContent.includes('files') && !document.querySelector('.onboarding-fields').disabled");
    harness.chooseFolder(root); await click("#choose-project-parent"); await until("!document.getElementById('confirm-new-project').disabled && !document.querySelector('.onboarding-fields').disabled");
    if (name === "Routing encoder") {
      await screenshot("managed-create-preview"); await screenshot("managed-create-760", 760, 800);
      await check("creation confirmation stays visible in the small desktop dialog", "document.getElementById('confirm-new-project').getBoundingClientRect().bottom <= document.getElementById('project-dialog').getBoundingClientRect().bottom");
      window.setContentSize(1440, 960);
    }
    await click("#confirm-new-project"); await until("!document.querySelector('#project-dialog[open]')"); await loaded();
    return harness.registry.read().selectedId!;
  };
  const firstId = await create("Routing encoder", one);
  await check("new project shows its baseline without fabricated runs or candidates", "document.querySelector('.baseline-name').textContent.includes('Routing encoder') && document.querySelectorAll('[data-candidate-id]').length === 0 && document.getElementById('source-state').textContent.includes('Managed workspace')");
  await screenshot("managed-baseline");
  await nav("datasets"); await textButton("Import dataset");
  const sealed = join(root, "held-out.jsonl"); writeFileSync(sealed, '{"evaluation_partition":"sealed"}\n');
  await evaluate("document.getElementById('dataset-purpose').value='training';document.getElementById('dataset-purpose').dispatchEvent(new Event('change',{bubbles:true}))");
  harness.chooseFolder(sealed); await click("#choose-dataset-file"); await until("document.querySelector('dialog .form-error').textContent.includes('non-training partition')");
  await check("training import rejects held-out rows visibly before copying", "document.getElementById('confirm-dataset-import').disabled && document.querySelectorAll('[data-dataset-id]').length === 0");
  const sourceData = join(root, "train.jsonl"); writeFileSync(sourceData, '{"text":"source only one","split":"train"}\n{"text":"source only two","split":"train"}\n');
  harness.chooseFolder(sourceData); await click("#choose-dataset-file"); await until("!document.getElementById('confirm-dataset-import').disabled && !document.querySelector('.onboarding-fields').disabled");
  await type("import-dataset-name", "Routing training source"); await screenshot("managed-import-preview");
  await click("#confirm-dataset-import"); await until("!document.querySelector('#project-dialog[open]') && document.querySelectorAll('[data-dataset-id]').length === 1");
  await check("dataset view exposes provenance and counts, not native payloads", "document.querySelector('.dataset-summary').textContent.includes('2 records') && !document.getElementById('page').textContent.includes('source only one')");
  await screenshot("managed-datasets");
  await click("#theme-toggle"); await screenshot("managed-datasets-light"); await click("#theme-toggle");
  for (const width of [760, 390]) {
    await screenshot("managed-datasets-" + width, width, 820);
    await check("managed dataset page fits width " + width, "document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth");
  }
  window.setContentSize(1440, 960);
  const secondId = await create("Support encoder", two);
  await nav("datasets"); await check("second project never inherits first project's data", "document.querySelectorAll('[data-dataset-id]').length === 0 && document.getElementById('page').textContent.includes('Add your first dataset')");
  await click('[data-project-id="' + firstId + '"]'); await loaded(); await nav("datasets");
  await check("switching back restores only this project's imports", "document.querySelectorAll('[data-dataset-id]').length === 1 && document.getElementById('page').textContent.includes('Routing training source')");
  harness.chooseFolder(root); await click("#open-project"); await until("document.getElementById('toast').textContent.includes('not an Encoder Gym workspace')");
  await check("Open rejects an arbitrary folder and keeps the collection", "document.querySelectorAll('[data-project-id]').length === 2");
  const original = join(root, "Routing encoder"), moved = join(root, "routing-moved"); renameSync(original, moved);
  await click("#reload-evidence"); await loaded();
  await check("missing managed project hides its old baseline", "document.querySelector('.empty-state').textContent.includes('Cannot read') && !document.querySelector('.baseline-name')");
  harness.chooseFolder(join(root, "Support encoder")); await textButton("Locate folder"); await until("document.getElementById('toast').textContent.includes('different project identity')");
  if (harness.registry.get(firstId).source.kind !== "folder") throw new Error("Managed source lost");
  harness.chooseFolder(moved); await textButton("Locate folder"); await until("document.querySelectorAll('[data-dataset-id]').length === 1"); await loaded();
  await nav("project"); await textButton("Verify project files"); await until("document.getElementById('page').textContent.includes('Checksums verified')");
  await screenshot("managed-settings");
  await check("reconnect and verification preserve the original identity", "document.querySelector('[data-project-id=" + JSON.stringify(firstId) + "]').textContent.includes('Routing encoder')");
  await textButton("Remove project entry…"); await textButton("Remove entry"); await until("!document.querySelector('#project-dialog[open]') && document.querySelectorAll('[data-project-id]').length === 1");
  if (!existsSync(join(moved, "encoder-gym.json"))) throw new Error("Forget deleted a workspace");
  harness.chooseFolder(moved); await click("#open-project"); await until("document.querySelectorAll('[data-project-id]').length === 2"); await loaded();
  if (harness.registry.read().selectedId !== firstId || !harness.registry.get(secondId)) throw new Error("Reopen changed identities");
  if (!before.equals(readFileSync(join(one, "model.safetensors")))) throw new Error("Onboarding changed source model bytes");
  await check("reopened project still has no invented run evidence", "document.querySelectorAll('[data-candidate-id]').length === 0");
  console.log("Managed-workspace Electron acceptance passed.");
}
