/** Closed, development-only read model. No provider payloads or native rows. */
export interface RepairEvidence {
  id: string; fingerprint: string; label: string;
  trainingRows: number | null; totalTrainingRows: number | null; overlapping: boolean;
  development: { suite: string; support: number; recallAt1: number; originalBaselineRecallAt1: number | null }[];
}
export interface DatasetRepairPlan {
  decisionCallId: string; proposalFingerprint: string; maximumRowChanges: number; inputRows: number;
  strategy: "question_variants_preserve_context";
  proposal: { summary: string; stop: boolean; stopReason?: "no_change" | "unsupported_repair";
    removals: { rowId: string; reason: string; evidenceIds: string[] }[];
    additions: { templateRowId: string; instruction: string; count: number; evidenceIds: string[] }[] };
  generation: { targetIndex: number; requested: number; admitted: number; rejected: number; unresolved: number; attempts: number; rejectionReasons: Record<string, number> }[];
  publication: { datasetVersionId: string; added: number; removed: number; rows: number; crossBatchDuplicates: number;
    semanticRejections?: number; coupledRejections?: number; targetCounts?: RepairPublicationCount[] } | null;
  evidence: RepairEvidence[];
  canary?: GenerationCanary;
  v3Canary?: V3CanaryGate;
  notExecuted?: RepairNotExecuted;
  outcomes?: RepairOutcome[];
}

export interface GenerationCanary {
  policy: "first_batch_all_admitted_v1" | "per_combination_semantic_v3" | null;
  status: "disabled" | "not_required" | "pending" | "interrupted" | "passed" | "rejected";
  callId: string | null; requested: number; admitted: number;
  rejected: { index: number; reason: string }[];
  rows: { index: number; fingerprint: string; question: string; taskKind: string | null }[];
}

export interface RepairPublicationCount {
  targetIndex: number; targetId: string | null; anchorRowId: string; combinationId: string | null;
  requested: number; generated: number; structurallyAdmitted: number; semanticallyAdmitted: number;
  coupledExcluded: number; duplicateExcluded: number; published: number;
}

export interface V3CanaryGate {
  status: "passed" | "rejected";
  units: { targetId: string; combinationId: string; strategy: "label_preserving_variant" | "existing_anchor_contrast";
    contrastPairId: string | null; requested: number; structurallyAdmitted: number; semanticallyAdmitted: number; rejectedRowIds: string[] }[];
}

export interface RepairNotExecuted {
  schemaVersion: 1; runId: string; iteration: number; proposalFingerprint: string; repairPlanFingerprint: string;
  reason: "canary_rejected" | "zero_surviving_edits"; canary: V3CanaryGate;
}

export interface RepairOutcome {
  schemaVersion: 1; runId: string; iteration: number; targetId: string; repairPlanFingerprint: string;
  proposalFingerprint: string; interventionFingerprint: string; clusterKeys: string[];
  intervention: { kind: "label_preserving_variants"; anchorIds: string[] }
    | { kind: "existing_anchor_contrast"; pairIds: string[] }
    | { kind: "proven_redundant_row_removal"; rows: { rowId: string; retainedRowId: string }[] };
  hypothesis: string; inputDataset: RepairIdentity; outputDataset: RepairIdentity; candidate: RepairIdentity;
  inputDevelopmentEvidenceFingerprint: string; outputDevelopmentEvidenceFingerprint: string; reports: RepairIdentity[];
  edits: { requestedAdditions: number; generated: number; structurallyAdmitted: number; semanticallyAdmitted: number;
    publishedAdditions: number; requestedRemovals: number; publishedRemovals: number };
  metricChange: "improved" | "regressed" | "unchanged" | "unavailable";
  observations: { clusterKey: string; suite: string; metric: string; expectedDirection: "increase" | "decrease";
    support: number | null; originalBaseline: number | null; precedingCandidate: number | null; candidate: number | null;
    deltaFromOriginalBaseline: number | null; deltaFromPrecedingCandidate: number | null;
    change: "improved" | "regressed" | "unchanged" | "unavailable" }[];
  globalVerdict: "keep" | "reject"; limitations: string[];
}

interface RepairIdentity { id: string; fingerprint: string }

