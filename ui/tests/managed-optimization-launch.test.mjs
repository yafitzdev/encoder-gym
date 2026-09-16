import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { access, readFile } from "node:fs/promises";
import { dirname } from "node:path";
import { ManagedOptimizationLaunch } from "../dist/evidence/managed-optimization-launch.js";
import { fingerprint, setupFixture } from "./optimization-setup-fixture.mjs";

function fixture() {
  const base = setupFixture(), providerId = randomUUID(), setupId = randomUUID(), launchId = randomUUID();
  base.workspace.providerCatalog = { id: providerId, projectId: base.projectId, sequence: 1, providers: [], actor: "operator", reason: "fixture", createdAt: new Date().toISOString(), fingerprint };
  const provider = maximumRequests => ({ maximumRequests, maximumInputTokens: 10_000, maximumOutputTokens: 2_000, maximumCostMicrousd: 0 });
  const scope = { projectId: base.projectId, setup: { id: setupId, fingerprint }, providerCatalog: { id: providerId, fingerprint },
    limits: { maximumIterations: 3, maximumModels: 3, maximumDatasetRowChanges: 5_000, maximumTrainingSeconds: 21_600, maximumDevelopmentEvaluations: 6, maximumFinalEvaluations: 1 },
    generation: provider(11), advisor: provider(7), finalEvaluation: "selected_candidate_once", fingerprint };
  const preview = { scope, modelName: "Baseline", datasetRows: 10, benchmarkNumber: 1 };
  const authorization = id => ({ id, scope, authorizedBy: "local-operator", createdAt: new Date().toISOString(), fingerprint });
  const run = (state = "queued", extra = {}) => ({
    run: { id: randomUUID(), projectId: base.projectId, launch: { id: launchId, fingerprint }, setup: { id: setupId, fingerprint }, createdAt: new Date().toISOString(), fingerprint },
    state, attempt: 0, materializationAttempt: 0, experimentAttempt: 0, executionAttempt: 0, finalAttempt: 0,
    lastSequence: 1, headFingerprint: fingerprint, updatedAt: new Date().toISOString(), ...extra,
  });
  return { ...base, setupId, launchId, scope, preview, authorization, run };
}
function ports(f, command, exclusive = async (_id, work) => work()) {
  return {
    open: async id => { assert.equal(id, f.projectId); return f.workspace; }, command, exclusive,
    exclusiveRun: async (_id, _runId, work) => work(new AbortController().signal), abortRun: () => {},
  };
}
function executionPorts(f, command) {
  return ports(f, async (...args) => args[0][0] === "optimization-launch" && args[0][2] === "list" ? [f.authorization(f.launchId)] : command(...args));
}
function agentAuthorization(f) {
  const value = f.authorization(f.launchId);
  value.scope.agentic = {
    mode: "quick_test", objective: "Inspect development failures.", maximumIterations: 1, maximumAgentTurnsPerIteration: 4,
    generationConcurrency: 1, maximumRowChanges: 8,
    training: { device: "auto", maximumEpochs: 1, batchSize: 8, learningRateNanos: 3_000, maximumSecondsPerIteration: 120, maximumTrainingRows: 64 },
  };
  value.scope.limits = { maximumIterations: 1, maximumModels: 1, maximumDatasetRowChanges: 8, maximumTrainingSeconds: 120, maximumDevelopmentEvaluations: 2, maximumFinalEvaluations: 0 };
  value.scope.finalEvaluation = "development_only";
  return value;
}
function agentRun(wire, state, attemptId = randomUUID(), head = fingerprint) {
  const executionState = state.replace("agent_", "");
  const completion = state === "agent_completed" ? { id: randomUUID(), fingerprint } : null;
  return { ...wire, state, agentExecution: { state: executionState, attemptId, attempts: 1, completion, lastSequence: 2, headFingerprint: head, updatedAt: wire.updatedAt } };
}

