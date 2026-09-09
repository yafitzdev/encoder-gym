import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import type { BrowserWindow } from "electron";
import type { ProjectRegistry } from "./project-registry.js";
import type { ManagedBackend } from "./managed-backend.js";
import type { ManagedOptimizationRequest, ManagedPromotionRequest, ManagedReadiness } from "./managed-control.js";

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
  await check("readiness distinguishes ready foundations from missing scientific authority", "document.querySelector('.readiness-list').textContent.includes('Connect the scientific runtime') && document.querySelector('.readiness-list > details').textContent.includes('foundations are ready') && document.querySelector('.readiness-list').textContent.includes('Prepare the reviewed run')");
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

  // Exercise the actual renderer/preload/main IPC journey using deterministic
  // lifecycle facts. Rust end-to-end tests own the workflow transitions; this
  // proves that the desktop exposes those facts and only their safe actions.
  const readinessMethod = harness.backend.readiness.bind(harness.backend);
  const prepareMethod = harness.backend.prepareOptimization.bind(harness.backend);
  const optimizeMethod = harness.backend.optimize.bind(harness.backend);
  const promoteMethod = harness.backend.promoteAccepted.bind(harness.backend);
  const managed = await harness.backend.openRegistered(firstId);
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
  let currentRun = run("planned", "resume");
  try {
    harness.backend.readiness = async projectId => { if (projectId !== firstId) return readinessMethod(projectId); await new Promise(resolve => setTimeout(resolve, 60)); return readiness; };
    harness.backend.prepareOptimization = async projectId => { if (projectId !== firstId) return prepareMethod(projectId); await new Promise(resolve => setTimeout(resolve, 60)); return { token: "managed-smoke-definition", name: "Reviewed repair", launchPreview: preview, authority, createdTrainingSnapshot: false, externalCalls: 0 }; };
    harness.backend.optimize = async (projectId, request) => {
      if (projectId !== firstId) return optimizeMethod(projectId, request);
      const intent = request as ManagedOptimizationRequest;
      if (intent.action === "start") currentRun = run("planned", "resume");
      else if (intent.action === "resume") currentRun = run("campaign_active", "authorize-sealed");
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
      return { ...managed, modelCatalog: { ...catalog, artifacts: [...catalog.artifacts, candidate], baselineRevisions: [...catalog.baselineRevisions, { id: revisionId, projectId: firstId, sequence: catalog.baselineRevisions.length + 1, modelArtifactId: candidate.id, previousRevisionId: catalog.activeBaselineRevisionId, change: { kind: "promotion" as const, decision_id: randomUUID(), decision_fingerprint: "sha256:" + "8".repeat(64) }, actor: "local-operator", reason: "Smoke accepted promotion", createdAt: new Date().toISOString(), fingerprint: "sha256:" + "9".repeat(64) }], activeBaselineRevisionId: revisionId } };
    };

    await nav("models"); await textButton("Start optimization"); await textButton("Refresh checks");
    await check("passive readiness refresh never claims to perform deep preparation", "document.querySelector('.workspace-progress').textContent.includes('Checking persisted project state') && !document.querySelector('.workspace-progress').textContent.includes('Replaying')");
    await until("document.querySelector('.launch-definition')?.textContent.includes('Approved repair on record')");
    await check("approved repair is understandable before preparation", "document.querySelector('.launch-definition').textContent.includes('one candidate') || document.querySelector('.launch-definition').textContent.includes('Candidates') && document.querySelector('.launch-definition').textContent.includes('External calls')");
    await textButton("Prepare approved run");
    await check("preparation alone describes the expensive integrity replay", "document.querySelector('.workspace-progress').textContent.includes('Replaying the approved native evidence')");
    await until("document.querySelector('.launch-definition')?.textContent.includes('Exact run definition')");
    await check("prepared definition exposes the reviewed objective and exact execution recipe", "document.querySelector('.launch-definition').textContent.includes('Repair the observed retrieval regression') && document.querySelector('.launch-definition').textContent.includes('generic_holdout') && document.querySelector('.launch-definition').textContent.includes('nomos_sealed_acceptance') && document.querySelector('.launch-definition').textContent.includes('Learning rate')");
    await check("prepared definition exposes finite execution and external limits", "document.querySelector('.launch-definition').textContent.includes('120 seconds') && document.querySelector('.launch-definition').textContent.includes('Sealed evaluations') && document.querySelector('.external-work').textContent.includes('No external provider calls') && document.querySelector('.launch-actions').textContent.includes('0 external calls')");
    await screenshot("managed-optimization-prepared");
    await textButton("Reserve optimization run"); await until("document.querySelector('.run-control')?.textContent.includes('Build and evaluate the candidate')");
    await check("reservation exposes only the next durable stage", "document.querySelector('.run-control').textContent.includes('Reserved run') && [...document.querySelectorAll('.run-control button')].some(b=>b.textContent === 'Build and evaluate the candidate') && document.querySelector('#nav-runs .nav-count').textContent === '1'");
    await textButton("Build and evaluate the candidate"); await until("document.querySelector('.run-control')?.textContent.includes('Review final acceptance')");
    await check("run supervision distinguishes completed records from reserved capacity", "document.querySelector('.run-usage').textContent.includes('Recorded work') && document.querySelector('.run-usage').textContent.includes('110s / 2m') && document.querySelector('.run-usage').textContent.includes('2 / 2') && document.querySelector('.run-control').textContent.includes('Journal span')");
    await check("sealed evidence requires its own visible authorization", "document.querySelector('.run-control').textContent.includes('one separately authorized sealed evaluation') && [...document.querySelectorAll('.run-control button')].some(b=>b.textContent === 'Authorize one sealed evaluation')");
    await screenshot("managed-optimization-running");
    await evaluate("window.__sealedPrompt='';window.confirm=(message)=>{window.__sealedPrompt=message;return true};true");
    await textButton("Authorize one sealed evaluation"); await until("document.querySelector('.promotion-result')?.textContent.includes('passed final acceptance')");
    await check("sealed authorization explains scope before recording consent", "window.__sealedPrompt.includes('exactly one sealed evaluation') && window.__sealedPrompt.includes('cannot be reused')");
    await check("accepted result remains a candidate pending operator promotion", "document.querySelector('.promotion-result').textContent.includes('remains a candidate') && [...document.querySelectorAll('.promotion-result button')].some(b=>b.textContent === 'Promote accepted candidate')");
    await textButton("Inspect complete run report"); await until("document.querySelector('.run-report')");
    await check("terminal report exposes aggregate evidence and provenance without sealed values", "document.querySelector('.run-report').textContent.includes('6,824 rows') && document.querySelector('.run-report').textContent.includes('generic_holdout') && document.querySelector('.run-report').textContent.includes('0.81 baseline') && document.querySelector('.run-report').textContent.includes('Known evidence limits') && !document.querySelector('.run-report').textContent.includes('sealed score')");
    await screenshot("managed-optimization-report");
    await textButton("Promote accepted candidate"); await until("document.querySelector('.promotion-result')?.textContent.includes('now the project baseline')");
    await check("promotion immediately updates the active managed baseline", "document.querySelector('.launch-summary h2').textContent === 'Baseline updated' && document.querySelector('.promotion-result').textContent.includes('managed custody') && document.getElementById('toast').textContent.includes('promoted')");
    await screenshot("managed-optimization-promoted");

    currentRun = run("failed", "none", undefined, "Native trainer exited before the checkpoint was committed.");
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "failed" } };
    await textButton("Refresh checks"); await until("document.querySelector('.run-stage.is-failed')");
    await check("failed run leads with its durable diagnostic and no unsafe continuation", "document.querySelector('.launch-summary h2').textContent === 'Run needs attention' && document.querySelector('.run-stage.is-failed').textContent.includes('Native trainer exited') && ![...document.querySelectorAll('.run-actions button')].some(b=>b.textContent.includes('Build')) && document.querySelector('.usage-caution')");
    await screenshot("managed-optimization-failed");

    currentRun = run("cancelled", "none", undefined, "Stop before committing more local compute.");
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "cancelled" } };
    await textButton("Refresh checks"); await until("document.querySelector('.run-stage.is-cancelled')");
    await check("cancelled run presents the operator reason as a neutral terminal record", "document.querySelector('.launch-summary h2').textContent === 'Run cancelled' && document.querySelector('.run-stage.is-cancelled').textContent.includes('Stop before committing') && !document.querySelector('.run-stage.is-cancelled').matches('[role=alert]') && !document.querySelector('.run-actions')");
    await screenshot("managed-optimization-cancelled");

    currentRun = run("completed", "none", "promote_candidate");
    readiness.launchPreview = { ...preview, existingRun: { runId, state: "completed" } };
    await textButton("Refresh checks"); await until("document.querySelector('.launch-summary h2')?.textContent === 'Baseline updated'");
  } finally {
    harness.backend.readiness = readinessMethod;
    harness.backend.prepareOptimization = prepareMethod;
    harness.backend.optimize = optimizeMethod;
    harness.backend.promoteAccepted = promoteMethod;
  }

  await nav("models");
  await nav("runs");
  await check("managed runs keep their optimization parent visible", "document.querySelector('.optimization-run-row').textContent.includes('Latest optimization') && document.querySelector('.optimization-run-row').textContent.includes('promote candidate') && document.querySelector('#nav-runs .nav-count').textContent === '1' && [...document.querySelectorAll('#page button')].some(b=>b.textContent === 'Inspect run')");
  await screenshot("managed-runs-parent");
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