function reject(): never { throw new Error("Invalid saved dataset repair plan."); }
function object(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== keys.length || Object.keys(value).some(key => !keys.includes(key))) reject();
  return value as Record<string, unknown>;
}
function text(value: unknown, max = 2000): string { if (typeof value !== "string" || !value.trim() || [...value].length > max) reject(); return value; }
function integer(value: unknown, max = Number.MAX_SAFE_INTEGER): number { if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > max) reject(); return value; }
function rate(value: unknown): number { if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) reject(); return value; }
function finite(value: unknown): number { if (typeof value !== "number" || !Number.isFinite(value)) reject(); return value; }
function bool(value: unknown): boolean { if (typeof value !== "boolean") reject(); return value; }
function uuid(value: unknown): string { const result = text(value, 36); if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(result)) reject(); return result.toLowerCase(); }
function hash(value: unknown): string { const result = text(value, 71); if (!/^sha256:[0-9a-f]{64}$/.test(result)) reject(); return result; }
function array(value: unknown, max = 5000): unknown[] { if (!Array.isArray(value) || value.length > max) reject(); return value; }
function references(value: unknown): string[] { const result = array(value, 20).map(id => text(id, 128)); if (!result.length || new Set(result).size !== result.length) reject(); return result; }

function identity(value: unknown): RepairIdentity {
  const item = object(value, ["id", "fingerprint"]);
  return { id: text(item.id, 256), fingerprint: hash(item.fingerprint) };
}

function v3Canary(value: unknown): V3CanaryGate {
  const item = object(value, ["status", "units"]);
  if (item.status !== "passed" && item.status !== "rejected") reject();
  const units = array(item.units, 32).map(raw => {
    const unit = object(raw, ["targetId", "combinationId", "strategy", "contrastPairId", "requested", "structurallyAdmitted", "semanticallyAdmitted", "rejectedRowIds"]);
    if (unit.strategy !== "label_preserving_variant" && unit.strategy !== "existing_anchor_contrast") reject();
    const result = { targetId: text(unit.targetId, 128), combinationId: hash(unit.combinationId), strategy: unit.strategy,
      contrastPairId: unit.contrastPairId === null ? null : text(unit.contrastPairId, 128), requested: integer(unit.requested, 16),
      structurallyAdmitted: integer(unit.structurallyAdmitted, 16), semanticallyAdmitted: integer(unit.semanticallyAdmitted, 16),
      rejectedRowIds: array(unit.rejectedRowIds, 16).map(id => text(id, 256)) } as V3CanaryGate["units"][number];
    if (!result.requested || result.semanticallyAdmitted > result.structurallyAdmitted || result.structurallyAdmitted > result.requested
      || new Set(result.rejectedRowIds).size !== result.rejectedRowIds.length
      || (result.strategy === "existing_anchor_contrast") !== (result.contrastPairId !== null)) reject();
    return result;
  });
  if (!units.length || new Set(units.map(unit => unit.combinationId)).size !== units.length) reject();
  const passed = units.every(unit => unit.requested === unit.semanticallyAdmitted && unit.rejectedRowIds.length === 0);
  if ((item.status === "passed") !== passed) reject();
  return { status: item.status, units };
}

