/** Closed, development-only read model. No provider payloads or native rows. */
export interface RepairEvidence {
  id: string; fingerprint: string; label: string;
  trainingRows: number | null; totalTrainingRows: number | null; overlapping: boolean;
  development: { suite: string; support: number; recallAt1: number; originalBaselineRecallAt1: number | null }[];
}
export interface DatasetRepairPlan {
  decisionCallId: string; proposalFingerprint: string; maximumRowChanges: number; inputRows: number;
  strategy: "question_variants_preserve_context";
  proposal: { summary: string; stop: boolean;
    removals: { rowId: string; reason: string; evidenceIds: string[] }[];
    additions: { templateRowId: string; instruction: string; count: number; evidenceIds: string[] }[] };
  generation: { targetIndex: number; requested: number; admitted: number; rejected: number; unresolved: number; attempts: number; rejectionReasons: Record<string, number> }[];
  publication: { datasetVersionId: string; added: number; removed: number; rows: number; crossBatchDuplicates: number } | null;
  evidence: RepairEvidence[];
  canary?: GenerationCanary;
}

export interface GenerationCanary {
  policy: "first_batch_all_admitted_v1" | null;
  status: "disabled" | "not_required" | "pending" | "interrupted" | "passed" | "rejected";
  callId: string | null; requested: number; admitted: number;
  rejected: { index: number; reason: string }[];
  rows: { index: number; fingerprint: string; question: string; taskKind: string | null }[];
}

function reject(): never { throw new Error("Invalid saved dataset repair plan."); }
function object(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== keys.length || Object.keys(value).some(key => !keys.includes(key))) reject();
  return value as Record<string, unknown>;
}
function text(value: unknown, max = 2000): string { if (typeof value !== "string" || !value.trim() || [...value].length > max) reject(); return value; }
function integer(value: unknown, max = Number.MAX_SAFE_INTEGER): number { if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > max) reject(); return value; }
function rate(value: unknown): number { if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > 1) reject(); return value; }
function bool(value: unknown): boolean { if (typeof value !== "boolean") reject(); return value; }
function uuid(value: unknown): string { const result = text(value, 36); if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(result)) reject(); return result.toLowerCase(); }
function hash(value: unknown): string { const result = text(value, 71); if (!/^sha256:[0-9a-f]{64}$/.test(result)) reject(); return result; }
function array(value: unknown, max = 5000): unknown[] { if (!Array.isArray(value) || value.length > max) reject(); return value; }
function references(value: unknown): string[] { const result = array(value, 20).map(id => text(id, 128)); if (!result.length || new Set(result).size !== result.length) reject(); return result; }