test("optimization launch preview and history are exact project-owned reads", async () => {
  const f = fixture(), calls = [], authorization = f.authorization(randomUUID());
  const backend = new ManagedOptimizationLaunch(ports(f, async args => { calls.push(args); return args[2] === "list" ? [authorization] : f.preview; }));
  assert.deepEqual(await backend.list(f.projectId), [authorization]);
  assert.deepEqual(await backend.preview(f.projectId, f.setupId), f.preview);
  assert.deepEqual(calls, [
    ["optimization-launch", "owned-project", "list"],
    ["optimization-launch", "owned-project", "preview", "--setup", f.setupId],
  ]);
  await assert.rejects(() => backend.preview(f.projectId, "../other"));
  const changed = structuredClone(f.preview); changed.scope.providerCatalog.id = randomUUID();
  await assert.rejects(() => new ManagedOptimizationLaunch(ports(f, async () => changed)).preview(f.projectId, f.setupId));
});

test("agent settings history preserves strict quick-test limits and legacy launch shapes", async () => {
  const f = fixture(), legacy = f.authorization(randomUUID()), quick = structuredClone(legacy);
  quick.id = randomUUID();
  quick.scope.agentic = {
    mode: "quick_test", objective: "Inspect development gaps.\nKeep the benchmark unchanged.",
    maximumIterations: 1, maximumAgentTurnsPerIteration: 4, generationConcurrency: 1, maximumRowChanges: 8,
    training: { device: "auto", maximumEpochs: 1, batchSize: 8, learningRateNanos: 3_000, maximumSecondsPerIteration: 120, maximumTrainingRows: 64 },
  };
  quick.scope.limits = { maximumIterations: 1, maximumModels: 1, maximumDatasetRowChanges: 8, maximumTrainingSeconds: 120, maximumDevelopmentEvaluations: 2, maximumFinalEvaluations: 0 };
  quick.scope.finalEvaluation = "development_only";
  const backend = new ManagedOptimizationLaunch(ports(f, async () => [legacy, quick]));
  assert.deepEqual(await backend.list(f.projectId), [legacy, quick]);
  for (const alter of [
    value => value.scope.agentic.generationConcurrency = 0,
    value => value.scope.agentic.generationConcurrency = 17,
    value => value.scope.agentic.maximumIterations = 2,
    value => value.scope.agentic.training.maximumTrainingRows = null,
    value => value.scope.agentic.training.maximumSecondsPerIteration = 7_200,
    value => value.scope.agentic.training.apiKey = "forbidden",
    value => value.scope.agentic.objective = "x".repeat(4_001),
    value => value.scope.limits.maximumIterations = 3,
    value => value.scope.limits.maximumFinalEvaluations = 1,
    value => value.scope.finalEvaluation = "selected_candidate_once",
  ]) {
    const invalid = structuredClone(quick); alter(invalid);
    await assert.rejects(() => new ManagedOptimizationLaunch(ports(f, async () => [invalid])).list(f.projectId));
  }
});

test("one-click authorization preserves its retry, removes temp files and rejects injected execution data", async () => {
  const f = fixture(), id = randomUUID(), request = { id, scope: f.scope }, writes = [], files = [];
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    assert.deepEqual(args.slice(0, 4), ["optimization-launch", "owned-project", "authorize", "--file"]);
    files.push(args[4]); const written = JSON.parse(await readFile(args[4], "utf8")); writes.push(written);
    return { actionId: randomUUID(), authorization: f.authorization(written.id) };
  }, async (projectId, work) => { assert.equal(projectId, f.projectId); return work(); }));
  const first = await backend.authorize(f.projectId, request);
  const second = await backend.authorize(f.projectId, request);
  assert.equal(first.authorization.id, id);
  assert.deepEqual(second.authorization.scope, first.authorization.scope);
  assert.deepEqual(writes, [request, request]);
  for (const file of files) { await assert.rejects(() => access(file)); await assert.rejects(() => access(dirname(file))); }
  for (const alter of [
    value => value.scope.apiKey = "secret",
    value => value.scope.execute = true,
    value => value.scope.projectId = randomUUID(),
    value => value.scope.limits.maximumModels = 99,
    value => value.scope.finalEvaluation = "unlimited",
    value => value.path = "other-project",
  ]) {
    const invalid = structuredClone(request); alter(invalid);
    await assert.rejects(() => backend.authorize(f.projectId, invalid));
  }
  assert.equal(writes.length, 2);
});

