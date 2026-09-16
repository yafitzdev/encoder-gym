import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { OptimizationFinalController } from "../dist/evidence/optimization-final-controller.js";
import { finalFixture } from "./optimization-final-fixture.mjs";
const deferred = () => { let resolve, reject; const promise = new Promise((a,b) => { resolve=a; reject=b; }); return {promise,resolve,reject}; };
function fixture(overrides = {}) {
  const f = finalFixture(), calls = [], review = {token:randomUUID(),scope:f.scope}; let saved=null;
  const bridge = {
    agentFinalResult: async () => saved,
    reviewAgentFinal: async () => { calls.push("review"); return review; },
    authorizeAgentFinal: async (project, run, token) => { assert.equal(project,f.projectId); assert.equal(run,f.runId); assert.equal(token,review.token); calls.push("authorize"); return saved=f.view(); },
    recoverAgentFinal: async (_project,_run,grant) => { assert.equal(grant,f.authorization.id); calls.push("recover"); return saved=f.view(); },
    promoteAgentFinal: async (_project,_run,request) => { assert.deepEqual(request,{baselineRevisionId:f.scope.comparisonBaselineRevision.id,receiptFingerprint:f.receipt.fingerprint}); calls.push("promote"); return f.workspace; },
    selectProject: async () => ({content:{state:"ready",workspace:{managed:f.workspace}}}), ...overrides,
  };
  return {...f,calls,bridge,review,save:value=>{saved=value;},controller:new OptimizationFinalController(f.projectId,bridge,()=>{})};
}
test("reads and review never authorize; consent and promotion are distinct explicit actions", async () => {
  const f=fixture(), c=f.controller;
  await c.ensure(f.runId); assert.deepEqual(f.calls,[]);
  await c.authorize(f.runId); assert.deepEqual(f.calls,[]);
  await c.review(f.runId); assert.deepEqual(f.calls,["review"]);
  await c.authorize(f.runId); assert.deepEqual(f.calls,["review","authorize"]);
  await c.promote(f.runId); assert.deepEqual(f.calls,["review","authorize","promote"]);
  assert.equal(c.state(f.runId).promoted,true);
});
test("uncertain consent recovers saved grant after reopening without fresh consent", async () => {
  const f=fixture(); f.bridge.authorizeAgentFinal=async()=>{f.save(f.view("outcome_unknown"));throw Error("Lost reply");};
  await f.controller.review(f.runId); await f.controller.authorize(f.runId);
  assert.equal(f.controller.state(f.runId).view.state,"outcome_unknown");
  const reopened=new OptimizationFinalController(f.projectId,f.bridge,()=>{});
  await reopened.ensure(f.runId); await reopened.recover(f.runId);
  assert.deepEqual(f.calls,["review","recover"]);
});
test("rejected results cannot promote and duplicate actions cannot dispatch", async () => {
  const f=fixture(), pending=deferred(); f.bridge.authorizeAgentFinal=()=>pending.promise;
  await f.controller.review(f.runId); const work=f.controller.authorize(f.runId);
  await f.controller.authorize(f.runId); await f.controller.review(randomUUID());
  const rejected=f.view(); rejected.execution.result.accepted=false; pending.resolve(rejected); await work;
  await f.controller.promote(f.runId); assert.deepEqual(f.calls,["review"]);
});
test("late status reads cannot erase explicit completed results", async () => {
  const f=fixture(), pending=deferred(); f.bridge.agentFinalResult=()=>pending.promise;
  const stale=f.controller.refresh(f.runId); f.bridge.agentFinalResult=async()=>null;
  await f.controller.review(f.runId); await f.controller.authorize(f.runId);
  pending.resolve(null); await stale; assert.equal(f.controller.state(f.runId).view.state,"completed");
});
test("Stop waits for owned work and preserves unknown final custody", async () => {
  const f=fixture(), pending=deferred(); f.bridge.authorizeAgentFinal=()=>pending.promise;
  f.bridge.stopAgentFinal=async()=>{f.save(f.view("outcome_unknown"));pending.reject(Error("Stopped"));};
  await f.controller.review(f.runId); const work=f.controller.authorize(f.runId);
  await f.controller.stop(f.runId); await work;
  assert.equal(f.controller.busyRun,undefined); assert.equal(f.controller.state(f.runId).view.state,"outcome_unknown");
});
