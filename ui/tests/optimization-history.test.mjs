import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { parseOptimizationHistory } from "../dist/evidence/optimization-history.js";

const projectId = randomUUID(), runId = randomUUID();
const first = { id: randomUUID(), number: 1, createdAt: new Date().toISOString(), startingModelId: randomUUID(), inputDatasetVersionId: randomUUID(), benchmarkVersionId: randomUUID(),
  experimentRunId: randomUUID(), qualifiedDatasetVersionId: randomUUID(), trainingDatasetVersionId: randomUUID(), modelId: randomUUID(),
  completed: true, noChange: false, selected: false, developmentPassed: false,
  checks: [{ reportId: randomUUID(), suite: "development", metric: "loss", direction: "lower_is_better", baseline: .5, candidate: .6, passed: false }] };
const history = () => ({ projectId, runId, iterations: [structuredClone(first)] });

const repair = () => ({ decisionCallId: randomUUID(), proposalFingerprint: "sha256:" + "a".repeat(64), maximumRowChanges: 3, inputRows: 2, strategy: "question_variants_preserve_context",
  proposal: { summary: "Fix ambiguous wording", stop: false, removals: [{rowId:"row",reason:"Conflicting example",evidenceIds:["cluster"]}], additions:[{templateRowId:"row",instruction:"Generate a clear search request",count:2,evidenceIds:["cluster"]}] },
  generation:[{targetIndex:0,requested:2,admitted:1,rejected:1,unresolved:0,attempts:1,rejectionReasons:{"Duplicate native input":1}}],
  publication:{datasetVersionId:first.qualifiedDatasetVersionId,added:1,removed:1,rows:2,crossBatchDuplicates:0},
  evidence:[{id:"cluster",fingerprint:"sha256:"+"b".repeat(64),label:"expected capability: search",trainingRows:1,totalTrainingRows:2,overlapping:true,development:[{suite:"development",support:4,recallAt1:0,originalBaselineRecallAt1:.5}]}] });

test("repair plans retain exact requested, rejected and published counts without inventing old history", () => {
  const value = history(); value.iterations[0].repairPlan = repair();
  assert.deepEqual(parseOptimizationHistory(value, projectId, runId)[0].repairPlan, value.iterations[0].repairPlan);
  value.iterations[0].repairPlan = null;
  assert.equal(parseOptimizationHistory(value, projectId, runId)[0].repairPlan, null);
  delete value.iterations[0].repairPlan;
  assert.equal(parseOptimizationHistory(value, projectId, runId)[0].repairPlan, undefined);
});

test("repair plans reject invented membership, inconsistent counts and injected native or sealed payloads", () => {
  for (const alter of [
    p => p.rawRows = [], p => p.evidence[0].sealedScore = 1, p => p.evidence[0].development[0].recallAt1 = Infinity,
    p => p.publication.datasetVersionId = randomUUID(), p => p.publication.added = 2, p => p.publication.rows = 0,
    p => p.proposal.additions[0].evidenceIds = ["invented"], p => p.proposal.removals.push(p.proposal.removals[0]),
    p => p.maximumRowChanges = 2, p => p.evidence.push(p.evidence[0]), p => p.evidence[0].totalTrainingRows = 3,
    p => p.generation[0].unresolved = 1, p => p.generation[0].rejectionReasons = {}, p => p.generation[0].attempts = 0,
    p => p.generation[0].requested = 3, p => p.proposal.stop = true, p => p.strategy = "modify_benchmark",
  ]) { const value = history(); value.iterations[0].repairPlan = repair(); alter(value.iterations[0].repairPlan); assert.throws(() => parseOptimizationHistory(value, projectId, runId)); }
});

test("canary history preserves bounded samples and rejects unsafe or contradictory receipts", () => {
  const value = history(); const plan = repair(); value.iterations[0].repairPlan = plan;
  Object.assign(value.iterations[0], {completed:false,developmentPassed:null,checks:[],modelId:null,experimentRunId:null,qualifiedDatasetVersionId:null,trainingDatasetVersionId:null});
  plan.canary = { policy: "first_batch_all_admitted_v1", status: "rejected", callId: randomUUID(), requested: 2, admitted: 1,
    rejected: [{index:1,reason:"Duplicate native input"}], rows: [{index:0,fingerprint:"sha256:"+"f".repeat(64),question:"Find the primary paper",taskKind:"route"}] };
  plan.publication = null;
  assert.deepEqual(parseOptimizationHistory(value, projectId, runId)[0].repairPlan.canary, plan.canary);
  for (const alter of [p=>p.canary.rows[0].registry={},p=>p.canary.rows[0].index=1,p=>p.canary.requested=9,
    p=>p.canary.status="passed",p=>p.canary.policy=null,p=>p.canary.callId=null,p=>p.canary.rows[0].question="x".repeat(8193),
    p=>p.publication=repair().publication,p=>p.canary.rejected[0].reason="",p=>p.canary.rows.push(p.canary.rows[0])]) {
    const bad=structuredClone(value);alter(bad.iterations[0].repairPlan);assert.throws(()=>parseOptimizationHistory(bad,projectId,runId));
  }
  plan.canary = {policy:null,status:"disabled",callId:null,requested:0,admitted:0,rejected:[],rows:[]};
  assert.equal(parseOptimizationHistory(value,projectId,runId)[0].repairPlan.canary.status,"disabled");
});

test("history preserves rejected candidate custody and explicit lower-is-better comparisons", () => {
  assert.deepEqual(parseOptimizationHistory(history(), projectId, runId), [first]);
  const value = history(); value.iterations.push({ ...first, id: randomUUID(), number: 2, noChange: true, developmentPassed: null,
    experimentRunId: null, qualifiedDatasetVersionId: null, trainingDatasetVersionId: null, modelId: null, checks: [] });
  assert.equal(parseOptimizationHistory(value, projectId, runId)[1].noChange, true);
});

test("history rejects foreign identity, injected evidence, invalid ordering and false custody", () => {
  for (const alter of [
    value => value.runId = randomUUID(), value => value.projectId = randomUUID(),
    value => value.iterations[0].number = 2, value => value.iterations[0].sealedScore = 0,
    value => value.iterations[0].selected = true, value => value.iterations[0].modelId = null,
    value => value.iterations[0].experimentRunId = null, value => value.iterations[0].completed = false,
    value => value.iterations[0].noChange = true, value => value.iterations[0].checks[0].candidate = Infinity,
    value => value.iterations[0].checks[0].direction = "guess", value => value.iterations[0].checks[0].rawRows = [],
  ]) { const value = history(); alter(value); assert.throws(() => parseOptimizationHistory(value, projectId, runId)); }
});
