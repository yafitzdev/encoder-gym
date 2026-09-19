import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { parseOptimizationCases, caseChange } from "../dist/evidence/optimization-cases.js";
import { DevelopmentCasesController } from "../dist/evidence/development-cases-controller.js";

const hash = "sha256:" + "a".repeat(64);
function fixture() {
  const prediction = rank => ({fingerprint:hash,question:"Find primary evidence",taskKind:"route",expectedCapabilities:["search"],predictedCapabilities:["write"],expectedRank:rank});
  const source = () => ({reportId:randomUUID(),reportFingerprint:hash,diagnosticsFingerprint:hash,reportSupport:16,retrievalSupport:200,availability:"available_sample",sampleSize:1});
  return {projectId:randomUUID(),runId:randomUUID(),iterationId:randomUUID(),iteration:1,baselineModelId:randomUUID(),candidateModelId:randomUUID(),
    comparisons:[{suite:"development",suiteFingerprint:hash,sampleLimit:50,baseline:source(),candidate:source(),cases:[{sourceRowId:"row",baseline:prediction(3),candidate:prediction(2),change:"rank_improved"}]}]};
}
const parse = value => parseOptimizationCases(value,value.projectId,value.runId,value.iteration);
test("saved comparisons validate exact evidence and both rank directions without inventing absent predictions", () => {
  const value = fixture(); assert.deepEqual(parse(value),value);
  const pair = value.comparisons[0].cases[0];
  pair.candidate.expectedRank=4;pair.change="rank_regressed";assert.deepEqual(parse(value),value);
  pair.candidate.expectedRank=3;pair.change="rank_unchanged";assert.deepEqual(parse(value),value);
  pair.candidate.question="Changed input";pair.change="not_comparable";assert.deepEqual(parse(value),value);
  pair.candidate=null;value.comparisons[0].candidate.sampleSize=0;value.comparisons[0].candidate.availability="available_empty";assert.deepEqual(parse(value),value);
  value.comparisons[0].candidate.sampleSize=null;value.comparisons[0].candidate.diagnosticsFingerprint=null;value.comparisons[0].candidate.retrievalSupport=null;value.comparisons[0].candidate.availability="missing";assert.deepEqual(parse(value),value);
  for (const availability of ["corrupt","incompatible"]) { value.comparisons[0].candidate.availability=availability;assert.deepEqual(parse(value),value); }
  assert.equal(caseChange(null,pair.baseline),"not_comparable");
});
test("case decoder rejects foreign identities, injected native payloads and false comparison claims", () => {
  for (const change of [v=>v.comparisons[0].sealedScore=1,v=>v.comparisons[0].cases[0].candidate.rawTrace="secret",
    v=>v.comparisons[0].candidate.sampleSize=0,v=>v.comparisons[0].candidate.retrievalSupport=0,v=>v.comparisons[0].sampleLimit=100,
    v=>v.comparisons[0].cases[0].change="rank_regressed",v=>v.comparisons[0].cases[0].candidate.expectedRank=0,
    v=>v.comparisons[0].cases[0].candidate.expectedCapabilities.push("search"),v=>v.comparisons[0].cases[0].candidate.question=null,
    v=>v.comparisons[0].cases.push(v.comparisons[0].cases[0]),v=>v.comparisons.push(v.comparisons[0]),v=>v.comparisons=[]]) {
    const value=fixture();change(value);assert.throws(()=>parse(value));
  }
  const value=fixture();assert.throws(()=>parseOptimizationCases(value,randomUUID(),value.runId,1));
  assert.throws(()=>parseOptimizationCases(value,value.projectId,randomUUID(),1));
  assert.throws(()=>parseOptimizationCases(value,value.projectId,value.runId,2));
});
function iteration(value) {return {id:value.iterationId,number:value.iteration,completed:true,noChange:false,modelId:value.candidateModelId,startingModelId:value.baselineModelId};}
function deferred() {let resolve;const promise=new Promise(done=>resolve=done);return {promise,resolve};}
test("case loading is explicit, deduplicated, iteration-isolated and fenced after refresh", async () => {
  const value=fixture(),first=deferred(),second=deferred();let calls=0;
  const controller=new DevelopmentCasesController(value.projectId,()=>{calls++;return calls===1?first.promise:second.promise;},()=>{});
  assert.equal(controller.state(value.runId,value.iterationId).status,"idle");assert.equal(calls,0);
  const reading=controller.load(value.runId,iteration(value));await controller.load(value.runId,iteration(value));assert.equal(calls,1);
  controller.clear();const retry=controller.load(value.runId,iteration(value));first.resolve(value);await reading;
  assert.equal(controller.state(value.runId,value.iterationId).status,"loading");
  second.resolve(value);await retry;assert.equal(controller.state(value.runId,value.iterationId).status,"ready");
  assert.equal(controller.state(value.runId,randomUUID()).status,"idle");
  await controller.load(value.runId,iteration(value));assert.equal(calls,2);
});
test("uncompleted/no-change iterations never load diagnostics and model substitution fails closed", async () => {
  const value=fixture();let calls=0;
  const controller=new DevelopmentCasesController(value.projectId,async()=>{calls++;return {...value,candidateModelId:randomUUID()};},()=>{});
  await controller.load(value.runId,{...iteration(value),completed:false});await controller.load(value.runId,{...iteration(value),noChange:true});assert.equal(calls,0);
  await controller.load(value.runId,iteration(value));assert.equal(controller.state(value.runId,value.iterationId).status,"error");
});