test("one-click optimization reserves exact current inputs and removes its private request", async () => {
  const f = fixture(), files = [], wire = f.run();
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    if (args[0] === "optimization-launch") {
      assert.deepEqual(args, ["optimization-launch", "owned-project", "preview", "--setup", f.setupId, "--agentic"]);
      return { ...f.preview, scope: agentAuthorization(f).scope };
    }
    assert.deepEqual(args.slice(0, 4), ["optimization-run", "owned-project", "start", "--file"]);
    files.push(args[4]);
    const request = JSON.parse(await readFile(args[4], "utf8"));
    assert.equal(request.scope.setup.id, f.setupId);
    assert.ok(request.scope.agentic, "Optimize must reserve Agent authority, not the fixed-recipe envelope");
    return { actionId: randomUUID(), run: { ...wire, run: { ...wire.run, launch: { id: request.id, fingerprint } } } };
  }));
  const started = await backend.start(f.projectId, f.setupId);
  assert.equal(started.run.id, wire.run.id);
  assert.equal(started.run.setupId, f.setupId);
  assert.equal(started.run.state, "queued");
  for (const file of files) { await assert.rejects(() => access(file)); await assert.rejects(() => access(dirname(file))); }
});

test("a new Optimize action fails closed if an older backend returns only legacy authority", async () => {
  const f = fixture(), calls = [];
  const backend = new ManagedOptimizationLaunch(ports(f, async args => { calls.push(args); return f.preview; }));
  await assert.rejects(() => backend.start(f.projectId, f.setupId), /Agent launch authority/);
  assert.equal(calls.length, 1);
  assert.equal(calls[0][2], "preview");
});

test("one-click optimization drives recoverable stages, forwards exact work, and withholds credentials from non-execution work", async () => {
  const f = fixture(), calls = [], phases = [], details = [], environment = { SYNTH_OPENAI_API_KEY: "secret" }, wire = f.run("candidate_rejected", {
    outcome: { run: { id: randomUUID(), fingerprint }, experimentFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "candidate_ready", selectedModel: { id: randomUUID(), fingerprint }, createdAt: new Date().toISOString(), fingerprint },
    finalResult: { run: { id: randomUUID(), fingerprint }, outcomeFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "candidate_rejected", model: { id: randomUUID(), fingerprint }, finalReport: { id: randomUUID(), fingerprint }, createdAt: new Date().toISOString(), fingerprint },
  });
  wire.finalResult.experimentRun = wire.outcome.experimentRun;
  const configured = new ManagedOptimizationLaunch({ ...executionPorts(f, async (args, env, progress) => {
    calls.push({ args, env });
    if (args[2] === "materialize") progress?.({ phase: "writing_training_rows", completed: 40, total: 100 });
    return args[2] === "show" ? wire : { actionId: randomUUID(), run: wire };
  }), environment: () => environment });
  const result = await configured.drive(f.projectId, wire.run.id, (phase, native) => { phases.push(phase); if (native) details.push(native); });
  assert.equal(result.state, "candidate_rejected");
  assert.deepEqual(calls.map(call => call.args[2]), ["show", "prepare", "materialize", "attach", "execute", "register", "show", "finalize", "show"]);
  assert.deepEqual(calls.filter(call => call.env).map(call => call.args[2]), ["execute", "finalize"]);
  assert.deepEqual(phases, ["checking_inputs", "preparing_data", "preparing_data", "starting", "training", "saving_candidate", "evaluating", "complete"]);
  assert.deepEqual(details, [{ phase: "writing_training_rows", completed: 40, total: 100 }]);
});

test("a development result that retains the baseline skips sealed evaluation", async () => {
  const f = fixture(), wire = f.run("baseline_retained", { outcome: { run: { id: randomUUID(), fingerprint }, experimentFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "baseline_retained", createdAt: new Date().toISOString(), fingerprint } }), commands = [];
  const backend = new ManagedOptimizationLaunch({ ...executionPorts(f, async args => { commands.push(args[2]); return args[2] === "show" ? wire : {}; }), environment: () => ({}) });
  assert.equal((await backend.drive(f.projectId, wire.run.id)).state, "baseline_retained");
  assert.ok(!commands.includes("finalize"));
});

