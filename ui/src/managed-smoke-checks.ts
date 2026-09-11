import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import type { BrowserWindow } from "electron";
import type { ProjectRegistry } from "./project-registry.js";
import type { ManagedBackend } from "./managed-backend.js";
import type { ManagedBaselineRestorationRequest, ManagedOptimizationRequest, ManagedPromotionRequest, ManagedReadiness, ManagedRunStatus } from "./managed-control.js";

interface Harness { registry: ProjectRegistry; backend: ManagedBackend; chooseFolder(path?: string): void; restart: boolean }
export async function runManagedSmokeChecks(window: BrowserWindow, output: string, harness: Harness): Promise<void> {
  mkdirSync(output, { recursive: true });
  const web = window.webContents;
  const evaluate = (code: string) => web.executeJavaScript(code, true);
  const until = async (expression: string) => evaluate("new Promise((resolve,reject)=>{let n=0;const poll=()=>{if(" + expression + ")resolve(true);else if(n++>1000)reject(new Error('Timed out: '+" + JSON.stringify(expression) + "));else setTimeout(poll,20)};poll()})");
  const check = async (name: string, expression: string) => { if (!await evaluate(expression)) { writeFileSync(join(output, "managed-failure.png"), (await web.capturePage()).toPNG()); throw new Error("Managed renderer check failed: " + name); } console.log("PASS " + name); };
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
    await check("managed projects and selected identity survive another Electron process", "document.querySelectorAll('[data-project-id]').length === 3 && document.getElementById('breadcrumb').textContent.includes('Offline benchmark fixture') && document.getElementById('source-state').textContent.includes('Managed workspace')");
    await nav("benchmarks"); await until("document.querySelectorAll('[data-benchmark-model]').length === 3");
    await check("benchmark versions and missing results survive restart", "document.querySelectorAll('#benchmark-version option').length === 2 && document.querySelectorAll('.benchmark-results td').length === 3 && [...document.querySelectorAll('.benchmark-results td')].filter(cell=>cell.textContent.includes('Not evaluated')).length === 2");
    await evaluate("(()=>{const selector=document.getElementById('benchmark-version');selector.value=selector.options[0].value;selector.dispatchEvent(new Event('change',{bubbles:true}))})()");
    await until("document.querySelector('.benchmark-results')?.textContent.includes('0.600000')");
    await check("historical benchmark keeps its rejected candidate after restart", "document.querySelectorAll('[data-benchmark-model]').length === 3 && document.querySelectorAll('.benchmark-score').length === 3");
    await screenshot("managed-benchmark-restarted");
    await click("#project-optimize"); await until("document.getElementById('optimization-start') && !document.querySelector('.workspace-progress')");
    await check("optimization inputs survive an independent desktop process", "document.getElementById('optimization-dataset').selectedOptions[0].textContent.includes('Variant') && document.getElementById('optimization-benchmark').selectedOptions[0].textContent === 'Benchmark v1' && !document.getElementById('optimization-start').disabled && !document.querySelector('.launch-definition')");
    await screenshot("managed-optimization-inputs-restarted");
    const routing = harness.registry.read().projects.find(project => project.name === "Routing encoder")!;
    await click('[data-project-id="' + routing.id + '"]'); await loaded();
    await nav("project"); await until("document.querySelectorAll('.provider-summary').length === 2 && !document.querySelector('.provider-state .neutral')");
    await check("provider settings and encrypted credential availability survive restart", "document.querySelectorAll('.provider-state .success').length === 2 && !document.getElementById('page').textContent.includes('smoke-secret')");
    await nav("datasets");
    await until("document.querySelectorAll('[data-dataset-id]').length === 2");
    await check("dataset variants and versions survive restart", "document.querySelector('.dataset-collection').textContent.includes('Base dataset') && document.querySelector('.dataset-collection').textContent.includes('Version 4')");
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
      await check("creation preview shows the complete destination", "document.getElementById('project-parent-path').textContent.endsWith('\\\\Routing encoder') && document.getElementById('confirm-new-project-hint').textContent === '189 B'");
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
  await check("new project shows its baseline without fabricated runs or candidates", "document.querySelector('.artifact-row-name').textContent.includes('Routing encoder') && document.querySelectorAll('[data-candidate-id]').length === 0 && document.getElementById('source-state').textContent.includes('Managed workspace')");
  await screenshot("managed-baseline");
  await check("Models is an inventory with a project-level Optimize action", "document.querySelector('.page-heading h1').textContent === 'Models' && !document.querySelector('.page-heading p') && document.querySelectorAll('.artifact-row').length === 1 && !document.getElementById('project-optimize').hidden && !document.getElementById('page').textContent.includes('Start optimization')");
  await check("Models omits decorative model and candidate count badges", "![...document.querySelectorAll('.baseline-name .tag')].some(e=>e.textContent.includes('sentence-transformers')) && document.querySelector('.candidate-section > .section-heading .tag') === null");
  await click('[data-project-id]');
  await check("selected project folder collapses independently", "document.querySelector('[data-project-id]').getAttribute('aria-expanded') === 'false' && !document.querySelector('.project-pages')");
  await click('[data-project-id]');
  await check("selected project folder expands without changing the page", "document.querySelector('[data-project-id]').getAttribute('aria-expanded') === 'true' && document.querySelector('.project-pages') && document.querySelector('.page-heading h1').textContent === 'Models'");
  const pageFrames: { left: number; top: number; width: number }[] = [];
  for (const page of ["models", "datasets", "runs", "benchmarks", "activity", "project"]) {
    await nav(page); await until("document.querySelector('.workspace-page > .page-heading + .workspace-page-body')");
    pageFrames.push(await evaluate("(()=>{const page=document.querySelector('.workspace-page').getBoundingClientRect(),heading=document.querySelector('.workspace-page > .page-heading').getBoundingClientRect();return {left:Math.round(page.left),top:Math.round(heading.top),width:Math.round(page.width)}})()"));
  }
  if (pageFrames.some(frame => JSON.stringify(frame) !== JSON.stringify(pageFrames[0]))) throw new Error("Managed collection pages do not share one frame: " + JSON.stringify(pageFrames));
  console.log("PASS managed collection pages share one frame");
  await nav("activity"); await until("document.querySelector('.activity-list')");
  await check("project activity exposes action UUIDs and complete event chains", "document.querySelector('.activity-action') && document.querySelector('.activity-action code').textContent.includes('…') && !document.getElementById('page').textContent.includes('smoke-secret')");
  await nav("models");
  await click("#project-optimize"); await until("document.querySelector('.optimization-inputs') && !document.querySelector('.workspace-progress')");
  await check("Optimize starts from model, data and evaluation, not a prepared repair", "document.querySelectorAll('.optimization-input').length === 3 && document.getElementById('optimization-start').disabled && document.querySelector('.optimization-inputs').textContent.includes('Set up dataset') && document.querySelector('.optimization-inputs').textContent.includes('Set up evaluation') && !document.querySelector('.launch-definition')");
  await check("optimization inputs expose no paths or credentials", "!document.querySelector('.optimization-inputs input') && !document.querySelector('.optimization-inputs').textContent.includes('project.sqlite')");
  await screenshot("managed-readiness");
  await nav("project"); await until("[...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Configure providers' && !b.disabled)");
  await check("scientific runtime setup is a real project-settings action", "[...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Connect scientific runtime') && !document.getElementById('page').textContent.includes('not implemented')");
  await textButton("Connect scientific runtime"); await until("document.querySelector('#project-dialog[open] .runtime-form')");
  await check("runtime setup is compact and requires verification", "document.querySelector('.runtime-form').textContent.includes('Isolated clean checkout required') && document.querySelector('.runtime-form').textContent.includes('Existing history') && [...document.querySelectorAll('.runtime-form button')].find(b=>b.textContent === 'Import history') && [...document.querySelectorAll('.runtime-form button')].find(b=>b.textContent === 'Connect runtime').disabled && document.querySelectorAll('.runtime-form p').length === 0");
  await screenshot("managed-runtime-setup", 760, 760); window.setContentSize(1440, 960);
  await textButton("Cancel"); await until("!document.querySelector('#project-dialog[open]')");
  await textButton("Configure providers"); await until("document.querySelector('#project-dialog[open]')");
  await check("provider setup exposes only URL and API key", "document.querySelectorAll('.provider-form input').length === 4 && [...document.querySelectorAll('.provider-form label')].map(e=>e.firstChild.textContent.trim()).join('|') === 'URL|API key|URL|API key' && !document.querySelector('.provider-form').textContent.includes('Model') && !document.querySelector('.provider-form').textContent.includes('Maximum')");
  await check("DeepSeek and Yan are one-click provider presets", "[...document.querySelectorAll('.provider-presets button')].filter(b=>b.textContent === 'DeepSeek').length === 2 && [...document.querySelectorAll('.provider-presets button')].filter(b=>b.textContent === 'Yan').length === 2");
  await evaluate("[...document.querySelectorAll('.provider-presets')][0].querySelector('[data-endpoint=\"https://yan.tail85512d.ts.net\"]').click();[...document.querySelectorAll('.provider-presets')][1].querySelector('[data-endpoint=\"https://yan.tail85512d.ts.net\"]').click();true");
  await type("generation-credential", "generation-smoke-secret-123"); await type("advisor-credential", "advisor-smoke-secret-456");
  await textButton("Save provider setup"); await until("!document.querySelector('#project-dialog[open]') && document.querySelectorAll('.provider-summary').length === 2");
  await check("separate provider authorities expose availability without secret values", "document.querySelectorAll('.provider-state .success').length === 2 && !document.getElementById('page').textContent.includes('generation-smoke-secret-123') && !document.getElementById('page').textContent.includes('advisor-smoke-secret-456')");
  const savedProviders = (await harness.backend.providerStatus(firstId)).catalog?.providers;
  if (savedProviders?.find(provider => provider.role === "generation")?.model !== "deepseek-v4-flash" || savedProviders.find(provider => provider.role === "advisor")?.model !== "deepseek-v4-pro") throw new Error("Provider presets did not resolve their app-managed model choices.");
  console.log("PASS provider presets persist app-managed models without exposing model inputs");
  const credentialIndex = readFileSync(join(harness.registry.file, "..", "credentials.json"), "utf8");
  if (credentialIndex.includes("generation-smoke-secret-123") || credentialIndex.includes("advisor-smoke-secret-456")) throw new Error("Credential plaintext reached the desktop profile");
  await screenshot("managed-provider-settings");
  await nav("activity"); await until("[...document.querySelectorAll('.activity-action strong')].some(e=>e.textContent === 'Providers configured')");
  await check("provider changes are inspectable by action UUID without credential values", "[...document.querySelectorAll('.activity-action strong')].filter(e=>e.textContent === 'Credential saved').length === 2 && !document.getElementById('page').textContent.includes('smoke-secret')");
  await nav("project");

  // Exercise the actual renderer/preload/main IPC journey using deterministic
  // lifecycle facts. Rust end-to-end tests own the workflow transitions; this
  // proves that the desktop exposes those facts and only their safe actions.
  const readinessMethod = harness.backend.readiness.bind(harness.backend);
  const prepareMethod = harness.backend.prepareOptimization.bind(harness.backend);
  const optimizeMethod = harness.backend.optimize.bind(harness.backend);
  const promoteMethod = harness.backend.promoteAccepted.bind(harness.backend);
  const restoreMethod = harness.backend.restoreBaseline.bind(harness.backend);
  const managed = await harness.backend.openRegistered(firstId);
  let displayedManaged = managed;
  const catalog = managed.modelCatalog;
  if (!catalog) throw new Error("Managed smoke project has no model catalog");
  const runId = randomUUID(), experimentId = randomUUID(), candidateId = randomUUID();
  const trainingSnapshotId = randomUUID(), benchmarkGenerationId = randomUUID();
  const budget = { maximum_iterations: 1, maximum_candidates: 1, maximum_training_seconds: 120, maximum_development_evaluations: 2, maximum_sealed_evaluations: 1, maximum_backend_operations: 4, maximum_external_calls: 0 };
  const preview = {
    manifestFingerprint: "sha256:" + "1".repeat(64), specificationFingerprint: "sha256:" + "2".repeat(64), projectId: firstId, projectFingerprint: "sha256:" + "3".repeat(64), trainingSnapshotId, benchmarkGenerationId,
    runName: "Repair generic retrieval without regressing tool routing", developmentSuites: ["generic_holdout", "retired_post_scaling"], sealedSuite: "nomos_sealed_acceptance",
    candidateRecipes: [{ sequence: 1, maximumTrainingSeconds: 120, parameters: { learning_rate: 0.00002, epochs: 1, hypothesis: "bounded-repair" } }], maximumEvaluationSeconds: 120,
    candidateCount: 1, budget,
  };
  const authority = { proposalId: randomUUID(), selectionId: randomUUID(), trainingSnapshotId, benchmarkGenerationId, hypotheses: ["Repair the observed retrieval regression without weakening generic holdout behavior."], candidateCount: 1, baseTrainingInputs: 2, deltaRows: 24, budget: { maximum_total_rows: 7000, maximum_rows_per_target: 24, maximum_candidates: 1, maximum_training_seconds: 120, maximum_evaluation_seconds: 120, maximum_development_evaluations: 2, maximum_external_calls: 0, maximum_sealed_uses: 1 }, validUntil: new Date(Date.now() + 3_600_000).toISOString() };
  const readiness: ManagedReadiness = { report: { projectId: firstId, baselineRevisionId: catalog.activeBaselineRevisionId, computedAt: new Date().toISOString(), overall: "action_required" as const, runnable: false, checks: [{ key: "optimization.preview", category: "optimization" as const, state: "action_required" as const, required: true, summary: "Approved repair is ready to freeze", evidence: "One reviewed bounded repair remains current.", nextAction: { key: "prepare-optimization", label: "Prepare approved run" } }] }, optimizationAuthority: authority };
  const createdAt = "2026-03-01T12:00:00.000Z";
  const reserved = { iterations: 1, candidates: 1, training_seconds: 120, development_evaluations: 2, sealed_evaluations: 1, backend_operations: 4 };
  const noneRemaining = { iterations: 0, candidates: 0, training_seconds: 0, development_evaluations: 0, sealed_evaluations: 0, backend_operations: 0 };
  const run = (state: "planned" | "campaign_active" | "completed" | "cancelled" | "failed", next: "resume" | "authorize-sealed" | "none", decision?: string, terminalReason?: string) => ({
    run_id: runId, existing: false, created_at: createdAt, last_transition_at: state === "planned" ? createdAt : state === "campaign_active" ? "2026-03-01T12:02:05.000Z" : "2026-03-01T12:02:19.000Z", state, ...(state !== "planned" && state !== "cancelled" ? { campaign_state: state === "completed" ? "completed" : state === "failed" ? "failed" : "development_selected", experiment_state: state === "completed" ? "completed" : state === "failed" ? "failed" : "development_selected" } : {}), ...(decision ? { decision, selected_candidate_id: candidateId } : {}), ...(terminalReason ? { failed_or_uncertain: terminalReason } : {}),
    completed: state === "planned" || state === "cancelled" ? [] : ["candidate_training_completed", "candidate_development_completed"], stopped_reason: next === "authorize-sealed" ? "sealed_authorization_required" : state === "completed" ? "accepted_candidate" : state === "failed" ? "failed" : state === "cancelled" ? "cancelled_by_operator" : "run_reserved", human_authorization_required: next === "authorize-sealed", last_sequence: state === "completed" ? 8 : state === "campaign_active" || state === "failed" ? 5 : state === "cancelled" ? 2 : 1, head_fingerprint: "sha256:" + "4".repeat(64), artifacts: { experiment_run_id: experimentId, training_snapshot_id: trainingSnapshotId, benchmark_generation_id: benchmarkGenerationId },
    budgets: { maximum: budget, ...(state === "planned" || state === "cancelled" ? {} : { reserved, remaining_unreserved: noneRemaining }) },
    recorded_usage: state === "planned" || state === "cancelled" ? { models_trained: 0, candidates_failed: 0, training_seconds: 0, development_evaluations: 0, sealed_evaluations: 0 } : { models_trained: 1, candidates_failed: 0, training_seconds: 110, development_evaluations: 2, sealed_evaluations: state === "completed" ? 1 : 0 },
    stage: state === "failed" ? { key: "terminal", label: "Candidate development failed", detail: "The candidate stage ended without a durable successful completion.", execution: "terminal" as const } : state === "cancelled" ? { key: "terminal", label: "Run cancelled", detail: "The operator stopped this run at a safe boundary.", execution: "terminal" as const } : next === "resume" ? { key: "development", label: "Build and evaluate the candidate", detail: "The next bounded native stage trains one candidate and records development evidence.", execution: "native" as const, development: { completed_units: 0, total_units: 2 } } : next === "authorize-sealed" ? { key: "sealed-authorization", label: "Review final acceptance", detail: "The selected candidate is eligible for one separately authorized sealed evaluation.", execution: "authorization" as const, development: { completed_units: 2, total_units: 2, active_candidate_id: candidateId } } : { key: "terminal", label: "Accepted candidate recorded", detail: "The exact checkpoint passed the final acceptance contract.", execution: "terminal" as const },
    next_command: next,
  });
  const report = {
    schema_version: 1 as const, run_id: runId, name: preview.runName, state: "completed" as const, decision: "promote_candidate",
    project: { id: firstId, revision: "smoke-revision", fingerprint: "sha256:" + "a".repeat(64), baseline_model: { id: catalog.artifacts[0]!.id, key: "routing-baseline", fingerprint: catalog.artifacts[0]!.fingerprint } },
    diagnosis: { id: randomUUID(), fingerprint: "sha256:" + "b".repeat(64), weaknesses: [{ key: "generic-retrieval" }], source_campaign_id: randomUUID(), source_experiment_run_id: randomUUID() },
    approved_repair: { proposal_id: authority.proposalId, proposal_fingerprint: "sha256:" + "c".repeat(64), targets: [{}], actions: [{}], candidate_hypotheses: authority.hypotheses, delta_selection_id: authority.selectionId, delta_selection_fingerprint: "sha256:" + "d".repeat(64) },
    training_data_change: { snapshot_id: trainingSnapshotId, snapshot_fingerprint: "sha256:" + "e".repeat(64), base_rows: 6800, delta_rows: 24, total_rows: 6824, combined_membership_fingerprint: "sha256:" + "f".repeat(64) },
    selected_candidate_id: candidateId, sealed_evidence: { used: true, candidate_exposures: 1, generation_id: benchmarkGenerationId, generation_state: "exhausted", authorization: "local-operator" },
    candidate_results: [{ candidate_id: candidateId, state: "development_completed", checkpoint: { key: "routing-repair-candidate", format: "onnx", bytes: 8192, fingerprint: "sha256:" + "1".repeat(64), training_duration_seconds: 110 }, development_suites: [{ suite: "generic_holdout", baseline_report_id: randomUUID(), baseline_metrics: { mrr: 0.81 }, candidate_report_id: randomUUID(), candidate_metrics: { mrr: 0.84 }, assessment_id: randomUUID(), verdict: "passed", primary_improvement: 0.03, failed_gates: [] }, { suite: "retired_post_scaling", baseline_report_id: randomUUID(), baseline_metrics: { mrr: 0.76 }, candidate_report_id: randomUUID(), candidate_metrics: { mrr: 0.77 }, assessment_id: randomUUID(), verdict: "passed", primary_improvement: 0.01, failed_gates: [] }] }],
    budget_and_recovery: { maximum: budget, observed_training_seconds: 110, observed_development_evaluations: 2, observed_sealed_evaluations: 1, candidate_failure_events: 0, adopted_previously_proven_run: false, optimization_event_count: 8, campaign_event_count: 7, experiment_event_count: 9 },
    known_evidence_limits: ["Results apply only to the pinned project and benchmark generation.", "Candidate sealed evidence covers only the development-selected candidate."], provenance_head: "sha256:" + "4".repeat(64),
  };
  let currentRun: ManagedRunStatus = run("planned", "resume");
  let releaseTraining!: () => void, statusFails = false;
  const trainingPending = new Promise<void>(resolve => { releaseTraining = resolve; });
  try {
    harness.backend.readiness = async projectId => { if (projectId !== firstId) return readinessMethod(projectId); await new Promise(resolve => setTimeout(resolve, 60)); return readiness; };
    harness.backend.prepareOptimization = async projectId => { if (projectId !== firstId) return prepareMethod(projectId); await new Promise(resolve => setTimeout(resolve, 60)); return { token: "managed-smoke-definition", name: "Reviewed repair", launchPreview: preview, authority, createdTrainingSnapshot: false, externalCalls: 0 }; };
    harness.backend.optimize = async (projectId, request) => {
      if (projectId !== firstId) return optimizeMethod(projectId, request);
      const intent = request as ManagedOptimizationRequest;
      if (intent.action === "start") currentRun = run("planned", "resume");
      else if (intent.action === "resume") {
        const at = new Date().toISOString();
        currentRun = { ...run("campaign_active", "resume"),
          recorded_usage: run("planned", "resume").recorded_usage,
          worker: { state: "running", started_at: at, memory_bytes: 2 * 1024 ** 3, cpu_milliseconds: 4300 },
          activity: { running: true, phase: "training", completed: 24, total: 120, startedAt: at, updatedAt: at,
            events: [{ at, phase: "loading_model" }, { at, phase: "preparing_batches" }, { at, phase: "training" }] },
          timeline: [{ at, label: "Candidate training requested" }] };
        await trainingPending;
        currentRun = { ...run("campaign_active", "authorize-sealed"), worker: { state: "idle" } };
      }
      else if (intent.action === "status" && statusFails) throw new Error("Worker status connection lost");
      else if (intent.action === "authorize-sealed") currentRun = run("completed", "none", "promote_candidate");
      else if (intent.action === "cancel") currentRun = run("cancelled", "none", undefined, intent.reason);
      else if (intent.action === "report") return report;
      return currentRun;
    };
    harness.backend.promoteAccepted = async (projectId, request) => {
      if (projectId !== firstId) return promoteMethod(projectId, request);
      const intent = request as ManagedPromotionRequest;
      if (intent.runId !== runId || intent.expectedBaselineRevisionId !== catalog.activeBaselineRevisionId) throw new Error("Promotion request lost its accepted run or comparison baseline");
      const previous = catalog.baselineRevisions.find(item => item.id === catalog.activeBaselineRevisionId)!;
      const candidate = { id: randomUUID(), projectId: firstId, name: "Accepted smoke candidate", createdAt: new Date().toISOString(), origin: "trained" as const, path: "models/candidates/smoke", format: managed.manifest.baseline.format, bytes: 10, fingerprint: "sha256:" + "5".repeat(64), parentModelId: previous.modelArtifactId, producingRun: { id: experimentId, fingerprint: "sha256:" + "6".repeat(64) }, trainingSnapshot: { id: trainingSnapshotId, fingerprint: "sha256:" + "7".repeat(64) } };
      const revisionId = randomUUID();
      displayedManaged = { ...managed, modelCatalog: { ...catalog, artifacts: [...catalog.artifacts, candidate], baselineRevisions: [...catalog.baselineRevisions, { id: revisionId, projectId: firstId, sequence: catalog.baselineRevisions.length + 1, modelArtifactId: candidate.id, previousRevisionId: catalog.activeBaselineRevisionId, change: { kind: "promotion" as const, decision_id: randomUUID(), decision_fingerprint: "sha256:" + "8".repeat(64) }, actor: "local-operator", reason: "Smoke accepted promotion", createdAt: new Date().toISOString(), fingerprint: "sha256:" + "9".repeat(64) }], activeBaselineRevisionId: revisionId } };
      return displayedManaged;
    };
    harness.backend.restoreBaseline = async (projectId, request) => {
      if (projectId !== firstId) return restoreMethod(projectId, request);
      const intent = request as ManagedBaselineRestorationRequest, current = displayedManaged.modelCatalog;
      if (!current || intent.expectedBaselineRevisionId !== current.activeBaselineRevisionId) throw new Error("Restoration request lost its active baseline");
      const target = current.baselineRevisions.find(revision => revision.id === intent.targetRevisionId);
      if (!target) throw new Error("Restoration target is not in baseline history");
      const revisionId = randomUUID();
      displayedManaged = { ...displayedManaged, modelCatalog: { ...current, baselineRevisions: [...current.baselineRevisions, { id: revisionId, projectId: firstId, sequence: current.baselineRevisions.length + 1, modelArtifactId: target.modelArtifactId, previousRevisionId: current.activeBaselineRevisionId, change: { kind: "restoration" as const, target_revision_id: target.id }, actor: "local-operator", reason: "Smoke baseline restoration", createdAt: new Date().toISOString(), fingerprint: "sha256:" + "a".repeat(64) }], activeBaselineRevisionId: revisionId } };
      return displayedManaged;
    };

    // Existing CLI-created runs remain supervised through Runs. Optimize no
    // longer launches a pre-reviewed recipe unrelated to the selected inputs.
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "planned" } };
    await nav("datasets"); await until("document.querySelector('.empty-state h2')?.textContent === 'No datasets'");
    await check("Data contains dataset objects, not optimization authority or decorative counts", "document.querySelector('.page-heading h1').textContent === 'Data' && !document.querySelector('.page-heading p') && !document.querySelector('.scientific-data-card') && !document.querySelector('.page-heading .tag') && !document.getElementById('page').textContent.includes('repair rows')");
    await screenshot("managed-data-with-training-snapshot");
    await nav("benchmarks"); await until("document.querySelector('.empty-state h2')?.textContent === 'No benchmark'");
    await check("Evaluation does not substitute a prepared run for a project benchmark", "document.querySelector('.page-heading h1').textContent === 'Evaluation' && !document.querySelector('.page-heading p') && !document.querySelector('.evaluation-plan') && !document.querySelector('.benchmark-section')");
    await screenshot("managed-evaluation-empty");
    await nav("runs"); await until("document.querySelector('.optimization-run-row')"); await textButton("Continue run");
    await until("document.querySelector('.run-control')?.textContent.includes('Build and evaluate the candidate')");
    await check("reservation exposes only the next durable stage", "document.querySelector('.launch-summary h2').textContent === 'Ready to continue' && [...document.querySelectorAll('.run-control button')].some(b=>b.textContent === 'Build and evaluate the candidate') && document.querySelector('#nav-runs .nav-count').textContent === '1'");
    await textButton("Build and evaluate the candidate"); await until("document.querySelector('.live-counter progress')?.value === 24");
    await check("running page shows real counters and activity without contradictory states or duplicate actions", "document.querySelector('.launch-summary h2').textContent === 'Running' && document.querySelector('.live-run h3').textContent === 'Training candidate' && document.querySelector('.live-counter progress').max === 120 && document.querySelectorAll('.run-activity li').length >= 3 && !document.querySelector('.run-actions') && !document.querySelector('.launch-definition') && !document.querySelector('.run-usage').checkVisibility()");
    await evaluate("window.__elapsed=document.querySelector('[data-elapsed-start]').textContent;document.querySelector('.run-control > details').open=true");
    await until("document.querySelector('[data-elapsed-start]')?.textContent !== window.__elapsed");
    await until("document.querySelector('.run-freshness')?.textContent === 'Updated just now'");
    await check("polling retains expanded details", "document.querySelector('.run-control > details').open");
    await evaluate("document.querySelector('.run-control > details').open=false");
    await screenshot("managed-optimization-live");
    await screenshot("managed-optimization-live-narrow", 760, 800); window.setContentSize(1440, 960);
    statusFails = true;
    await until("document.querySelector('.run-connection-warning')?.textContent.includes('Worker status connection lost')");
    await check("failed status polling stays visible without claiming the worker stopped", "document.querySelector('.launch-summary h2').textContent === 'Running' && document.querySelector('.run-connection-warning')");
    statusFails = false;
    await nav("models");
    currentRun.activity = { ...currentRun.activity!, phase: "evaluating_retrieval", completed: undefined, total: undefined };
    await nav("runs"); await textButton("Continue run");
    await until("document.querySelector('.live-run h3')?.textContent === 'Evaluating retrieval'");
    await check("returning to an executing run restores observation without stale training percentages", "!document.querySelector('.live-counter') && document.querySelector('.launch-summary h2').textContent === 'Running' && !document.querySelector('.run-connection-warning')");
    releaseTraining(); await until("[...document.querySelectorAll('.run-control button')].some(button=>button.textContent === 'Authorize one sealed evaluation' && !button.disabled)");
    await check("run supervision distinguishes completed records from reserved capacity", "document.querySelector('.run-usage').textContent.includes('Recorded work') && document.querySelector('.run-usage').textContent.includes('110s / 2m') && document.querySelector('.run-usage').textContent.includes('2 / 2') && document.querySelector('.run-control').textContent.includes('Journal span')");
    await check("sealed evidence requires its own visible authorization", "document.querySelector('.run-control').textContent.includes('Review final acceptance') && [...document.querySelectorAll('.run-control button')].some(b=>b.textContent === 'Authorize one sealed evaluation')");
    await screenshot("managed-optimization-awaiting-approval");
    await evaluate("window.__sealedPrompt='';window.confirm=(message)=>{window.__sealedPrompt=message;return true};true");
    await textButton("Authorize one sealed evaluation"); await until("document.querySelector('.promotion-result')?.textContent.includes('passed final acceptance')");
    await check("sealed authorization explains scope before recording consent", "window.__sealedPrompt.includes('exactly one sealed evaluation') && window.__sealedPrompt.includes('cannot be reused')");
    await check("accepted result remains pending operator promotion", "document.querySelector('.promotion-result').textContent.includes('Candidate passed final acceptance') && [...document.querySelectorAll('.promotion-result button')].some(b=>b.textContent === 'Promote accepted candidate')");
    await textButton("Inspect complete run report"); await until("document.querySelector('.run-report')");
    await check("terminal report exposes aggregate evidence and provenance without sealed values", "document.querySelector('.run-report').textContent.includes('6,824 rows') && document.querySelector('.run-report').textContent.includes('generic_holdout') && document.querySelector('.run-report').textContent.includes('0.81 baseline') && document.querySelector('.run-report').textContent.includes('Known evidence limits') && !document.querySelector('.run-report').textContent.includes('sealed score')");
    await screenshot("managed-optimization-report");
    await textButton("Promote accepted candidate"); await until("document.querySelector('.promotion-result')?.textContent.includes('Accepted candidate is now the baseline')");
    await check("promotion immediately updates the active managed baseline", "document.querySelector('.launch-summary h2').textContent === 'Baseline updated' && document.querySelector('.promotion-result').textContent.includes('Accepted candidate is now the baseline') && document.getElementById('toast').textContent.includes('Baseline updated')");
    await screenshot("managed-optimization-promoted");

    currentRun = run("failed", "none", undefined, "Native trainer exited before the checkpoint was committed.");
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "failed" } };
    await textButton("Refresh"); await until("document.querySelector('.run-stage.is-failed')");
    await check("failed run leads with its durable diagnostic and no unsafe continuation", "document.querySelector('.launch-summary h2').textContent === 'Run needs attention' && document.querySelector('.run-stage.is-failed').textContent.includes('Native trainer exited') && ![...document.querySelectorAll('.run-actions button')].some(b=>b.textContent.includes('Build')) && document.querySelector('.usage-caution')");
    await screenshot("managed-optimization-failed");

    currentRun = run("cancelled", "none", undefined, "Stop before committing more local compute.");
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "cancelled" } };
    await textButton("Refresh"); await until("document.querySelector('.run-stage.is-cancelled')");
    await check("cancelled run presents the operator reason as a neutral terminal record", "document.querySelector('.launch-summary h2').textContent === 'Run cancelled' && document.querySelector('.run-stage.is-cancelled').textContent.includes('Stop before committing') && !document.querySelector('.run-stage.is-cancelled').matches('[role=alert]') && !document.querySelector('.run-actions')");
    await screenshot("managed-optimization-cancelled");

    currentRun = { ...run("campaign_active", "resume"), worker: { state: "interrupted" }, recorded_usage: run("planned", "resume").recorded_usage,
      timeline: [{ at: new Date().toISOString(), label: "Candidate training requested" }] };
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "campaign_active" } };
    await textButton("Refresh"); await until("document.querySelector('.launch-summary h2')?.textContent === 'Run stopped'");
    await check("a dead worker cannot masquerade as running or inherit a stale completion report", "!document.querySelector('.live-run') && !document.querySelector('.run-report') && document.querySelector('.run-stage').textContent.includes('worker stopped') && document.querySelector('.run-activity').textContent.includes('Candidate training requested')");
    await screenshot("managed-optimization-interrupted");

    currentRun = run("completed", "none", "promote_candidate");
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "completed" } };
    await textButton("Refresh"); await until("document.querySelector('.launch-summary h2')?.textContent === 'Baseline updated'");
    await nav("models"); await until("document.querySelectorAll('.artifact-row').length === 2");
    await evaluate("[...document.querySelectorAll('.artifact-row')].find(row=>row.textContent.includes('Previous baseline')).querySelector('.candidate-link').click()");
    await until("[...document.querySelectorAll('#page button')].some(button=>button.textContent === 'Restore baseline')");
    await check("historical model exposes restoration while the active baseline has no redundant action", "document.querySelector('.model-view .tag').textContent === 'Previous baseline' && [...document.querySelectorAll('#page button')].some(button=>button.textContent === 'Restore baseline') && ![...document.querySelectorAll('#page button')].some(button=>button.textContent === 'Make baseline')");
    await textButton("Restore baseline"); await until("document.querySelector('.model-view .tag')?.textContent === 'Baseline'");
    await check("restoring from the model viewer updates the shared baseline snapshot", "![...document.querySelectorAll('#page button')].some(button=>button.textContent === 'Restore baseline') && document.getElementById('toast').textContent.includes('restored')");
    await screenshot("managed-model-baseline-restored");
  } finally {
    releaseTraining();
    harness.backend.readiness = readinessMethod;
    harness.backend.prepareOptimization = prepareMethod;
    harness.backend.optimize = optimizeMethod;
    harness.backend.promoteAccepted = promoteMethod;
    harness.backend.restoreBaseline = restoreMethod;
  }

  await nav("models");
  await nav("runs");
  await check("managed runs keep their optimization parent visible", "document.querySelector('.optimization-run-row').textContent.includes('Latest optimization') && document.querySelector('.optimization-run-row').textContent.includes('promote candidate') && document.querySelector('#nav-runs .nav-count').textContent === '1' && [...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Inspect run')");
  await screenshot("managed-runs-parent");
  await nav("benchmarks");
  await until("document.querySelector('.empty-state h2')?.textContent === 'No benchmark'");
  await check("managed Evaluation stays empty without invented evidence or narration", "document.querySelector('.empty-state h2').textContent === 'No benchmark' && !document.querySelector('.empty-state p')");
  await nav("datasets"); await until("document.querySelector('.empty-state h2')?.textContent === 'No datasets'");
  const sealed = join(root, "held-out.jsonl"); writeFileSync(sealed, '{"evaluation_partition":"sealed"}\n');
  harness.chooseFolder(sealed); await textButton("Import dataset"); await until("document.querySelector('.operation-failure')?.textContent.includes('non-training partition')");
  await check("training import rejects held-out rows visibly before copying", "document.querySelectorAll('[data-dataset-id]').length === 0 && !document.querySelector('.dataset-progress')");
  await check("import error leads with recovery and keeps diagnostics collapsed", "document.querySelector('.operation-failure strong').textContent === 'This file contains held-out data' && !document.querySelector('.operation-failure details').open");
  await screenshot("managed-import-rejection", 760, 800); window.setContentSize(1440, 960);
  await screenshot("managed-import-error-760x560", 760, 560);
  await check("minimum desktop import error leaves recovery actions reachable", "document.querySelector('.operation-failure button').getBoundingClientRect().bottom < innerHeight");
  window.setContentSize(1440, 960);
  const sourceData = join(root, "train.jsonl"); writeFileSync(sourceData, '{"text":"source only one","split":"train"}\n{"text":"source only two","split":"train"}\n');
  const sourceBytes = readFileSync(sourceData);
  const chooseData = harness.backend.chooseDataset.bind(harness.backend);
  harness.backend.chooseDataset = async (...args) => { const choice = await chooseData(...args); writeFileSync(sourceData, sourceBytes.toString() + '{"text":"changed after preview","split":"train"}\n'); return choice; };
  try {
    harness.chooseFolder(sourceData); await textButton("Import dataset"); await until("document.querySelector('.operation-failure')?.textContent.includes('Dataset changed after preview')");
    await check("changed input cannot create a dataset from stale preview evidence", "document.querySelectorAll('[data-dataset-id]').length === 0 && !document.querySelector('.dataset-progress')");
  } finally { harness.backend.chooseDataset = chooseData; }
  writeFileSync(sourceData, sourceBytes);
  const importData = harness.backend.importDataset.bind(harness.backend);
  let releaseImport!: () => void, importCalls = 0;
  const pendingImport = new Promise<void>(resolve => { releaseImport = resolve; });
  let enteredImport!: () => void;
  const importStarted = new Promise<void>(resolve => { enteredImport = resolve; });
  harness.backend.importDataset = async (...args) => { importCalls++; enteredImport(); await pendingImport; return importData(...args); };
  try {
    harness.chooseFolder(sourceData); await textButton("Import dataset"); await until("document.querySelector('.dataset-progress')");
    await importStarted;
    await screenshot("managed-import-busy-760x560", 760, 560);
    await key("Escape");
    await textButton("Import dataset");
    await check("slow import keeps progress visible and cannot be resubmitted", "document.querySelector('.dataset-progress').textContent.includes('Saving dataset') && document.querySelector('.dataset-progress').getBoundingClientRect().bottom < innerHeight && [...document.querySelectorAll('#page button')].filter(b=>['Import dataset','Create dataset'].includes(b.textContent)).every(b=>b.disabled)");
    if (importCalls !== 1) throw new Error('Duplicate import reached backend');
    releaseImport(); await until("document.querySelectorAll('.dataset-row-entry').length === 2 && !document.querySelector('.dataset-progress')");
  } finally { releaseImport(); harness.backend.importDataset = importData; }
  window.setContentSize(1440, 960);
  await check("dataset viewer shows real rows and shares a four-tab object layout", "document.querySelector('.dataset-view h1').textContent === 'Base dataset' && document.querySelectorAll('.tabs button').length === 4 && document.querySelector('.dataset-rows').textContent.includes('source only one')");
  await evaluate("document.querySelector('.dataset-row-entry button').click()");
  await check("row inspection reveals exact native data only on request", "document.querySelector('dialog[open] .native-row').textContent.includes('source only one') && !document.querySelector('dialog[open] details').open");
  await textButton("Close");
  await textButton("Create variant");
  await type("dataset-name", "   "); await click("#dataset-confirm");
  await until("document.querySelector('dialog[open] .form-error').textContent.includes('Enter a dataset name')");
  await check("invalid dataset name stays editable without creating a variant", "!document.getElementById('dataset-name').disabled && !document.getElementById('dataset-confirm').disabled");
  await type("dataset-name", "Candidate dataset"); await click("#dataset-confirm");
  await until("document.querySelector('.dataset-view h1')?.textContent === 'Candidate dataset' && document.querySelectorAll('.dataset-row-entry').length === 2 && !document.querySelector('dialog[open]')");
  const firstRowId = await evaluate("document.querySelector('.dataset-row-entry').dataset.rowId");
  const mutateDataset = harness.backend.datasetVersions.mutate.bind(harness.backend.datasetVersions), savedRequests: unknown[] = [];
  harness.backend.datasetVersions.mutate = async (id, request) => {
    savedRequests.push(structuredClone(request));
    const result = await mutateDataset(id, request);
    if (savedRequests.length === 1) throw new Error("Dataset response lost after commit");
    return result;
  };
  try {
    await evaluate("[...document.querySelectorAll('.dataset-row-entry:first-child button')].find(b=>b.textContent==='Remove').click()"); await click("#dataset-confirm");
    await until("document.querySelector('dialog[open] .form-error').textContent.includes('Dataset response lost')");
    await click("#dataset-confirm");
    await until("document.querySelector('[data-change-kind=removed]') && !document.querySelector('dialog[open]')");
    if (savedRequests.length !== 2 || JSON.stringify(savedRequests[0]) !== JSON.stringify(savedRequests[1])) throw new Error("Dataset retry changed its reserved identity");
    await check("retry after a committed save preserves one version and dismisses the error", "document.querySelectorAll('#dataset-version option').length === 2 && !document.querySelector('.operation-failure')");
  } finally { harness.backend.datasetVersions.mutate = mutateDataset; }
  await check("removal saves a new version with inspectable before evidence", "document.querySelector('#dataset-version option:checked').textContent === 'Version 2' && document.querySelector('[data-change-kind=removed]').textContent.includes('source only one')");
  await click("#tab-rows"); await until("document.querySelectorAll('.dataset-row-entry').length === 1");
  if (await evaluate("document.querySelector('.dataset-row-entry').dataset.rowId") === firstRowId) throw new Error("Removed row survived in variant");
  const replacement = join(root, "replacement.jsonl"); writeFileSync(replacement, '{"text":"replacement row","split":"train"}\n');
  harness.chooseFolder(replacement); await textButton("Replace"); await until("document.querySelector('[data-change-kind=replaced]')");
  await check("replacement displays before and after without overwriting prior history", "document.querySelector('[data-change-kind=replaced]').textContent.includes('source only two') && document.querySelector('[data-change-kind=replaced]').textContent.includes('replacement row')");
  await screenshot("managed-dataset-changes");
  for (const width of [760, 390]) {
    await screenshot("managed-dataset-changes-" + width, width, 820);
    await check("dataset changes fit width " + width, "document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1");
  }
  window.setContentSize(1440, 960);
  await click("#tab-rows"); await until("document.querySelectorAll('.dataset-row-entry').length === 1");
  const extra = join(root, "extra.jsonl"); writeFileSync(extra, Array.from({length:26},(_,i)=>JSON.stringify({text:'added '+i,split:'train'})+'\n').join(''));
  harness.chooseFolder(extra); await textButton("Add rows"); await until("document.querySelectorAll('[data-change-kind=added]').length === 25");
  await click("#tab-rows"); await until("document.querySelectorAll('.dataset-row-entry').length === 25");
  await screenshot("managed-dataset-rows");
  await textButton("Next"); await until("document.querySelectorAll('.dataset-row-entry').length === 2");
  await check("row pagination is bounded and counts persisted membership", "document.querySelector('.dataset-pagination').textContent.includes('26–27 of 27')");
  await click("#navigate-back"); await until("document.querySelectorAll('.dataset-row-entry').length === 25");
  await check("back restores the previous row page", "document.querySelector('.dataset-pagination').textContent.includes('1–25 of 27')");
  await click("#tab-versions");
  await check("all four immutable versions remain inspectable", "document.querySelectorAll('.dataset-versions .artifact-row').length === 4");
  await screenshot("managed-dataset-versions");
  await evaluate("[...document.querySelectorAll('.dataset-versions .artifact-row')].at(-1).querySelector('button').click()"); await until("document.querySelectorAll('.dataset-row-entry').length === 2");
  await check("old version preserves original content and cannot be edited in place", "document.querySelector('.dataset-rows').textContent.includes('source only one') && ![...document.querySelectorAll('#page button')].some(b=>b.textContent==='Remove')");
  await textButton("All datasets");
  await check("collection lists datasets rather than source files or snapshot badges", "document.querySelectorAll('[data-dataset-id]').length === 2 && document.querySelector('.dataset-collection').textContent.includes('Base dataset') && document.querySelector('.dataset-collection').textContent.includes('Candidate dataset') && !document.querySelector('.dataset-collection').textContent.includes('train.jsonl') && !document.querySelector('.scientific-data-card')");
  await screenshot("managed-datasets");
  await click("#theme-toggle"); await screenshot("managed-datasets-light"); await click("#theme-toggle");
  for (const width of [760, 390]) {
    await screenshot("managed-datasets-" + width, width, 820);
    await check("managed dataset page fits width " + width, "document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth");
  }
  window.setContentSize(1440, 960);
  const secondId = await create("Support encoder", two);
  await nav("datasets"); await until("document.querySelector('.empty-state h2')?.textContent === 'No datasets'"); await check("second project never inherits first project's data", "document.querySelectorAll('[data-dataset-id]').length === 0");
  await click('[data-project-id="' + firstId + '"]'); await loaded();
  await until("document.querySelectorAll('[data-dataset-id]').length === 2");
  await check("switching back restores only this project's datasets", "document.getElementById('page').textContent.includes('Candidate dataset')");
  await check("switching projects restores the previous dataset page", "document.querySelector('#nav-datasets').getAttribute('aria-current') === 'page'");
  harness.chooseFolder(root); await click("#open-project"); await until("document.querySelector('#operation-error strong')?.textContent === 'This folder is not a Gym project'");
  await check("Open rejects an arbitrary folder and keeps the collection", "document.querySelectorAll('[data-project-id]').length === 2");
  await evaluate("new Promise(resolve=>setTimeout(resolve,6100))");
  await check("Open failure remains available after the toast lifetime", "document.querySelector('#operation-error') && !document.querySelector('#operation-error details').open && document.querySelectorAll('[data-dataset-id]').length === 2");
  await screenshot("managed-open-error"); await textButton("Dismiss");
  const original = join(root, "Routing encoder"), moved = join(root, "routing-moved"); renameSync(original, moved);
  await click("#reload-evidence"); await loaded();
  await check("missing managed project hides its old baseline", "document.querySelector('.project-recovery').textContent.includes('Cannot read') && !document.querySelector('.baseline-name')");
  await nav("project");
  await check("unavailable managed settings never pretend to be legacy journals", "document.getElementById('page').textContent.includes('Managed project') && !document.getElementById('page').textContent.includes('Read-only experiment evidence')");
  await nav("datasets");
  harness.chooseFolder(join(root, "Support encoder")); await textButton("Locate folder"); await until("document.querySelector('#operation-error')?.textContent.includes('different project identity')");
  if (harness.registry.get(firstId).source.kind !== "folder") throw new Error("Managed source lost");
  harness.chooseFolder(moved); await textButton("Locate folder"); await until("document.querySelectorAll('[data-dataset-id]').length === 2"); await loaded();
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
  // Import actual recorded-input evidence through the ordinary CLI, then inspect
  // the resulting model/version relationship through renderer + IPC reads.
  const recordedModel = join(root, "model-with-data"); writeLocalModel(recordedModel, 3);
  writeFileSync(join(recordedModel, "nomos_training_manifest.json"), JSON.stringify({ inputs: ["recorded.jsonl"], input_state_counts: { "recorded.jsonl": 2 } }));
  writeFileSync(join(root, "recorded.jsonl"), ["one", "two"].map(id => JSON.stringify({ evaluation_partition: "train", accepted: true, decision_state_id: id, tool_registry: [], legal_candidate_ids: [], question: "Recorded row " + id }) + "\n").join(""));
  await create("Recorded encoder", recordedModel);
  const recordedFolder = join(root, "Recorded encoder");
  for (const args of [["backfill-nomos", recordedFolder, "--source-root", root], ["dataset", recordedFolder, "adopt-baseline"]]) execFileSync(harness.backend.executable, ["--output", "json", "workspace", ...args], { windowsHide: true, maxBuffer: 16 * 1024 * 1024 });
  await click("#reload-evidence"); await loaded();
  await evaluate("document.querySelector('.artifact-row .candidate-link').click()");
  await textButton("Inspect dataset"); await until("document.querySelectorAll('.dataset-row-entry').length === 2");
  await check("model opens its exact persisted training version", "document.querySelector('.dataset-view h1').textContent === 'Base dataset' && document.querySelector('.dataset-rows').textContent.includes('Recorded row one')");
  await click("#tab-models");
  await check("dataset model links are derived from the same recorded version", "document.querySelector('#detail-panel .artifact-row').textContent.includes('Baseline')");
  await textButton("Inspect model");
  await check("dataset returns to the shared model viewer", "document.querySelector('.model-view') && document.querySelectorAll('.tabs button').length === 4");
  await screenshot("managed-model-dataset-link");
  await nav("project"); await textButton("Remove project entry…"); await textButton("Remove entry");
  await until("!document.querySelector('dialog[open]') && document.querySelectorAll('[data-project-id]').length === 2"); await loaded();
  if (harness.registry.read().selectedId !== firstId) { await click('[data-project-id="' + firstId + '"]'); await loaded(); }
  // Seed only isolated synthetic records; all benchmark interactions below use
  // the actual renderer, IPC, production CLI, and project/scientific stores.
  const fixtureExecutable = join(harness.backend.executable, "..", process.platform === "win32" ? "synth-benchmark-fixture.exe" : "synth-benchmark-fixture");
  const benchmarkFixture = JSON.parse(execFileSync(fixtureExecutable, [join(root, "benchmark-fixture")], { encoding: "utf8", windowsHide: true, maxBuffer: 16 * 1024 * 1024 })) as { folder: string; firstRun: string; changedRun: string };
  harness.chooseFolder(benchmarkFixture.folder); await click("#open-project"); await until("document.querySelectorAll('[data-project-id]').length === 3"); await loaded();
  await check("managed Models remains an inventory without duplicate comparison controls", "document.querySelectorAll('[data-model-id]').length === 3 && !document.querySelector('.artifact-list input[type=checkbox]') && !document.getElementById('compare-selected')");
  const scientificBefore = readFileSync(join(benchmarkFixture.folder, "runs/scientific.sqlite"));
  await nav("benchmarks"); await until("document.querySelector('.empty-state h2')?.textContent === 'No benchmark'");
  const chooseBenchmark = async (runId: string) => {
    await textButton("Choose recorded benchmark"); await until("document.querySelector('#benchmark-source-run')");
    await evaluate("document.getElementById('benchmark-source-run').value=" + JSON.stringify(runId) + ";document.getElementById('benchmark-source-run').dispatchEvent(new Event('change',{bubbles:true}))");
    await until("document.getElementById('benchmark-confirm') && !document.getElementById('benchmark-confirm').disabled");
    await click("#benchmark-confirm"); await until("document.querySelectorAll('[data-benchmark-model]').length === 3 && !document.querySelector('.dataset-progress')");
  };
  await chooseBenchmark(benchmarkFixture.firstRun);
  await check("one benchmark table shows baseline, rejected candidate and not-evaluated model", "document.querySelectorAll('.benchmark-results').length === 1 && document.querySelectorAll('[data-benchmark-model]').length === 3 && document.querySelector('.benchmark-results').textContent.includes('0.600000') && document.querySelector('.benchmark-results').textContent.includes('Not evaluated') && !document.querySelector('.benchmark-section') && !document.querySelector('.page-heading p')");
  await check("distinct baseline evaluations are shown without selecting the best score", "document.querySelectorAll('.benchmark-score').length === 3 && document.querySelector('.benchmark-results').textContent.includes('0.700000') && document.querySelector('.benchmark-results').textContent.includes('0.720000')");
  await screenshot("managed-benchmark-results");
  await evaluate("[...document.querySelectorAll('.benchmark-score')].find(button=>button.textContent === '0.600000').click()");
  await until("document.querySelector('.benchmark-report-dialog')");
  await check("score inspection preserves original rejection and hides sealed evidence", "document.querySelector('.benchmark-report-dialog').textContent.includes('Failed development checks') && !document.querySelector('.benchmark-report-dialog').textContent.includes('99999.125') && !document.querySelector('.benchmark-report-dialog').textContent.includes('NEVER_PROJECT_NATIVE_METADATA')");
  await screenshot("managed-benchmark-report"); await key("Escape");
  await evaluate("[...document.querySelectorAll('.benchmark-model-link')].find(button=>button.textContent === 'Candidate 01').click()");
  await check("comparison opens the shared model viewer", "document.querySelector('.model-view h1').textContent === 'Candidate 01' && document.querySelectorAll('.tabs button').length === 4");
  await click("#navigate-back"); await until("document.querySelectorAll('[data-benchmark-model]').length === 3");
  await click("#tab-protocol");
  await check("scoring rules and protected-test identity stay in Protocol", "document.querySelector('.benchmark-protocol').textContent.includes('Final holdout') && document.querySelector('.benchmark-rules').textContent.includes('Maximum regression') && !document.getElementById('page').textContent.includes('99999.125')");
  await screenshot("managed-benchmark-protocol");
  await click("#tab-versions"); await chooseBenchmark(benchmarkFixture.changedRun);
  await check("changing the benchmark creates a second version without borrowing candidate scores", "document.querySelectorAll('#benchmark-version option').length === 2 && !document.querySelector('.benchmark-results').textContent.includes('0.600000') && [...document.querySelectorAll('.benchmark-results td')].filter(cell=>cell.textContent.includes('Not evaluated')).length === 2");
  await click("#navigate-back"); await check("Back returns to the earlier benchmark's history tab", "document.getElementById('tab-versions').getAttribute('aria-selected') === 'true'");
  await click("#navigate-forward"); await until("document.querySelector('.benchmark-results')");
  for (const width of [760, 390]) {
    await screenshot("managed-benchmark-results-" + width, width, 800);
    await check("benchmark table scrolls locally without overflowing the page at " + width, "document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth && document.querySelector('.benchmark-table-scroll').clientWidth > 100");
  }
  window.setContentSize(1440, 960);
  // Actual CLI custody fixtures; all selection, save/retry and inspection below
  // runs through the renderer and production IPC/CLI, with no optimizer stub.
  const setupCommand = (args: string[]) => JSON.parse(execFileSync(harness.backend.executable, ["--output", "json", "workspace", ...args], { encoding: "utf8", windowsHide: true, maxBuffer: 16 * 1024 * 1024 }));
  const setupSource = join(root, "optimization-training.jsonl");
  writeFileSync(setupSource, JSON.stringify({ text: "OPTIMIZATION_TRAINING_ROW_CANARY", label: "training" }) + "\n");
  const setupProjectId = harness.registry.read().selectedId!;
  const importPreview = await harness.backend.chooseDataset(setupProjectId, setupSource, "training");
  const imported = await harness.backend.importDataset(setupProjectId, importPreview.token, "Starting data");
  const sourceId = imported.datasets.at(-1)!.id;
  const baseId = randomUUID(), baseVersion = randomUUID(), variantId = randomUUID(), variantVersion = randomUUID();
  setupCommand(["dataset", benchmarkFixture.folder, "create", "--name", "Base dataset", "--dataset-id", baseId, "--version-id", baseVersion, "--source", sourceId]);
  setupCommand(["dataset", benchmarkFixture.folder, "fork", baseVersion, "--name", "Variant", "--dataset-id", variantId, "--version-id", variantVersion]);
  await click("#reload-evidence"); await loaded(); await click("#project-optimize");
  await until("document.getElementById('optimization-dataset') && document.getElementById('optimization-benchmark') && !document.querySelector('.workspace-progress')");
  await check("multiple starting datasets require a choice instead of guessing", "document.getElementById('optimization-dataset').value === '' && document.getElementById('optimization-start').disabled");
  const selectInput = async (id: string, value: string) => evaluate("(()=>{const input=document.getElementById(" + JSON.stringify(id) + ");input.value=" + JSON.stringify(value) + ";input.dispatchEvent(new Event('change',{bubbles:true}))})()");
  await selectInput("optimization-dataset", baseVersion);
  await check("Optimize contains only named input versions, not row payload or old recipes", "document.querySelector('.optimization-inputs').textContent.includes('Baseline') && document.querySelectorAll('.optimization-input').length === 3 && !document.getElementById('page').textContent.includes('OPTIMIZATION_TRAINING_ROW_CANARY') && !document.querySelector('.launch-definition') && !document.getElementById('optimization-start').disabled");
  await textButton("Inspect model"); await until("document.querySelector('.model-view')"); await click("#navigate-back");
  await textButton("Inspect dataset"); await until("document.querySelectorAll('.dataset-row-entry').length === 1"); await click("#navigate-back");
  await textButton("Inspect benchmark"); await until("document.querySelector('.benchmark-protocol')"); await click("#navigate-back");
  await check("artifact inspection returns to the same unsaved selections", "document.getElementById('optimization-dataset').value === " + JSON.stringify(baseVersion));
  const setupCatalog = imported.modelCatalog!, baseline = setupCatalog.baselineRevisions.find(value => value.id === setupCatalog.activeBaselineRevisionId)!;
  const saveInputs = harness.backend.optimizationSetup.save.bind(harness.backend.optimizationSetup);
  const inputRequests: unknown[] = []; let loseInputReply = true;
  const basePreview = await harness.backend.optimizationSetup.preview(setupProjectId, { modelId: baseline.modelArtifactId, datasetVersionId: baseVersion, benchmarkVersionId: await evaluate("document.getElementById('optimization-benchmark').value") as string });
  const baseRequest = { id: randomUUID(), expectedParent: basePreview.expectedParent, inputs: basePreview.inputs };
  try {
    harness.backend.optimizationSetup.save = async (id, request) => {
      inputRequests.push(structuredClone(request)); const result = await saveInputs(id, request);
      if (loseInputReply) { loseInputReply = false; throw new Error("Input-save response lost after commit"); }
      return result;
    };
    await harness.backend.optimizationSetup.save(setupProjectId, baseRequest).catch(() => undefined);
    await harness.backend.optimizationSetup.save(setupProjectId, baseRequest);
    if (inputRequests.length !== 2 || JSON.stringify(inputRequests[0]) !== JSON.stringify(inputRequests[1])) throw new Error("Input-save retry changed the reviewed request");
  } finally { harness.backend.optimizationSetup.save = saveInputs; }
  let setupHistory = await harness.backend.optimizationSetup.list(setupProjectId);
  if (setupHistory.length !== 1 || setupHistory[0]!.inputs.dataset.id !== baseVersion) throw new Error("Input-save retry duplicated or substituted setup");
  await selectInput("optimization-dataset", variantVersion);
  const oldBenchmark = await evaluate("document.getElementById('optimization-benchmark').options[1].value") as string;
  await selectInput("optimization-benchmark", oldBenchmark);
  const variantPreview = await harness.backend.optimizationSetup.preview(setupProjectId, { modelId: baseline.modelArtifactId, datasetVersionId: variantVersion, benchmarkVersionId: oldBenchmark });
  await harness.backend.optimizationSetup.save(setupProjectId, { id: randomUUID(), expectedParent: variantPreview.expectedParent, inputs: variantPreview.inputs });
  setupHistory = await harness.backend.optimizationSetup.list(setupProjectId);
  if (setupHistory.length !== 2 || setupHistory[1]!.inputs.dataset.id !== variantVersion || setupHistory[1]!.inputs.benchmark.id !== oldBenchmark || setupHistory[1]!.parent?.id !== setupHistory[0]!.id) throw new Error("Changed inputs did not preserve setup history");
  await click("#reload-evidence"); await loaded(); await until("document.getElementById('optimization-dataset')?.value === " + JSON.stringify(variantVersion));
  await check("saved selection remains one actionable Optimize screen", "document.getElementById('optimization-start').textContent === 'Optimize' && !document.getElementById('optimization-start').disabled && !document.getElementById('page').textContent.includes('No run started') && !document.getElementById('page').textContent.includes('Automatic execution')");
  await screenshot("managed-optimization-inputs");
  for (const width of [760, 390]) {
    await screenshot("managed-optimization-inputs-" + width, width, 800);
    await check("input selection fits the page at " + width, "document.getElementById('page').scrollWidth <= document.getElementById('page').clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth");
  }
  window.setContentSize(1440, 960);
  const fakeLimits = { maximumRequests: 10, maximumInputTokens: 10_000, maximumOutputTokens: 2_000, maximumCostMicrousd: 0 };
  await harness.backend.configureProviders(setupProjectId, {
    version: 1,
    generation: { kind: "fake", model: "fixture-generation", authentication: "none", limits: fakeLimits },
    advisor: { kind: "fake", model: "fixture-advisor", authentication: "none", limits: fakeLimits },
  });
  const queued = await harness.backend.optimizationLaunch.start(setupProjectId, setupHistory[1]!.id);
  await nav("runs"); await until("document.querySelector('[data-project-run-id=" + JSON.stringify(queued.run.id) + "]')");
  await check("project optimization survives as a resumable top-level run", "document.querySelector('[data-project-run-id=" + JSON.stringify(queued.run.id) + "]').textContent.includes('Checking inputs') && document.querySelector('[data-project-run-id=" + JSON.stringify(queued.run.id) + "] button').textContent === 'Continue'");
  await check("Runs does not expose repair recipes as the product workflow", "!document.querySelector('.project-run-section').textContent.toLowerCase().includes('repair')");
  if (!scientificBefore.equals(readFileSync(join(benchmarkFixture.folder, "runs/scientific.sqlite")))) throw new Error("Benchmark inspection/adoption changed scientific evidence");
  console.log("Managed-workspace Electron acceptance passed.");
}