function repairOutcome(value: unknown): RepairOutcome {
  const item = object(value, ["schemaVersion", "runId", "iteration", "targetId", "repairPlanFingerprint", "proposalFingerprint", "interventionFingerprint", "clusterKeys", "intervention", "hypothesis", "inputDataset", "outputDataset", "candidate", "inputDevelopmentEvidenceFingerprint", "outputDevelopmentEvidenceFingerprint", "reports", "edits", "metricChange", "observations", "globalVerdict", "limitations"]);
  if (item.schemaVersion !== 1 || !["improved", "regressed", "unchanged", "unavailable"].includes(String(item.metricChange)) || !["keep", "reject"].includes(String(item.globalVerdict))) reject();
  const rawIntervention = item.intervention as Record<string, unknown>;
  let intervention: RepairOutcome["intervention"];
  if (rawIntervention?.kind === "label_preserving_variants") {
    const parsed = object(rawIntervention, ["kind", "anchorIds"]); intervention = { kind: rawIntervention.kind, anchorIds: references(parsed.anchorIds) };
  } else if (rawIntervention?.kind === "existing_anchor_contrast") {
    const parsed = object(rawIntervention, ["kind", "pairIds"]); intervention = { kind: rawIntervention.kind, pairIds: references(parsed.pairIds) };
  } else if (rawIntervention?.kind === "proven_redundant_row_removal") {
    const parsed = object(rawIntervention, ["kind", "rows"]); intervention = { kind: rawIntervention.kind, rows: array(parsed.rows, 32).map(raw => { const row = object(raw, ["rowId", "retainedRowId"]); const result = { rowId: text(row.rowId, 128), retainedRowId: text(row.retainedRowId, 128) }; if (result.rowId === result.retainedRowId) reject(); return result; }) };
    if (!intervention.rows.length) reject();
  } else reject();
  const rawEdits = object(item.edits, ["requestedAdditions", "generated", "structurallyAdmitted", "semanticallyAdmitted", "publishedAdditions", "requestedRemovals", "publishedRemovals"]);
  const edits = { requestedAdditions: integer(rawEdits.requestedAdditions, 5000), generated: integer(rawEdits.generated, 5000), structurallyAdmitted: integer(rawEdits.structurallyAdmitted, 5000), semanticallyAdmitted: integer(rawEdits.semanticallyAdmitted, 5000), publishedAdditions: integer(rawEdits.publishedAdditions, 5000), requestedRemovals: integer(rawEdits.requestedRemovals, 5000), publishedRemovals: integer(rawEdits.publishedRemovals, 5000) };
  if (edits.generated !== edits.requestedAdditions || edits.publishedAdditions > edits.semanticallyAdmitted || edits.semanticallyAdmitted > edits.structurallyAdmitted || edits.structurallyAdmitted > edits.generated || edits.publishedRemovals > edits.requestedRemovals) reject();
  const nullable = (raw: unknown): number | null => raw === null ? null : finite(raw);
  const observations = array(item.observations, 128).map(raw => { const point = object(raw, ["clusterKey", "suite", "metric", "expectedDirection", "support", "originalBaseline", "precedingCandidate", "candidate", "deltaFromOriginalBaseline", "deltaFromPrecedingCandidate", "change"]); if (!['increase','decrease'].includes(String(point.expectedDirection)) || !["improved", "regressed", "unchanged", "unavailable"].includes(String(point.change))) reject(); return { clusterKey:text(point.clusterKey,160),suite:text(point.suite,128),metric:text(point.metric,128),expectedDirection:point.expectedDirection as "increase"|"decrease",support:point.support===null?null:integer(point.support),originalBaseline:nullable(point.originalBaseline),precedingCandidate:nullable(point.precedingCandidate),candidate:nullable(point.candidate),deltaFromOriginalBaseline:nullable(point.deltaFromOriginalBaseline),deltaFromPrecedingCandidate:nullable(point.deltaFromPrecedingCandidate),change:point.change as RepairOutcome["metricChange"] }; });
  const clusterKeys = references(item.clusterKeys);
  const reports = array(item.reports, 20).map(identity), limitations = array(item.limitations, 8).map(value => text(value, 1000));
  if (!observations.length || !reports.length || !limitations.length || new Set(clusterKeys).size !== clusterKeys.length || new Set(reports.map(report => `${report.id}:${report.fingerprint}`)).size !== reports.length) reject();
  return { schemaVersion:1, runId:uuid(item.runId), iteration:integer(item.iteration,10), targetId:text(item.targetId,128), repairPlanFingerprint:hash(item.repairPlanFingerprint), proposalFingerprint:hash(item.proposalFingerprint), interventionFingerprint:hash(item.interventionFingerprint), clusterKeys, intervention, hypothesis:text(item.hypothesis,1000), inputDataset:identity(item.inputDataset), outputDataset:identity(item.outputDataset), candidate:identity(item.candidate), inputDevelopmentEvidenceFingerprint:hash(item.inputDevelopmentEvidenceFingerprint), outputDevelopmentEvidenceFingerprint:hash(item.outputDevelopmentEvidenceFingerprint), reports, edits, metricChange:item.metricChange as RepairOutcome["metricChange"], observations, globalVerdict:item.globalVerdict as RepairOutcome["globalVerdict"], limitations };
}