export function parseDatasetRepairPlan(value: unknown): DatasetRepairPlan | null {
  if (value === null) return null;
  const hasCanary = !!value && typeof value === "object" && Object.hasOwn(value, "canary");
  const plan = object(value, ["decisionCallId", "proposalFingerprint", "maximumRowChanges", "proposal", "generation", "inputRows", "strategy", "publication", "evidence", ...(hasCanary ? ["canary"] : [])]);
  if (plan.strategy !== "question_variants_preserve_context") reject();
  const proposal = object(plan.proposal, ["summary", "stop", "removals", "additions"]);
  const result: DatasetRepairPlan = {
    decisionCallId: uuid(plan.decisionCallId), proposalFingerprint: hash(plan.proposalFingerprint), maximumRowChanges: integer(plan.maximumRowChanges, 5000),
    inputRows: integer(plan.inputRows), strategy: plan.strategy,
    proposal: { summary: text(proposal.summary, 400), stop: bool(proposal.stop),
      removals: array(proposal.removals).map(value => { const row = object(value, ["rowId", "reason", "evidenceIds"]); return { rowId: text(row.rowId, 128), reason: text(row.reason), evidenceIds: references(row.evidenceIds) }; }),
      additions: array(proposal.additions).map(value => { const target = object(value, ["templateRowId", "instruction", "count", "evidenceIds"]); const count = integer(target.count, 5000); if (!count) reject(); return { templateRowId: text(target.templateRowId, 128), instruction: text(target.instruction), count, evidenceIds: references(target.evidenceIds) }; }) },
    generation: array(plan.generation).map((value, index) => {
      const target = object(value, ["targetIndex", "requested", "admitted", "rejected", "unresolved", "attempts", "rejectionReasons"]);
      if (target.targetIndex !== index || !target.rejectionReasons || typeof target.rejectionReasons !== "object" || Array.isArray(target.rejectionReasons)) reject();
      const rejectionReasons = Object.fromEntries(Object.entries(target.rejectionReasons).map(([reason, count]) => { text(reason, 400); const amount = integer(count, 5000); if (!amount) reject(); return [reason, amount]; }));
      return { targetIndex: index, requested: integer(target.requested, 5000), admitted: integer(target.admitted, 5000), rejected: integer(target.rejected, 5000), unresolved: integer(target.unresolved, 5000), attempts: integer(target.attempts, 0xffff_ffff), rejectionReasons };
    }),
    publication: plan.publication === null ? null : (() => { const value = object(plan.publication, ["datasetVersionId", "added", "removed", "rows", "crossBatchDuplicates"]); return { datasetVersionId: uuid(value.datasetVersionId), added: integer(value.added, 5000), removed: integer(value.removed, 5000), rows: integer(value.rows), crossBatchDuplicates: integer(value.crossBatchDuplicates, 5000) }; })(),
    evidence: array(plan.evidence, 100000).map(value => {
      const item = object(value, ["id", "fingerprint", "label", "trainingRows", "totalTrainingRows", "overlapping", "development"]);
      return { id: text(item.id, 128), fingerprint: hash(item.fingerprint), label: text(item.label, 450),
        trainingRows: item.trainingRows === null ? null : integer(item.trainingRows), totalTrainingRows: item.totalTrainingRows === null ? null : integer(item.totalTrainingRows), overlapping: bool(item.overlapping),
        development: array(item.development, 20).map(value => { const metric = object(value, ["suite", "support", "recallAt1", "originalBaselineRecallAt1"]); return { suite: text(metric.suite, 200), support: integer(metric.support), recallAt1: rate(metric.recallAt1), originalBaselineRecallAt1: metric.originalBaselineRecallAt1 === null ? null : rate(metric.originalBaselineRecallAt1) }; }) };
    }),
  };
  const { removals, additions, stop } = result.proposal;
  const requested = removals.length + additions.reduce((sum, target) => sum + target.count, 0);
  if (!result.inputRows || requested > result.maximumRowChanges || stop !== (requested === 0) || removals.length > result.inputRows
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
    || result.publication.added + result.publication.crossBatchDuplicates !== result.generation.reduce((sum, target) => sum + target.admitted, 0)
    || result.publication.rows !== result.inputRows - result.publication.removed + result.publication.added)) reject();
  if (hasCanary) {
    const raw = object(plan.canary, ["policy", "status", "callId", "requested", "admitted", "rejected", "rows"]);
    if (raw.policy !== null && raw.policy !== "first_batch_all_admitted_v1" || !["disabled", "not_required", "pending", "interrupted", "passed", "rejected"].includes(String(raw.status))) reject();
    const canary: GenerationCanary = { policy: raw.policy as GenerationCanary["policy"], status: raw.status as GenerationCanary["status"], callId: raw.callId === null ? null : uuid(raw.callId), requested: integer(raw.requested, 8), admitted: integer(raw.admitted, 8),
      rejected: array(raw.rejected, 8).map(value => { const row = object(value, ["index", "reason"]); return { index: integer(row.index, 7), reason: text(row.reason, 400) }; }),
      rows: array(raw.rows, 8).map(value => { const row = object(value, ["index", "fingerprint", "question", "taskKind"]); return { index: integer(row.index, 7), fingerprint: hash(row.fingerprint), question: text(row.question, 8192), taskKind: row.taskKind === null ? null : text(row.taskKind, 200) }; }) };
    const indices = [...canary.rows, ...canary.rejected].map(row => row.index);
    const terminal = canary.status === "passed" || canary.status === "rejected";
    if ((canary.policy === null) !== (canary.status === "disabled") || canary.rows.length !== canary.admitted
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
  return result;
}
