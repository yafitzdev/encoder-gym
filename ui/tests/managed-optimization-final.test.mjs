import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { access, readFile } from "node:fs/promises";
import { dirname } from "node:path";
import { ManagedOptimizationFinal } from "../dist/evidence/managed-optimization-final.js";
import { finalFixture } from "./optimization-final-fixture.mjs";

function harness(f, handler) {
  const calls = []; let controller, locked = false, aborts = 0;
  const ports = {
    open: async id => { assert.equal(id, f.projectId); return structuredClone(f.workspace); },
    command: async (args, progress, signal) => {
      assert.deepEqual(args.slice(0, 2), ["optimization-run", "owned-project"]);
      assert.equal(args[3], f.runId); calls.push(args);
      if (args[2] === "show") return structuredClone(f.run);
      return handler(args, progress, signal);
    },
    exclusive: async (id, work) => {
      assert.equal(id, f.projectId); if (locked) throw new Error("Busy"); locked = true;
      try { return await work(); } finally { locked = false; }
    },
    exclusiveRun: (id, runId, work) => ports.exclusive(id, async () => {
      assert.equal(runId, f.runId); controller = new AbortController(); return work(controller.signal);
    }),
    abortRun: (id, runId) => { assert.equal(id, f.projectId); assert.equal(runId, f.runId); aborts++; controller.abort(); },
  };
  return { backend: new ManagedOptimizationFinal(ports), ports, calls, aborts: () => aborts };
}

test("final reads and review are read-only and never infer approval", async () => {
  const f = finalFixture(), h = harness(f, async args => {
    if (args[2] === "final-agent-result") return null;
    assert.equal(args[2], "preview-final-agent"); return structuredClone(f.scope);
  });
  assert.equal(await h.backend.read(f.projectId, f.runId), null);
  const review = await h.backend.review(f.projectId, f.runId);
  assert.deepEqual(review.scope, f.scope);
  assert.deepEqual(await h.backend.review(f.projectId, f.runId), review);
  assert.deepEqual(h.calls.map(a => a[2]), ["show", "final-agent-result", "show", "preview-final-agent", "show", "preview-final-agent"]);
  await assert.rejects(() => h.backend.authorize(f.projectId, f.runId, randomUUID()), /Review/);
  await assert.rejects(() => h.backend.stop(f.projectId, f.runId), /No final-evaluation worker/);
  assert.equal(h.aborts(), 0);
});

test("final authority rejects changed review and unfinished or foreign runs before dispatch", async () => {
  const f = finalFixture(), h = harness(f, async args => { assert.equal(args[2], "preview-final-agent"); return structuredClone(f.scope); });
  const review = await h.backend.review(f.projectId, f.runId);
  f.scope.model.id = randomUUID();
  await assert.rejects(() => h.backend.authorize(f.projectId, f.runId, review.token), /changed/);
  f.run.run.id = randomUUID();
  await assert.rejects(() => h.backend.read(f.projectId, f.runId), /Finish adaptive/);
  f.run.run.id = f.runId; f.run.state = "agent_paused"; f.run.agentExecution.state = "paused"; f.run.agentExecution.completion = null;
  await assert.rejects(() => h.backend.read(f.projectId, f.runId), /Finish adaptive/);
  f.workspace.manifest.id = randomUUID();
  await assert.rejects(() => h.backend.read(f.projectId, f.runId), /another project/);
  assert.ok(h.calls.every(args => ["show", "preview-final-agent"].includes(args[2])));
});

test("explicit consent uses a private request and lost replies retry the same authority", async () => {
  const f = finalFixture(), files = [], requests = []; let loseReply = true;
  const h = harness(f, async (args, progress, signal) => {
    if (args[2] === "preview-final-agent") return structuredClone(f.scope);
    assert.ok(signal instanceof AbortSignal);
    if (args[2] === "authorize-final-agent") {
      assert.deepEqual(args.slice(4, 5), ["--file"]); assert.deepEqual(args.slice(6), ["--authorized-by", "local-operator"]);
      files.push(args[5]); const request = JSON.parse(await readFile(args[5], "utf8")); requests.push(request);
      assert.deepEqual(Object.keys(request).sort(), ["id", "scope"]); assert.deepEqual(request.scope, f.scope);
      f.authorization.id = request.id; f.dispatch.authorization.id = request.id; f.receipt.dispatch.id = request.id;
      if (loseReply) { loseReply = false; throw new Error("Lost consent reply"); }
      return structuredClone(f.authorization);
    }
    assert.equal(args[2], "finalize-agent"); assert.deepEqual(args.slice(4), ["--authorization-id", f.authorization.id]);
    assert.equal(typeof progress, "function"); return f.view();
  });
  const review = await h.backend.review(f.projectId, f.runId);
  await assert.rejects(() => h.backend.authorize(f.projectId, f.runId, review.token), /Lost consent reply/);
  assert.deepEqual(await h.backend.review(f.projectId, f.runId), review);
  assert.deepEqual(await h.backend.authorize(f.projectId, f.runId, review.token), f.view());
  assert.deepEqual(requests[0], requests[1]);
  for (const file of files) { await assert.rejects(() => access(file)); await assert.rejects(() => access(dirname(file))); }
});

