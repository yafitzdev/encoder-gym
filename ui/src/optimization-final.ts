/** Closed, row-free final-holdout boundary. No scores or native payloads. */
export interface FinalIdentity { id: string; fingerprint: string }
export interface AgentFinalScope {
  run: FinalIdentity; launch: FinalIdentity; executionHead: string; adaptiveClosedAt: string;
  completion: FinalIdentity; iteration: FinalIdentity; trainingBindingFingerprint: string;
  developmentResult: FinalIdentity; comparisonBaselineRevision: FinalIdentity; benchmark: FinalIdentity;
  scientificProject: FinalIdentity; protocol: FinalIdentity; candidate: FinalIdentity; model: FinalIdentity;
  dataset: FinalIdentity & { datasetId: string; projectId: string; number: number };
  finalSuite: string; baselineFinalReport: FinalIdentity; metricContractFingerprint: string;
  maximumEvaluationSeconds: number; fingerprint: string;
}
export interface AgentFinalAuthorization { id: string; scope: AgentFinalScope; authorizedBy: string; createdAt: string; fingerprint: string }
export interface AgentFinalDispatch { authorization: FinalIdentity; reportId: string; assessmentId: string; createdAt: string; fingerprint: string }
export interface AgentFinalReceipt {
  dispatch: FinalIdentity; report: FinalIdentity; assessment: FinalIdentity; accepted: boolean;
  reportCreatedAt: string; createdAt: string; fingerprint: string;
}
export interface AgentFinalView {
  state: "authorized" | "outcome_unknown" | "completed";
  execution: { authorization: AgentFinalAuthorization; dispatch: AgentFinalDispatch | null; result: AgentFinalReceipt | null };
}
export interface AgentFinalReview { token: string; scope: AgentFinalScope }
export interface AgentFinalPromotionRequest { baselineRevisionId: string; receiptFingerprint: string }

export function finalObject(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== keys.length
    || Object.keys(value).some(key => !keys.includes(key))) throw new Error("Invalid final-acceptance record.");
  return value as Record<string, unknown>;
}
export function finalUuid(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) throw new Error("Invalid final-acceptance identity.");
  return value.toLowerCase();
}
export function finalFingerprint(value: unknown): string {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) throw new Error("Invalid final-acceptance fingerprint.");
  return value;
}
function text(value: unknown): string {
  if (typeof value !== "string" || !value.trim() || value !== value.trim() || value.length > 240 || /[\u0000-\u001f]/.test(value)) throw new Error("Invalid final-acceptance text.");
  return value;
}
function timestamp(value: unknown): string { const result = text(value); if (!Number.isFinite(Date.parse(result))) throw new Error("Invalid final-acceptance time."); return result; }
function count(value: unknown): number { if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) throw new Error("Invalid final-acceptance limit."); return value; }
function identity(value: unknown): FinalIdentity { const item = finalObject(value, ["id", "fingerprint"]); return { id: finalUuid(item.id), fingerprint: finalFingerprint(item.fingerprint) }; }
function matches(a: FinalIdentity, b: FinalIdentity): boolean { return a.id === b.id && a.fingerprint === b.fingerprint; }