export function parseDatasetRepairPlan(value: unknown): DatasetRepairPlan | null {
  if (value === null) return null;
  const hasCanary = !!value && typeof value === "object" && Object.hasOwn(value, "canary");
  const hasV3Canary = !!value && typeof value === "object" && Object.hasOwn(value, "v3Canary");
  const hasNotExecuted = !!value && typeof value === "object" && Object.hasOwn(value, "notExecuted");
  const hasOutcomes = !!value && typeof value === "object" && Object.hasOwn(value, "outcomes");
  const plan = object(value, ["decisionCallId", "proposalFingerprint", "maximumRowChanges", "proposal", "generation", "inputRows", "strategy", "publication", "evidence", ...(hasCanary ? ["canary"] : []), ...(hasV3Canary ? ["v3Canary"] : []), ...(hasNotExecuted ? ["notExecuted"] : []), ...(hasOutcomes ? ["outcomes"] : [])]);
  if (plan.strategy !== "question_variants_preserve_context") reject();
  const rawProposal = plan.proposal as Record<string, unknown>;
  const hasStopReason = !!rawProposal && Object.hasOwn(rawProposal, "stopReason");
  const proposal = object(plan.proposal, ["summary", "stop", "removals", "additions", ...(hasStopReason ? ["stopReason"] : [])]);
  const stopReason = !hasStopReason ? undefined : (() => { if (proposal.stopReason !== "no_change" && proposal.stopReason !== "unsupported_repair") reject(); return proposal.stopReason; })();
  const result: DatasetRepairPlan = {
    decisionCallId: uuid(plan.decisionCallId), proposalFingerprint: hash(plan.proposalFingerprint), maximumRowChanges: integer(plan.maximumRowChanges, 5000),
    inputRows: integer(plan.inputRows), strategy: plan.strategy,
    proposal: { summary: text(proposal.summary, 400), stop: bool(proposal.stop), ...(stopReason ? { stopReason } : {}),
      removals: array(proposal.removals).map(value => { const row = object(value, ["rowId", "reason", "evidenceIds"]); return { rowId: text(row.rowId, 128), reason: text(row.reason), evidenceIds: references(row.evidenceIds) }; }),
      additions: array(proposal.additions).map(value => { const target = object(value, ["templateRowId", "instruction", "count", "evidenceIds"]); const count = integer(target.count, 5000); if (!count) reject(); return { templateRowId: text(target.templateRowId, 128), instruction: text(target.instruction), count, evidenceIds: references(target.evidenceIds) }; }) },
    generation: array(plan.generation).map((value, index) => {
      const target = object(value, ["targetIndex", "requested", "admitted", "rejected", "unresolved", "attempts", "rejectionReasons"]);
      if (target.targetIndex !== index || !target.rejectionReasons || typeof target.rejectionReasons !== "object" || Array.isArray(target.rejectionReasons)) reject();
      const rejectionReasons = Object.fromEntries(Object.entries(target.rejectionReasons).map(([reason, count]) => { text(reason, 400); const amount = integer(count, 5000); if (!amount) reject(); return [reason, amount]; }));
      return { targetIndex: index, requested: integer(target.requested, 5000), admitted: integer(target.admitted, 5000), rejected: integer(target.rejected, 5000), unresolved: integer(target.unresolved, 5000), attempts: integer(target.attempts, 0xffff_ffff), rejectionReasons };
    }),
    publication: plan.publication === null ? null : (() => {
      const raw = plan.publication as Record<string, unknown>, extended = Object.hasOwn(raw, "targetCounts") || Object.hasOwn(raw, "semanticRejections") || Object.hasOwn(raw, "coupledRejections");
      const value = object(plan.publication, ["datasetVersionId", "added", "removed", "rows", "crossBatchDuplicates", ...(extended ? ["semanticRejections", "coupledRejections", "targetCounts"] : [])]);
      const publication: NonNullable<DatasetRepairPlan["publication"]> = { datasetVersionId: uuid(value.datasetVersionId), added: integer(value.added, 5000), removed: integer(value.removed, 5000), rows: integer(value.rows), crossBatchDuplicates: integer(value.crossBatchDuplicates, 5000) };
      if (extended) {
        publication.semanticRejections = integer(value.semanticRejections, 5000); publication.coupledRejections = integer(value.coupledRejections, 5000);
        publication.targetCounts = array(value.targetCounts, 32).map(raw => { const count = object(raw, ["targetIndex", "targetId", "anchorRowId", "combinationId", "requested", "generated", "structurallyAdmitted", "semanticallyAdmitted", "coupledExcluded", "duplicateExcluded", "published"]); const result: RepairPublicationCount = { targetIndex:integer(count.targetIndex,3),targetId:count.targetId===null?null:text(count.targetId,128),anchorRowId:text(count.anchorRowId,128),combinationId:count.combinationId===null?null:hash(count.combinationId),requested:integer(count.requested,5000),generated:integer(count.generated,5000),structurallyAdmitted:integer(count.structurallyAdmitted,5000),semanticallyAdmitted:integer(count.semanticallyAdmitted,5000),coupledExcluded:integer(count.coupledExcluded,5000),duplicateExcluded:integer(count.duplicateExcluded,5000),published:integer(count.published,5000) }; if (result.generated !== result.requested || result.published + result.duplicateExcluded + result.coupledExcluded > result.semanticallyAdmitted || result.semanticallyAdmitted > result.structurallyAdmitted || result.structurallyAdmitted > result.generated) reject(); return result; });
      }
      return publication;
    })(),
    evidence: array(plan.evidence, 100000).map(value => {
      const item = object(value, ["id", "fingerprint", "label", "trainingRows", "totalTrainingRows", "overlapping", "development"]);
      return { id: text(item.id, 128), fingerprint: hash(item.fingerprint), label: text(item.label, 450),
        trainingRows: item.trainingRows === null ? null : integer(item.trainingRows), totalTrainingRows: item.totalTrainingRows === null ? null : integer(item.totalTrainingRows), overlapping: bool(item.overlapping),
        development: array(item.development, 20).map(value => { const metric = object(value, ["suite", "support", "recallAt1", "originalBaselineRecallAt1"]); return { suite: text(metric.suite, 200), support: integer(metric.support), recallAt1: rate(metric.recallAt1), originalBaselineRecallAt1: metric.originalBaselineRecallAt1 === null ? null : rate(metric.originalBaselineRecallAt1) }; }) };
    }),
  };
  const { removals, additions, stop } = result.proposal;
  const requested = removals.length + additions.reduce((sum, target) => sum + target.count, 0);
  if (!result.inputRows || requested > result.maximumRowChanges || stop !== (requested === 0) || !!result.proposal.stopReason && !stop || removals.length > result.inputRows
    || new Set(removals.map(row => row.rowId)).size !== removals.length || result.generation.length !== additions.length) reject();
  for (const [index, target] of result.generation.entries()) {
    if (target.requested !== additions[index]!.count || target.admitted + target.rejected + target.unresolved !== target.requested
      || Object.values(target.rejectionReasons).reduce((a, b) => a + b, 0) !== target.rejected || target.attempts === 0 && target.unresolved !== target.requested) reject();
  }
  const refs = new Set([...removals, ...additions].flatMap(item => item.evidenceIds));
  if (new Set(result.evidence.map(item => item.id)).size !== result.evidence.length || refs.size !== result.evidence.length || result.evidence.some(item => !refs.has(item.id)
    || (item.trainingRows === null) !== (item.totalTrainingRows === null) || item.trainingRows !== null && (item.totalTrainingRows !== result.inputRows || item.trainingRows > item.totalTrainingRows)
    || new Set(item.development.map(metric => metric.suite)).size !== item.development.length)) reject();
  if (result.publication && (stop || result.generation.some(target => target.unresolved > 0) || result.publication.removed !== removals.length
    || result.publication.added + result.publication.crossBatchDuplicates + (result.publication.semanticRejections ?? 0) + (result.publication.coupledRejections ?? 0) !== result.generation.reduce((sum, target) => sum + target.admitted, 0)
    || result.publication.rows !== result.inputRows - result.publication.removed + result.publication.added)) reject();
  if (result.publication?.targetCounts && (result.publication.targetCounts.reduce((sum, count) => sum + count.requested, 0) !== additions.reduce((sum, target) => sum + target.count, 0)
    || result.publication.targetCounts.reduce((sum, count) => sum + count.structurallyAdmitted, 0) !== result.generation.reduce((sum, target) => sum + target.admitted, 0)
    || result.publication.targetCounts.reduce((sum, count) => sum + count.published, 0) !== result.publication.added
    || result.publication.targetCounts.reduce((sum, count) => sum + count.duplicateExcluded, 0) !== result.publication.crossBatchDuplicates
    || result.publication.targetCounts.reduce((sum, count) => sum + count.coupledExcluded, 0) !== (result.publication.coupledRejections ?? 0))) reject();
  if (hasCanary) {
    const raw = object(plan.canary, ["policy", "status", "callId", "requested", "admitted", "rejected", "rows"]);
    if (raw.policy !== null && raw.policy !== "first_batch_all_admitted_v1" && raw.policy !== "per_combination_semantic_v3" || !["disabled", "not_required", "pending", "interrupted", "passed", "rejected"].includes(String(raw.status))) reject();
    const canary: GenerationCanary = { policy: raw.policy as GenerationCanary["policy"], status: raw.status as GenerationCanary["status"], callId: raw.callId === null ? null : uuid(raw.callId), requested: integer(raw.requested, 64), admitted: integer(raw.admitted, 64),
      rejected: array(raw.rejected, 64).map(value => { const row = object(value, ["index", "reason"]); return { index: integer(row.index, 63), reason: text(row.reason, 400) }; }),
      rows: array(raw.rows, 64).map(value => { const row = object(value, ["index", "fingerprint", "question", "taskKind"]); return { index: integer(row.index, 7), fingerprint: hash(row.fingerprint), question: text(row.question, 8192), taskKind: row.taskKind === null ? null : text(row.taskKind, 200) }; }) };
    const indices = [...canary.rows, ...canary.rejected].map(row => row.index);
    const terminal = canary.status === "passed" || canary.status === "rejected";
    if (canary.policy === "per_combination_semantic_v3") {
      if ((canary.status === "not_required") !== (additions.length === 0) || canary.callId !== null || canary.rejected.length !== 0
        || !terminal && canary.rows.length !== 0 || terminal !== hasV3Canary || result.publication && canary.status !== "passed"
        || stop !== (result.proposal.stopReason !== undefined)) reject();
    } else if ((canary.policy === null) !== (canary.status === "disabled") || canary.rows.length !== canary.admitted
        || new Set(indices).size !== indices.length || indices.some(index => index >= canary.requested)
        || canary.requested !== (canary.policy ? Math.min(additions[0]?.count ?? 0, 8) : 0)
        || canary.policy && (canary.status === "not_required") !== (additions.length === 0)
        || terminal && (!canary.callId || indices.length !== canary.requested || !canary.requested)
        || !terminal && indices.length !== 0
        || canary.status === "passed" && canary.rejected.length !== 0
        || canary.status === "rejected" && canary.rejected.length === 0
        || canary.status === "interrupted" && !canary.callId
        || ["disabled", "not_required"].includes(canary.status) && canary.callId !== null
        || result.publication && canary.policy && !["passed", "not_required"].includes(canary.status)
        || result.generation[0] && (canary.admitted > result.generation[0].admitted || canary.rejected.length > result.generation[0].rejected)) reject();
    result.canary = canary;
  }
  if (hasV3Canary) {
    const gate = v3Canary(plan.v3Canary); result.v3Canary = gate;
    if (result.canary?.policy !== "per_combination_semantic_v3" || result.canary.status !== gate.status
      || result.canary.requested !== gate.units.reduce((sum, unit) => sum + unit.requested, 0)
      || result.canary.admitted !== gate.units.reduce((sum, unit) => sum + unit.semanticallyAdmitted, 0)) reject();
  }
  if (hasNotExecuted) {
    const raw = object(plan.notExecuted, ["schemaVersion", "runId", "iteration", "proposalFingerprint", "repairPlanFingerprint", "reason", "canary"]);
    if (raw.schemaVersion !== 1 || raw.reason !== "canary_rejected" && raw.reason !== "zero_surviving_edits") reject();
    const receipt: RepairNotExecuted = { schemaVersion:1,runId:uuid(raw.runId),iteration:integer(raw.iteration,10),proposalFingerprint:hash(raw.proposalFingerprint),repairPlanFingerprint:hash(raw.repairPlanFingerprint),reason:raw.reason,canary:v3Canary(raw.canary) };
    if ((receipt.reason === "canary_rejected") !== (receipt.canary.status === "rejected")) reject();
    if (!result.v3Canary || JSON.stringify(receipt.canary) !== JSON.stringify(result.v3Canary) || receipt.proposalFingerprint !== result.proposalFingerprint || result.publication !== null) reject();
    result.notExecuted = receipt;
  }
  if (hasOutcomes) {
    const outcomes = array(plan.outcomes, 4).map(repairOutcome);
    if (new Set(outcomes.map(outcome => outcome.targetId)).size !== outcomes.length || outcomes.some(outcome => outcome.proposalFingerprint !== result.proposalFingerprint)) reject();
    result.outcomes = outcomes;
  }
  return result;
}