test("reopened final recovery uses only saved consent and never reauthorizes", async () => {
  for (const state of ["authorized", "outcome_unknown", "completed"]) {
    const f = finalFixture(), h = harness(f, async args => {
      if (args[2] === "final-agent-result") return f.view(state);
      assert.equal(args[2], "finalize-agent"); return f.view();
    });
    await assert.rejects(() => h.backend.recover(f.projectId, f.runId, randomUUID()), /Saved final consent changed/);
    assert.deepEqual(await h.backend.recover(f.projectId, f.runId, f.authorization.id), f.view());
    assert.ok(h.calls.every(args => ["show", "final-agent-result", "finalize-agent"].includes(args[2])));
    assert.equal(h.calls.filter(args => args[2] === "finalize-agent").length, 1);
  }
});

test("incomplete or substituted final replies are not presented as completed", async () => {
  for (const state of ["outcome_unknown", "foreign"]) {
    const f = finalFixture(), h = harness(f, async args => {
      if (args[2] === "final-agent-result") return f.view("authorized");
      assert.equal(args[2], "finalize-agent");
      if (state === "outcome_unknown") return f.view(state);
      f.authorization.id = randomUUID(); f.dispatch.authorization.id = f.authorization.id; f.receipt.dispatch.id = f.authorization.id;
      return f.view();
    });
    await assert.rejects(() => h.backend.recover(f.projectId, f.runId, f.authorization.id), /not complete/);
  }
});

test("final Stop cancels pre-dispatch verification and waits for owned work to settle", async () => {
  const f = finalFixture(); let release, started;
  const entered = new Promise(resolve => { started = resolve; });
  const h = harness(f, async (args, _progress, signal) => {
    assert.equal(args[2], "final-agent-result"); assert.ok(signal instanceof AbortSignal);
    started(); await new Promise(resolve => { release = resolve; }); signal.throwIfAborted();
    assert.fail("Stopped verification cannot dispatch");
  });
  const work = h.backend.recover(f.projectId, f.runId, f.authorization.id);
  const rejected = assert.rejects(work, /abort/i); await entered;
  await assert.rejects(() => h.backend.stop(f.projectId, randomUUID()), /No final-evaluation worker/);
  await assert.rejects(() => h.backend.recover(f.projectId, f.runId, f.authorization.id), /Busy/);
  let settled = false; const stopped = h.backend.stop(f.projectId, f.runId).then(() => { settled = true; });
  await Promise.resolve(); assert.equal(settled, false); assert.equal(h.aborts(), 1);
  release(); await Promise.all([rejected, stopped]); assert.equal(settled, true);
  assert.ok(h.calls.every(args => args[2] !== "finalize-agent"));
  await assert.rejects(() => h.backend.stop(f.projectId, f.runId), /No final-evaluation worker/);
});

test("promotion is explicit, accepts only the exact saved final receipt and uses ordinary custody", async () => {
  const f = finalFixture(), h = harness(f, async args => {
    if (args[2] === "final-agent-result") return f.view();
    assert.deepEqual(args.slice(2), ["promote-final-agent", f.runId, "--expected-baseline-revision", f.baseline.id, "--expected-final-receipt", f.receipt.fingerprint]);
    return f.workspace;
  });
  const request = { baselineRevisionId: f.baseline.id, receiptFingerprint: f.receipt.fingerprint };
  for (const invalid of [{ ...request, accepted: true }, { ...request, baselineRevisionId: randomUUID() }, { ...request, receiptFingerprint: "sha256:" + "b".repeat(64) }]) {
    await assert.rejects(() => h.backend.promote(f.projectId, f.runId, invalid));
  }
  f.receipt.accepted = false;
  await assert.rejects(() => h.backend.promote(f.projectId, f.runId, request), /final-accepted/);
  assert.equal(h.calls.filter(args => args[2] === "promote-final-agent").length, 0);
  f.receipt.accepted = true;
  assert.deepEqual(await h.backend.promote(f.projectId, f.runId, request), f.workspace);
  assert.equal(h.calls.filter(args => args[2] === "promote-final-agent").length, 1);
});