test("cancellation is recorded before the active worker is aborted", async () => {
  const f = fixture(), active = f.run("optimizing"), cancelled = { ...active, state: "cancelled", failureCode: "user_requested", lastSequence: 3 };
  const order = [];
  const backend = new ManagedOptimizationLaunch({
    ...ports(f, async args => {
      assert.deepEqual(args, ["optimization-run", "owned-project", "cancel", active.run.id]);
      order.push("recorded"); return { actionId: randomUUID(), run: cancelled };
    }),
    abortRun: (_projectId, runId) => { assert.equal(runId, active.run.id); order.push("aborted"); },
  });
  assert.equal((await backend.cancel(f.projectId, active.run.id)).state, "cancelled");
  assert.deepEqual(order, ["recorded", "aborted"]);
});

test("project optimization history is a typed project-owned read", async () => {
  const f = fixture(), older = f.run("baseline_retained", { outcome: { run: { id: randomUUID(), fingerprint }, experimentFingerprint: fingerprint, experimentRun: { id: randomUUID(), fingerprint }, kind: "baseline_retained", createdAt: new Date().toISOString(), fingerprint } }), newer = f.run();
  const backend = new ManagedOptimizationLaunch(ports(f, async args => { assert.deepEqual(args, ["optimization-run", "owned-project", "list"]); return [older, newer]; }));
  const runs = await backend.runs(f.projectId);
  assert.deepEqual(runs.map(run => run.id), [older.run.id, newer.run.id]);
  assert.deepEqual(runs.map(run => run.setupId), [f.setupId, f.setupId]);
  const foreign = structuredClone(newer); foreign.run.projectId = randomUUID();
  await assert.rejects(() => new ManagedOptimizationLaunch(ports(f, async () => [foreign])).runs(f.projectId));
});

test("Stop interrupts the in-flight work without cancelling the run and Resume keeps the same UUID", async () => {
  const f = fixture(), wire = f.run("ready"), calls = [];
  let finish, entered;
  const waiting = new Promise((_resolve, reject) => { finish = () => reject(new Error("Worker interrupted")); });
  const started = new Promise(resolve => { entered = resolve; });
  let first = true;
  const backend = new ManagedOptimizationLaunch({ ...executionPorts(f, async args => {
    calls.push(args);
    if (args[2] === "prepare" && first) { first = false; entered(); await waiting; }
    if (args[2] === "register") wire.state = "baseline_retained";
    return args[2] === "show" ? wire : {};
  }), abortRun: (_project, id) => { assert.equal(id, wire.run.id); finish(); } });
  const driving = backend.drive(f.projectId, wire.run.id);
  await started;
  await backend.stop(f.projectId, wire.run.id);
  assert.ok(!calls.some(args => args[2] === "materialize"));
  assert.equal((await driving).state, "ready");
  assert.ok(!calls.some(args => args[2] === "cancel" || args[2] === "execute"));
  assert.equal((await backend.drive(f.projectId, wire.run.id)).state, "baseline_retained");
  assert.ok(calls.every(args => args[3] === wire.run.id));
});

test("attached child identity is projected before execution and foreign children are rejected", async () => {
  const f = fixture(), wire = f.run("ready_to_run"), ref = () => ({ id: randomUUID(), fingerprint });
  wire.experiment = { run: { id: wire.run.id, fingerprint }, materializationFingerprint: fingerprint,
    scientificProject: ref(), benchmark: ref(), sourceProtocol: ref(), candidate: ref(), protocol: ref(), experimentRun: ref(), createdAt: wire.updatedAt, fingerprint };
  const backend = new ManagedOptimizationLaunch(ports(f, async () => wire));
  assert.equal((await backend.show(f.projectId, wire.run.id)).experimentRunId, wire.experiment.experimentRun.id);
  wire.experiment.run = ref();
  await assert.rejects(() => backend.show(f.projectId, wire.run.id), /another run/);
});

