/** Development-only read model, produced by the CLI's verified custody readers. */
import { parseDatasetRepairPlan, type DatasetRepairPlan } from "./optimization-repair.js";
export interface OptimizationIteration {
  id: string; number: number; createdAt: string;
  startingModelId: string; inputDatasetVersionId: string; benchmarkVersionId: string;
  experimentRunId: string | null; qualifiedDatasetVersionId: string | null;
  trainingDatasetVersionId: string | null; modelId: string | null;
  completed: boolean; noChange: boolean; selected: boolean; developmentPassed: boolean | null;
  checks: { reportId: string; suite: string; metric: string; direction: "higher_is_better" | "lower_is_better"; baseline: number; candidate: number; passed: boolean }[];
  repairPlan?: DatasetRepairPlan | null;
}

function object(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)
    || Object.keys(value).length !== keys.length || Object.keys(value).some(key => !keys.includes(key))) throw new Error("Invalid iteration history.");
  return value as Record<string, unknown>;
}
function uuid(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) throw new Error("Invalid iteration identity.");
  return value.toLowerCase();
}
function bool(value: unknown): boolean { if (typeof value !== "boolean") throw new Error("Invalid iteration status."); return value; }
function text(value: unknown): string { if (typeof value !== "string" || !value || value.length > 200) throw new Error("Invalid iteration metric."); return value; }
function number(value: unknown): number { if (typeof value !== "number" || !Number.isFinite(value)) throw new Error("Invalid iteration score."); return value; }

export function parseOptimizationHistory(value: unknown, projectId: string, runId: string): OptimizationIteration[] {
  const history = object(value, ["projectId", "runId", "iterations"]);
  if (uuid(history.projectId) !== projectId || uuid(history.runId) !== runId || !Array.isArray(history.iterations)) throw new Error("Iteration history belongs to another run.");
  const iterations = history.iterations.map((value: unknown, index: number): OptimizationIteration => {
    const hasRepair = !!value && typeof value === "object" && Object.hasOwn(value, "repairPlan");
    const item = object(value, ["id", "number", "createdAt", "startingModelId", "inputDatasetVersionId", "benchmarkVersionId", "experimentRunId", "qualifiedDatasetVersionId", "trainingDatasetVersionId", "modelId", "completed", "noChange", "selected", "developmentPassed", "checks", ...(hasRepair ? ["repairPlan"] : [])]);
    if (item.number !== index + 1 || typeof item.createdAt !== "string" || !Number.isFinite(Date.parse(item.createdAt)) || !Array.isArray(item.checks)) throw new Error("Invalid iteration ordering.");
    const nullableId = (key: string): string | null => item[key] === null ? null : uuid(item[key]);
    const iteration: OptimizationIteration = {
      id: uuid(item.id), number: index + 1, createdAt: item.createdAt,
      startingModelId: uuid(item.startingModelId), inputDatasetVersionId: uuid(item.inputDatasetVersionId), benchmarkVersionId: uuid(item.benchmarkVersionId),
      experimentRunId: nullableId("experimentRunId"), qualifiedDatasetVersionId: nullableId("qualifiedDatasetVersionId"), trainingDatasetVersionId: nullableId("trainingDatasetVersionId"), modelId: nullableId("modelId"),
      completed: bool(item.completed), noChange: bool(item.noChange), selected: bool(item.selected), developmentPassed: item.developmentPassed === null ? null : bool(item.developmentPassed),
      checks: item.checks.map(value => {
        const check = object(value, ["reportId", "suite", "metric", "direction", "baseline", "candidate", "passed"]);
        if (check.direction !== "higher_is_better" && check.direction !== "lower_is_better") throw new Error("Invalid metric direction.");
        return { reportId: uuid(check.reportId), suite: text(check.suite), metric: text(check.metric), direction: check.direction, baseline: number(check.baseline), candidate: number(check.candidate), passed: bool(check.passed) };
      }),
      ...(hasRepair ? { repairPlan: parseDatasetRepairPlan(item.repairPlan) } : {}),
    };
    if ((iteration.experimentRunId === null) !== (iteration.trainingDatasetVersionId === null)
      || (iteration.experimentRunId === null) !== (iteration.qualifiedDatasetVersionId === null)
      || iteration.modelId !== null && iteration.experimentRunId === null
      || iteration.noChange && (!iteration.completed || iteration.developmentPassed !== null || iteration.modelId !== null)
      || iteration.developmentPassed !== null && (!iteration.completed || !iteration.modelId)
      || iteration.selected && iteration.developmentPassed !== true
      || iteration.repairPlan && (iteration.completed && iteration.noChange !== iteration.repairPlan.proposal.stop
        || iteration.qualifiedDatasetVersionId !== null && iteration.repairPlan.publication?.datasetVersionId !== iteration.qualifiedDatasetVersionId)) throw new Error("Inconsistent iteration custody.");
    return iteration;
  });
  if (new Set(iterations.map(item => item.id)).size !== iterations.length || iterations.filter(item => item.selected).length > 1) throw new Error("Conflicting iteration history.");
  return iterations;
}
