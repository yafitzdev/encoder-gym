import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, renameSync, readFileSync, existsSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { ManagedBackend, managedSnapshot, redactBackendError } from "../dist/evidence/managed-backend.js";
import { ProjectRegistry } from "../dist/evidence/project-registry.js";
import { writeLocalModel } from "./fixtures/local-model.mjs";
import { experimentFixture, writeExperimentDatabase } from "./fixtures/experiment.mjs";

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

test("managed snapshots project only their bound scientific experiment store", () => {
  const f = fixture(), project = join(f.root, "bound-project"), runs = join(project, "runs");
  mkdirSync(runs, { recursive: true });
  const scientific = join(runs, "scientific.sqlite");
  writeExperimentDatabase(scientific, experimentFixture("managed"));
  writeFileSync(join(project, "foreign.db"), "not a database");
  const id = randomUUID(), revisionId = randomUUID(), modelId = randomUUID(), baseline = { source: project, format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "a".repeat(64), execution: "not-configured" };
  const managed = {
    folder: project, verified: true, manifest: { version: 1, id, name: "Managed evidence", createdAt: new Date().toISOString(), task: "retrieval", baseline }, datasets: [],
    modelCatalog: { projectId: id, artifacts: [{ id: modelId, projectId: id, name: "Managed baseline", createdAt: new Date().toISOString(), origin: "imported", path: "models/baseline", format: baseline.format, bytes: baseline.bytes, fingerprint: baseline.fingerprint }], baselineRevisions: [{ id: revisionId, projectId: id, sequence: 1, modelArtifactId: modelId, change: { kind: "initialization", source_fingerprint: baseline.fingerprint }, actor: "test", reason: "test", createdAt: new Date().toISOString(), fingerprint: "revision" }], activeBaselineRevisionId: revisionId },
    scientificBinding: { id: randomUUID(), projectId: id, baselineRevisionId: revisionId, adapter: { key: "fixture", protocol: "v1", configurationFingerprint: "adapter" }, runtime: { kind: "external-isolated", location: "runtime", projectSnapshot: { id: "managed-project", fingerprint: "sha256:" + "3".repeat(64) } }, store: { databasePath: "runs/scientific.sqlite", schema: { id: "schema", fingerprint: "schema" } }, actor: "test", reason: "test", createdAt: new Date().toISOString(), specificationFingerprint: "spec", fingerprint: "binding" },
  };
  const snapshot = managedSnapshot(managed);
  assert.equal(snapshot.baseline.key, "Managed baseline");
  assert.equal(snapshot.runs.length, 1);
  assert.equal(snapshot.runs[0].id, "managed-run");
  assert.deepEqual(snapshot.databases, ["project.sqlite", "scientific.sqlite"]);
});