test("Stop during worker startup is not lost before the first command", async () => {
  const f = fixture(), wire = f.run("queued"), commands = [];
  const backend = new ManagedOptimizationLaunch(executionPorts(f, async args => { commands.push(args[2]); return wire; }));
  await backend.stop(f.projectId, wire.run.id);
  const stopped = await backend.drive(f.projectId, wire.run.id);
  assert.equal(stopped.id, wire.run.id);
  assert.deepEqual(commands, ["show", "show", "show"]);
});

test("Agent execution is parsed, driven by the existing coordinator, and receives only pinned provider credentials", async () => {
  const f = fixture(), queued = f.run("queued"), completed = agentRun(queued, "agent_completed"), calls = [], phases = [];
  let done = false;
  const environment = { SYNTH_ADVISOR_API_KEY: "agent-secret", SYNTH_OPENAI_API_KEY: "generator-secret" };
  const backend = new ManagedOptimizationLaunch({ ...ports(f, async (args, env) => {
    calls.push({ args, env });
    if (args[0] === "optimization-launch") return [agentAuthorization(f)];
    if (args[2] === "drive-agent") { done = true; return { runId: queued.run.id }; }
    if (args[2] === "show") return done ? completed : queued;
    throw new Error(`Unexpected command ${args.join(" ")}`);
  }), environment: () => environment });
  const result = await backend.drive(f.projectId, queued.run.id, phase => phases.push(phase));
  assert.equal(result.state, "agent_completed");
  assert.equal(result.agentExecution.completion.id, completed.agentExecution.completion.id);
  const drive = calls.find(call => call.args[2] === "drive-agent");
  assert.deepEqual(drive.args, ["optimization-run", "owned-project", "drive-agent", queued.run.id]);
  assert.deepEqual(drive.env, environment);
  assert.deepEqual(phases, ["training", "complete"]);
  assert.ok(!calls.some(call => ["prepare", "materialize", "attach", "execute", "register", "finalize"].includes(call.args[2])));
});

test("Agent Stop is durable, does not kill the coordinator, and Resume pins the observed paused head", async () => {
  const f = fixture(), queued = f.run("queued"), attempt = randomUUID(), pausedHead = `sha256:${"b".repeat(64)}`;
  let current = queued, release, entered, aborts = 0;
  const waiting = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { entered = resolve; });
  const calls = [];
  const backend = new ManagedOptimizationLaunch({ ...ports(f, async args => {
    calls.push(args);
    if (args[0] === "optimization-launch") return [agentAuthorization(f)];
    if (args[2] === "show") return current;
    if (args[2] === "drive-agent") {
      if (args.includes("--resume")) { current = agentRun(queued, "agent_completed", randomUUID()); return { runId: queued.run.id }; }
      current = agentRun(queued, "agent_running", attempt); entered(); await waiting; return { runId: queued.run.id, state: "paused" };
    }
    if (args[2] === "stop-agent") {
      assert.equal(args[4], "--request-id"); assert.match(args[5], /^[0-9a-f-]{36}$/);
      current = agentRun(queued, "agent_paused", attempt, pausedHead); release(); return current.agentExecution;
    }
    throw new Error(`Unexpected command ${args.join(" ")}`);
  }), environment: () => ({}), abortRun: () => { aborts++; } });
  const driving = backend.drive(f.projectId, queued.run.id);
  await started;
  await backend.stop(f.projectId, queued.run.id);
  assert.equal((await driving).state, "agent_paused");
  assert.equal(aborts, 0, "durable Agent Stop must let the owner unwind and acknowledge pause");
  assert.equal((await backend.drive(f.projectId, queued.run.id, undefined, pausedHead)).state, "agent_completed");
  const resumed = calls.find(args => args[2] === "drive-agent" && args.includes("--resume"));
  assert.deepEqual(resumed.slice(-2), ["--resume", pausedHead]);
});