export function parseAgentFinalScope(value: unknown, projectId: string, runId: string): AgentFinalScope {
  const item = finalObject(value, ["run", "launch", "executionHead", "adaptiveClosedAt", "completion", "iteration", "trainingBindingFingerprint", "developmentResult", "comparisonBaselineRevision", "benchmark", "scientificProject", "protocol", "candidate", "model", "dataset", "finalSuite", "baselineFinalReport", "metricContractFingerprint", "maximumEvaluationSeconds", "fingerprint"]);
  const dataset = finalObject(item.dataset, ["id", "datasetId", "projectId", "number", "fingerprint"]);
  const result: AgentFinalScope = {
    run: identity(item.run), launch: identity(item.launch), executionHead: finalFingerprint(item.executionHead), adaptiveClosedAt: timestamp(item.adaptiveClosedAt),
    completion: identity(item.completion), iteration: identity(item.iteration), trainingBindingFingerprint: finalFingerprint(item.trainingBindingFingerprint),
    developmentResult: identity(item.developmentResult), comparisonBaselineRevision: identity(item.comparisonBaselineRevision), benchmark: identity(item.benchmark),
    scientificProject: identity(item.scientificProject), protocol: identity(item.protocol), candidate: identity(item.candidate), model: identity(item.model),
    dataset: { id: finalUuid(dataset.id), datasetId: finalUuid(dataset.datasetId), projectId: finalUuid(dataset.projectId), number: count(dataset.number), fingerprint: finalFingerprint(dataset.fingerprint) },
    finalSuite: text(item.finalSuite), baselineFinalReport: identity(item.baselineFinalReport), metricContractFingerprint: finalFingerprint(item.metricContractFingerprint),
    maximumEvaluationSeconds: count(item.maximumEvaluationSeconds), fingerprint: finalFingerprint(item.fingerprint),
  };
  if (result.run.id !== runId || result.dataset.projectId !== projectId) throw new Error("Final acceptance belongs to another project or run.");
  return result;
}
export function parseAgentFinalAuthorization(value: unknown, projectId: string, runId: string): AgentFinalAuthorization {
  const item = finalObject(value, ["id", "scope", "authorizedBy", "createdAt", "fingerprint"]);
  const result = { id: finalUuid(item.id), scope: parseAgentFinalScope(item.scope, projectId, runId), authorizedBy: text(item.authorizedBy), createdAt: timestamp(item.createdAt), fingerprint: finalFingerprint(item.fingerprint) };
  if (Date.parse(result.createdAt) < Date.parse(result.scope.adaptiveClosedAt)) throw new Error("Final consent precedes adaptive completion.");
  return result;
}
export function parseAgentFinalView(value: unknown, projectId: string, runId: string): AgentFinalView | null {
  if (value === null) return null;
  const item = finalObject(value, ["state", "execution"]), execution = finalObject(item.execution, ["authorization", "dispatch", "result"]);
  const authorization = parseAgentFinalAuthorization(execution.authorization, projectId, runId);
  let dispatch: AgentFinalDispatch | null = null, result: AgentFinalReceipt | null = null;
  if (execution.dispatch !== null) {
    const raw = finalObject(execution.dispatch, ["authorization", "reportId", "assessmentId", "createdAt", "fingerprint"]);
    dispatch = { authorization: identity(raw.authorization), reportId: finalUuid(raw.reportId), assessmentId: finalUuid(raw.assessmentId), createdAt: timestamp(raw.createdAt), fingerprint: finalFingerprint(raw.fingerprint) };
    if (!matches(dispatch.authorization, authorization) || dispatch.reportId === dispatch.assessmentId || Date.parse(dispatch.createdAt) < Date.parse(authorization.createdAt)) throw new Error("Final dispatch differs from consent.");
  }
  if (execution.result !== null) {
    const raw = finalObject(execution.result, ["dispatch", "report", "assessment", "accepted", "reportCreatedAt", "createdAt", "fingerprint"]);
    if (typeof raw.accepted !== "boolean") throw new Error("Invalid final-acceptance decision.");
    result = { dispatch: identity(raw.dispatch), report: identity(raw.report), assessment: identity(raw.assessment), accepted: raw.accepted, reportCreatedAt: timestamp(raw.reportCreatedAt), createdAt: timestamp(raw.createdAt), fingerprint: finalFingerprint(raw.fingerprint) };
    if (!dispatch || !matches(result.dispatch, { id: authorization.id, fingerprint: dispatch.fingerprint }) || result.report.id !== dispatch.reportId || result.assessment.id !== dispatch.assessmentId
      || Date.parse(result.reportCreatedAt) < Date.parse(dispatch.createdAt) || Date.parse(result.createdAt) < Date.parse(result.reportCreatedAt)) throw new Error("Final result differs from its reserved evidence.");
  }
  const state = result ? "completed" : dispatch ? "outcome_unknown" : "authorized";
  if (item.state !== state) throw new Error("Final-acceptance state differs from persisted custody.");
  return { state, execution: { authorization, dispatch, result } };
}