test("managed snapshots reject scientific store paths outside the project", () => {
  const f = fixture(), id = randomUUID(), baseline = { source: f.root, format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "a".repeat(64), execution: "not-configured" };
  const managed = { folder: join(f.root, "project"), verified: true, manifest: { version: 1, id, name: "Escape", createdAt: new Date().toISOString(), task: null, baseline }, datasets: [], scientificBinding: { store: { databasePath: "../outside.sqlite" } } };
  assert.throws(() => managedSnapshot(managed), /escaped its managed project workspace/);
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

test("managed control accepts only native-picked manifests and fixed project-scoped intents", async () => {
  const root = mkdtempSync(join(tmpdir(), "gym-managed-control-")), folder = join(root, "project");
  mkdirSync(folder);
  const id = randomUUID(), scientificId = randomUUID(), baseline = { source: join(folder, "baseline"), format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "a".repeat(64), execution: "not-configured" };
  const workspace = { folder, verified: true, manifest: { version: 1, id, name: "Control fixture", createdAt: new Date().toISOString(), task: "retrieval", baseline }, datasets: [], scientificBinding: { runtime: { projectSnapshot: { id: scientificId } } } };
  const registry = new ProjectRegistry(join(root, "profile", "projects.json"));
  registry.addManaged(workspace);
  const calls = [];
  const readiness = { report: { projectId: id, computedAt: new Date().toISOString(), overall: "ready", runnable: true, checks: [] }, launchPreview: { projectId: scientificId } };
  const generatedManifest = join(folder, "runs", "definitions", "optimization-fixed.toml");
  const baselineRevision = randomUUID();
  let escapePrepared = false, mismatchPrepared = false, restorePrepared = false;
  const preparedOutput = () => ({ manifestPath: escapePrepared ? join(root, "outside.toml") : generatedManifest, manifestName: "Approved repair", readiness: mismatchPrepared ? { projectId: randomUUID() } : readiness.launchPreview, authority: { proposalId: randomUUID() }, createdTrainingSnapshot: false, externalCalls: 0 });
  const executor = async (_executable, args) => {
    calls.push(args);
    if (args[3] === "open") return JSON.stringify(workspace);
    if (args[3] === "readiness") return JSON.stringify(restorePrepared ? { ...readiness, preparedOptimization: preparedOutput() } : readiness);
    if (args[3] === "prepare-optimization") return JSON.stringify({ ...preparedOutput(), createdTrainingSnapshot: true });
    if (args[3] === "optimize") return JSON.stringify({ run_id: randomUUID(), state: "planned" });
    if (args[3] === "promote") return JSON.stringify(workspace);
    throw new Error("unexpected command");
  };
  const backend = new ManagedBackend("owned-synth", registry, executor);
  await assert.rejects(() => backend.readiness(id, "forged"), /Choose the optimization definition/);
  await assert.rejects(() => backend.optimize(id, { action: "start", manifestToken: "forged" }), /Choose the optimization definition/);
  const prepared = await backend.prepareOptimization(id);
  assert.equal(prepared.name, "Approved repair");
  assert.equal(prepared.launchPreview.projectId, scientificId);
  assert.notEqual(prepared.launchPreview.projectId, id);
  assert.equal(prepared.externalCalls, 0);
  assert.equal("path" in prepared, false);
  assert.deepEqual(calls.find(args => args[3] === "prepare-optimization"), ["--output", "json", "workspace", "prepare-optimization", folder]);
  await backend.optimize(id, { action: "start", manifestToken: prepared.token });
  restorePrepared = true;
  const restarted = new ManagedBackend("owned-synth", registry, executor);
  const restored = await restarted.readiness(id);
  assert.equal(restored.preparedOptimization.name, "Approved repair");
  assert.equal("path" in restored.preparedOptimization, false);
  await restarted.optimize(id, { action: "start", manifestToken: restored.preparedOptimization.token });
  restorePrepared = false;
  escapePrepared = true;
  await assert.rejects(() => backend.prepareOptimization(id), /escaped its project workspace/);
  escapePrepared = false;
  mismatchPrepared = true;
  await assert.rejects(() => backend.prepareOptimization(id), /does not match this project/);
  mismatchPrepared = false;
  const manifest = join(root, "reviewed.toml"); writeFileSync(manifest, "fixture");
  const selected = await backend.chooseOptimizationManifest(id, manifest);
  assert.equal(selected.name, "reviewed.toml");
  assert.equal("path" in selected, false);
  await backend.optimize(id, { action: "start", manifestToken: selected.token });
  await backend.optimize(id, { action: "status", runId: id });
  const optimizationCalls = calls.filter(args => args[3] === "optimize");
  assert.deepEqual(optimizationCalls[0], ["--output", "json", "workspace", "optimize", folder, "start", "--manifest", generatedManifest]);
  assert.deepEqual(optimizationCalls[1], ["--output", "json", "workspace", "optimize", folder, "start", "--manifest", generatedManifest]);
  assert.deepEqual(optimizationCalls[2], ["--output", "json", "workspace", "optimize", folder, "start", "--manifest", manifest]);
  assert.deepEqual(optimizationCalls[3], ["--output", "json", "workspace", "optimize", folder, "status", id]);
  await backend.promoteAccepted(id, { runId: id, expectedBaselineRevisionId: baselineRevision });
  assert.deepEqual(calls.find(args => args[3] === "promote"), ["--output", "json", "workspace", "promote", folder, "--run-id", id, "--expected-baseline-revision-id", baselineRevision, "--actor", "local-operator", "--reason", "Promote sealed-accepted optimization candidate"]);
  await assert.rejects(() => backend.promoteAccepted(id, { runId: "../other", expectedBaselineRevisionId: baselineRevision }), /Invalid run identity/);
  await assert.rejects(() => backend.optimize(id, { action: "shell", runId: id }), /supported optimization action/);
  await assert.rejects(() => backend.optimize(id, { action: "status", runId: "../other" }), /Invalid run identity/);
});

test("managed control rejects a duplicate mutating operation for one project", async () => {
  const root = mkdtempSync(join(tmpdir(), "gym-managed-exclusive-")), folder = join(root, "project"); mkdirSync(folder);
  const id = randomUUID(), baseline = { source: folder, format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "b".repeat(64), execution: "not-configured" };
  const workspace = { folder, verified: true, manifest: { version: 1, id, name: "Exclusive fixture", createdAt: new Date().toISOString(), task: null, baseline }, datasets: [] };
  const registry = new ProjectRegistry(join(root, "profile", "projects.json")); registry.addManaged(workspace);
  let release; const held = new Promise(resolve => { release = resolve; });
  const executor = async (_executable, args) => {
    if (args[3] === "open") return JSON.stringify(workspace);
    if (args[3] === "readiness") return JSON.stringify({ report: { projectId: id, computedAt: new Date().toISOString(), overall: "ready", runnable: true, checks: [] } });
    if (args[3] === "optimize") { await held; return JSON.stringify({ run_id: id, state: "planned" }); }
    throw new Error("unexpected command");
  };
  const backend = new ManagedBackend("owned-synth", registry, executor);
  const selected = await backend.chooseOptimizationManifest(id, join(root, "run.toml"));
  const first = backend.optimize(id, { action: "start", manifestToken: selected.token });
  await new Promise(resolve => setImmediate(resolve));
  await assert.rejects(() => backend.optimize(id, { action: "resume", runId: id }), /already running/);
  release(); await first;
});

test("native execution receives only project-scoped provider credentials in fixed child environment names", async () => {
  const root = mkdtempSync(join(tmpdir(), "gym-managed-secrets-")), folder = join(root, "project"); mkdirSync(folder);
  const id = randomUUID(), runId = randomUUID();
  const baseline = { source: folder, format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "e".repeat(64), execution: "not-configured" };
  const provider = (role, environmentFallback) => ({
    role, kind: "openai-compatible", endpoint: "https://api.example.test/v1", model: `${role}-model`, authentication: "bearer",
    secret: { id: `${id}:${role}`, environmentFallback }, limits: { maximumRequests: 2, maximumInputTokens: 2000, maximumOutputTokens: 500, maximumCostMicrousd: 1000 },
  });
  const workspace = {
    folder, verified: true, manifest: { version: 1, id, name: "Secret fixture", createdAt: new Date().toISOString(), task: "retrieval", baseline }, datasets: [],
    providerCatalog: { id: randomUUID(), projectId: id, sequence: 1, providers: [provider("generation", "SYNTH_OPENAI_API_KEY"), provider("advisor", "SYNTH_ADVISOR_API_KEY"), provider("evaluator", "SYNTH_EVALUATOR_API_KEY")], actor: "test", reason: "test", createdAt: new Date().toISOString(), fingerprint: "sha256:" + "f".repeat(64) },
  };
  const registry = new ProjectRegistry(join(root, "profile", "projects.json")); registry.addManaged(workspace);
  const calls = [], resolutions = [];
  const secrets = { generation: "generation-secret-123", advisor: "advisor-secret-456", evaluator: "evaluator-secret-789" };
  const executor = async (_executable, args, environment) => {
    calls.push({ args, environment });
    if (args[3] === "open") return JSON.stringify(workspace);
    if (args[3] === "readiness") return JSON.stringify({ report: { projectId: id, computedAt: new Date().toISOString(), overall: "ready", runnable: true, checks: [] }, launchPreview: { projectId: id } });
    if (args[3] === "optimize") return JSON.stringify({ run_id: runId, state: "planned" });
    throw new Error("unexpected command");
  };
  const backend = new ManagedBackend("owned-synth", registry, executor, { resolveCredential: (credentialId, environmentFallback) => {
    resolutions.push({ credentialId, environmentFallback });
    return secrets[credentialId.split(":").at(-1)];
  } });
  const selected = await backend.chooseOptimizationManifest(id, join(root, "run.toml"));
  await backend.optimize(id, { action: "start", manifestToken: selected.token });
  assert.equal(resolutions.length, 0, "reservation must not decrypt credentials");
  assert.equal(calls.find(call => call.args[5] === "start").environment, undefined);
  await backend.optimize(id, { action: "resume", runId });
  const execution = calls.find(call => call.args[5] === "resume");
  assert.deepEqual(execution.environment, { SYNTH_OPENAI_API_KEY: secrets.generation, SYNTH_ADVISOR_API_KEY: secrets.advisor, SYNTH_EVALUATOR_API_KEY: secrets.evaluator });
  assert.equal(JSON.stringify(execution.args).includes("secret-"), false);
  assert.deepEqual(resolutions.map(item => item.credentialId).sort(), [`${id}:advisor`, `${id}:evaluator`, `${id}:generation`]);

  workspace.providerCatalog.providers[0].secret = { id: `${randomUUID()}:generation`, environmentFallback: "SYNTH_OPENAI_API_KEY" };
  await assert.rejects(() => backend.optimize(id, { action: "resume", runId }), /not safe for desktop execution/);
});

test("backend diagnostics redact bearer, key, and token shaped credentials", () => {
  const diagnostic = "Bearer secret-value-123 key-live-value-123 token-debug-value-456 sk-project-value-789";
  const redacted = redactBackendError(diagnostic);
  assert.equal(redacted.includes("secret-value-123"), false);
  assert.equal(redacted.includes("live-value-123"), false);
  assert.equal(redacted.includes("debug-value-456"), false);
  assert.equal(redacted.includes("project-value-789"), false);
});

test("provider configuration serializes only validated non-secret settings to a temporary file", async () => {
  const root = mkdtempSync(join(tmpdir(), "gym-managed-providers-")), folder = join(root, "project"); mkdirSync(folder);
  const id = randomUUID(), baseline = { source: folder, format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "c".repeat(64), execution: "not-configured" };
  const workspace = { folder, verified: true, manifest: { version: 1, id, name: "Provider fixture", createdAt: new Date().toISOString(), task: null, baseline }, datasets: [] };
  const registry = new ProjectRegistry(join(root, "profile", "projects.json")); registry.addManaged(workspace);
  let temporary, serialized;
  const emptyStatus = { projectId: id, configured: false, catalog: null, credentialAvailability: [], liveProbePerformed: false };
  const executor = async (_executable, args) => {
    if (args[3] === "open") return JSON.stringify(workspace);
    if (args[3] === "providers" && args[5] === "show") return JSON.stringify(emptyStatus);
    if (args[3] === "providers" && args[5] === "configure") {
      temporary = args[args.indexOf("--file") + 1]; serialized = readFileSync(temporary, "utf8"); return JSON.stringify(emptyStatus);
    }
    throw new Error("unexpected command");
  };
  const backend = new ManagedBackend("owned-synth", registry, executor);
  const limits = { maximumRequests: 5, maximumInputTokens: 5000, maximumOutputTokens: 1000, maximumCostMicrousd: 25000 };
  const generation = { kind: "openai-compatible", endpoint: "https://api.example.test/v1", model: "generation-model", authentication: "bearer", environmentFallback: "SYNTH_OPENAI_API_KEY", limits };
  await assert.rejects(() => backend.configureProviders(id, { version: 1, generation: { ...generation, secret: "must-not-be-written" }, advisor: generation }), /unsupported setting/);
  await assert.rejects(() => backend.configureProviders(id, { version: 1, generation: { ...generation, environmentFallback: "PATH" }, advisor: { ...generation, environmentFallback: "SYNTH_ADVISOR_API_KEY" } }), /Invalid generation provider/);
  await backend.configureProviders(id, { version: 1, generation, advisor: { ...generation, model: "advisor-model", environmentFallback: "SYNTH_ADVISOR_API_KEY" }, actor: "operator", reason: "separate authorities" });
  const parsed = JSON.parse(serialized);
  assert.equal(JSON.stringify(parsed).includes("secret"), false);
  assert.equal(parsed.generation.environment_fallback, "SYNTH_OPENAI_API_KEY");
  assert.equal(parsed.advisor.environment_fallback, "SYNTH_ADVISOR_API_KEY");
  assert.equal(existsSync(temporary), false);
});

test("scientific binding uses only native-picked runtime tokens and requires a successful preview", async () => {
  const root = mkdtempSync(join(tmpdir(), "gym-managed-binding-")), folder = join(root, "project"); mkdirSync(folder);
  const id = randomUUID(), modelId = randomUUID(), revisionId = randomUUID();
  const baseline = { source: folder, format: "safetensors-encoder", architecture: "bert", files: [], bytes: 10, fingerprint: "sha256:" + "c".repeat(64), execution: "not-configured" };
  const workspace = {
    folder, verified: true,
    manifest: { version: 1, id, name: "Binding fixture", createdAt: new Date().toISOString(), task: null, baseline }, datasets: [],
    modelCatalog: { projectId: id, artifacts: [{ id: modelId, projectId: id, name: "baseline", createdAt: new Date().toISOString(), origin: "imported", path: "models/baseline", format: baseline.format, bytes: 10, fingerprint: baseline.fingerprint }], baselineRevisions: [], activeBaselineRevisionId: revisionId },
  };
  const registry = new ProjectRegistry(join(root, "profile", "projects.json")); registry.addManaged(workspace);
  const calls = [];
  const previewOutput = {
    projectId: id, projectName: workspace.manifest.name, baselineRevisionId: revisionId,
    activeModel: { name: "baseline", format: baseline.format, bytes: 10, fingerprint: baseline.fingerprint },
    adapter: { key: "nomos", protocol: "v1", configurationFingerprint: "sha256:" + "a".repeat(64) },
    runtimeLocation: join(root, "isolated"), sourceRevision: "abc123", sourceFingerprint: "sha256:" + "b".repeat(64),
    projectSnapshot: { id: randomUUID(), fingerprint: "sha256:" + "d".repeat(64) },
    python: { executable: join(root, "python.exe"), version: "3.12.4", compatibleVersion: true, capabilities: [], ready: true },
    store: { databasePath: "runs/scientific-<snapshot-sha256>.sqlite", action: "import_verified_history", importedHistory: { sourceName: "history.sqlite", projectSnapshot: { id: randomUUID(), fingerprint: "sha256:" + "d".repeat(64) }, inventory: { projects: 1, protocols: 1, experimentRuns: 1, benchmarkGenerations: 2, diagnoses: 1, proposals: 2, approvedDeltaSelections: 2, trainingSnapshots: 1, optimizationRuns: 1 }, verification: "current_schema_integrity_and_runtime_project_match" } }, ready: true,
  };
  let runtimeReady = false;
  const executor = async (_executable, args) => {
    calls.push(args);
    if (args[3] === "open") return JSON.stringify(workspace);
    if (args[3] === "preview-nomos-binding") return JSON.stringify({ ...previewOutput, ready: runtimeReady, python: { ...previewOutput.python, ready: runtimeReady } });
    if (args[3] === "prepare-nomos-python") { runtimeReady = true; return JSON.stringify({ networkUsed: true }); }
    if (args[3] === "bind-nomos") return JSON.stringify({ ...workspace, scientificBinding: { id: randomUUID() } });
    throw new Error("unexpected command");
  };
  const backend = new ManagedBackend("owned-synth", registry, executor);
  const runtime = await backend.chooseNomosRuntime(id, join(root, "isolated"));
  const python = await backend.chooseNomosPython(id, join(root, "python.exe"));
  const history = await backend.chooseNomosHistory(id, join(root, "history.sqlite"));
  await assert.rejects(() => backend.previewNomosBinding(id, "renderer-path", python.token), /Choose the isolated runtime/);
  await assert.rejects(() => backend.previewNomosBinding(id, runtime.token, python.token, "renderer-path"), /Choose the existing scientific history/);
  const blocked = await backend.previewNomosBinding(id, runtime.token, python.token, history.token);
  await assert.rejects(() => backend.prepareNomosPython(id, "renderer-path"), /Preview the scientific runtime/);
  await backend.prepareNomosPython(id, blocked.token);
  const installArgs = calls.find(args => args[3] === "prepare-nomos-python");
  assert.deepEqual(installArgs.slice(3), ["prepare-nomos-python", folder, "--runtime", join(root, "isolated"), "--python", join(root, "python.exe"), "--allow-network-install"]);
  const preview = await backend.previewNomosBinding(id, runtime.token, python.token, history.token);
  await assert.rejects(() => backend.bindNomos(id, "renderer-path"), /Preview the scientific runtime/);
  await backend.bindNomos(id, preview.token);
  const bindArgs = calls.find(args => args[3] === "bind-nomos");
  assert.equal(bindArgs[bindArgs.indexOf("--runtime") + 1], join(root, "isolated"));
  assert.equal(bindArgs[bindArgs.indexOf("--python") + 1], join(root, "python.exe"));
  assert.equal(bindArgs[bindArgs.indexOf("--history-database") + 1], join(root, "history.sqlite"));
  assert.equal(bindArgs.includes("renderer-path"), false);
});
