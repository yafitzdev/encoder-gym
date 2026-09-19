/** On-demand saved development samples, never sealed reports or fresh inference. */
export interface SavedCasePrediction {
  fingerprint: string; question: string | null; taskKind: string | null;
  expectedCapabilities: string[] | null; predictedCapabilities: string[] | null; expectedRank: number | null;
}
export type SavedCaseChange = "rank_improved" | "rank_regressed" | "rank_unchanged" | "not_comparable";
export interface SavedCasePair { sourceRowId: string; baseline: SavedCasePrediction | null; candidate: SavedCasePrediction | null; change: SavedCaseChange }
export interface SavedCaseSource { reportId: string; reportFingerprint: string; diagnosticsFingerprint: string | null; support: number; sampleSize: number | null }
export interface DevelopmentCaseComparison { suite: string; suiteFingerprint: string; sampleLimit: 50; baseline: SavedCaseSource; candidate: SavedCaseSource; cases: SavedCasePair[] }
export interface OptimizationCases {
  projectId: string; runId: string; iterationId: string; iteration: number;
  baselineModelId: string; candidateModelId: string; comparisons: DevelopmentCaseComparison[];
}

function invalid(): never { throw new Error("Invalid saved development-case comparison."); }
function object(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).length !== keys.length || Object.keys(value).some(key => !keys.includes(key))) invalid();
  return value as Record<string, unknown>;
}
function text(value: unknown, max: number): string { if (typeof value !== "string" || !value.trim() || [...value].length > max) invalid(); return value; }
function count(value: unknown, max = Number.MAX_SAFE_INTEGER): number { if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > max) invalid(); return value; }
function uuid(value: unknown): string { const id = text(value, 36); if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(id)) invalid(); return id.toLowerCase(); }
function hash(value: unknown): string { const result = text(value, 71); if (!/^sha256:[0-9a-f]{64}$/.test(result)) invalid(); return result; }
function list(value: unknown, max: number): unknown[] { if (!Array.isArray(value) || value.length > max) invalid(); return value; }
function capabilities(value: unknown): string[] | null {
  if (value === null) return null;
  const result = list(value, 128).map(item => text(item, 200)); if (new Set(result).size !== result.length) invalid(); return result;
}
function prediction(value: unknown): SavedCasePrediction | null {
  if (value === null) return null;
  const item = object(value, ["fingerprint", "question", "taskKind", "expectedCapabilities", "predictedCapabilities", "expectedRank"]);
  const rank = item.expectedRank === null ? null : count(item.expectedRank); if (rank === 0) invalid();
  return { fingerprint: hash(item.fingerprint), question: item.question === null ? null : text(item.question, 8192), taskKind: item.taskKind === null ? null : text(item.taskKind, 200),
    expectedCapabilities: capabilities(item.expectedCapabilities), predictedCapabilities: capabilities(item.predictedCapabilities), expectedRank: rank };
}
function source(value: unknown): SavedCaseSource {
  const item = object(value, ["reportId", "reportFingerprint", "diagnosticsFingerprint", "support", "sampleSize"]);
  if ((item.sampleSize === null) !== (item.diagnosticsFingerprint === null)) invalid();
  const result = { reportId: uuid(item.reportId), reportFingerprint: hash(item.reportFingerprint), diagnosticsFingerprint: item.diagnosticsFingerprint === null ? null : hash(item.diagnosticsFingerprint), support: count(item.support), sampleSize: item.sampleSize === null ? null : count(item.sampleSize, 50) };
  if (result.sampleSize !== null && result.sampleSize > result.support) invalid();
  return result;
}
export function caseChange(left: SavedCasePrediction | null, right: SavedCasePrediction | null): SavedCaseChange {
  if (!left || !right || left.question === null || left.question !== right.question || left.taskKind === null || left.taskKind !== right.taskKind
    || left.expectedCapabilities === null || right.expectedCapabilities === null || JSON.stringify([...left.expectedCapabilities].sort()) !== JSON.stringify([...right.expectedCapabilities].sort())
    || left.expectedRank === null || right.expectedRank === null) return "not_comparable";
  return right.expectedRank < left.expectedRank ? "rank_improved" : right.expectedRank > left.expectedRank ? "rank_regressed" : "rank_unchanged";
}
export function parseOptimizationCases(value: unknown, projectId: string, runId: string, iteration: number): OptimizationCases {
  const item = object(value, ["projectId", "runId", "iterationId", "iteration", "baselineModelId", "candidateModelId", "comparisons"]);
  const result: OptimizationCases = { projectId: uuid(item.projectId), runId: uuid(item.runId), iterationId: uuid(item.iterationId), iteration: count(item.iteration, 10),
    baselineModelId: uuid(item.baselineModelId), candidateModelId: uuid(item.candidateModelId), comparisons: list(item.comparisons, 20).map(value => {
      const suite = object(value, ["suite", "suiteFingerprint", "sampleLimit", "baseline", "candidate", "cases"]);
      if (suite.sampleLimit !== 50) invalid();
      const comparison: DevelopmentCaseComparison = { suite: text(suite.suite, 200), suiteFingerprint: hash(suite.suiteFingerprint), sampleLimit: 50, baseline: source(suite.baseline), candidate: source(suite.candidate), cases: list(suite.cases, 100).map(value => {
        const pair = object(value, ["sourceRowId", "baseline", "candidate", "change"]);
        const baseline = prediction(pair.baseline), candidate = prediction(pair.candidate), change = caseChange(baseline, candidate);
        if (!baseline && !candidate || pair.change !== change) invalid();
        return { sourceRowId: text(pair.sourceRowId, 200), baseline, candidate, change };
      }) };
      if (new Set(comparison.cases.map(pair => pair.sourceRowId)).size !== comparison.cases.length || comparison.baseline.support !== comparison.candidate.support
        || comparison.cases.filter(pair => pair.baseline).length !== (comparison.baseline.sampleSize ?? 0)
        || comparison.cases.filter(pair => pair.candidate).length !== (comparison.candidate.sampleSize ?? 0)) invalid();
      return comparison;
    }) };
  if (!result.iteration || result.projectId !== projectId || result.runId !== runId || result.iteration !== iteration || !result.comparisons.length
    || new Set(result.comparisons.map(suite => suite.suite)).size !== result.comparisons.length) invalid();
  return result;
}