test("Resume never adopts a newer Stop head and startup cannot implicitly resume a queued Stop", async () => {
  const f = fixture(), wire = f.run(), oldHead = `sha256:${"c".repeat(64)}`, newHead = `sha256:${"d".repeat(64)}`, calls = [];
  const paused = agentRun(wire, "agent_paused", randomUUID(), newHead);
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    calls.push(args);
    if (args[2] === "show") return paused;
    throw new Error("Must not dispatch work for stale or missing Resume intent");
  }));
  await assert.rejects(() => backend.drive(f.projectId, wire.run.id, undefined, oldHead), /Stop state changed/);
  await assert.rejects(() => backend.drive(f.projectId, wire.run.id), /explicitly Resume/);
  await assert.rejects(() => backend.drive(f.projectId, wire.run.id, undefined, "--injected"), /fingerprint/);
  assert.ok(calls.every(args => args[2] === "show"));
});

test("reconciliation does not turn an older Continue into permission to clear a Stop", async () => {
  const f = fixture(), wire = f.run(), attempt = randomUUID(), calls = [];
  let current = agentRun(wire, "agent_stopping", attempt);
  // The execution state's wire name differs from the root projection.
  current.agentExecution.state = "stop_requested";
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    calls.push(args);
    if (args[2] === "show") return current;
    if (args[2] === "reconcile-agent") { current = agentRun(wire, "agent_paused", attempt); return {}; }
    throw new Error("Reconciliation may not start work");
  }));
  await assert.rejects(() => backend.drive(f.projectId, wire.run.id), /explicitly Resume/);
  assert.ok(!calls.some(args => args[2] === "drive-agent"));
});

test("prepared Agent runs use their immutable launch authority before any root Agent attempt exists", async () => {
  for (const state of ["preparing", "preparation_failed", "ready"]) {
    const f = fixture(), wire = f.run(state), calls = []; let done = false;
    const backend = new ManagedOptimizationLaunch(ports(f, async args => {
      calls.push(args[2]);
      if (args[0] === "optimization-launch") return [agentAuthorization(f)];
      if (args[2] === "show") return done ? agentRun(wire, "agent_completed") : wire;
      if (args[2] === "drive-agent") { done = true; return {}; }
      throw new Error("Agent authority must not enter the fixed executor");
    }));
    assert.equal((await backend.drive(f.projectId, wire.run.id)).state, "agent_completed");
    assert.deepEqual(calls, ["show", "list", "drive-agent", "show"]);
  }
});

test("failed Stop replies reuse the command identity while a confirmed later Stop is distinct", async () => {
  const f = fixture(), wire = agentRun(f.run(), "agent_paused"), requests = [];
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    if (args[2] === "show") return wire;
    if (args[2] === "stop-agent") {
      requests.push(args[5]);
      if (requests.length === 1) throw new Error("Lost reply after persisted Stop");
      return {};
    }
    throw new Error("Unexpected command");
  }));
  await assert.rejects(() => backend.stop(f.projectId, wire.run.id), /Lost reply/);
  await backend.stop(f.projectId, wire.run.id);
  await backend.stop(f.projectId, wire.run.id);
  assert.equal(requests[0], requests[1]);
  assert.notEqual(requests[1], requests[2]);
});

test("retrying an old Stop after Resume cannot hide a failure in the newer attempt", async () => {
  const f = fixture(), wire = f.run(), calls = []; let current = agentRun(wire, "agent_paused"), release, entered;
  const pending = new Promise(resolve => { release = resolve; }), started = new Promise(resolve => { entered = resolve; });
  const backend = new ManagedOptimizationLaunch(ports(f, async args => {
    if (args[2] === "show") return current;
    if (args[2] === "stop-agent") {
      calls.push(args[5]);
      if (calls.length === 1) throw new Error("Lost Stop reply");
      return current; // The old request is already recorded; this attempt stays running.
    }
    if (args[2] === "drive-agent") {
      current = agentRun(wire, "agent_running"); entered(); await pending;
      current = agentRun(wire, "agent_failed"); throw new Error("New attempt failed");
    }
    throw new Error("Unexpected command");
  }));
  await assert.rejects(() => backend.stop(f.projectId, wire.run.id), /Lost Stop reply/);
  const driving = backend.drive(f.projectId, wire.run.id, undefined, current.agentExecution.headFingerprint);
  await started;
  await backend.stop(f.projectId, wire.run.id);
  assert.equal(calls[0], calls[1]);
  release();
  await assert.rejects(() => driving, /New attempt failed/);
});
