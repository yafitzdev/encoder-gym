import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "typebox";

import type { BenchmarkArchitectToolName, ToolExecutor } from "./protocol.js";

const empty = Type.Object({});
const confidence = Type.Union([Type.Literal("low"), Type.Literal("medium"), Type.Literal("high")]);

const schemas = {
  inspect_brief: empty,
  inspect_existing_benchmark: empty,
  inspect_exposure_history: empty,
  search_web: Type.Object({
    query: Type.String({ minLength: 1 }),
    sourceClasses: Type.Array(Type.String({ minLength: 1 })),
    maximumResults: Type.Integer({ minimum: 1 }),
  }),
  fetch_page: Type.Object({
    url: Type.String({ minLength: 1 }),
    maximumBytes: Type.Integer({ minimum: 1 }),
  }),
  record_evidence: Type.Object({
    key: Type.String({ minLength: 1 }),
    url: Type.String({ minLength: 1 }),
    title: Type.String({ minLength: 1 }),
    query: Type.String({ minLength: 1 }),
    sourceClass: Type.String({ minLength: 1 }),
    contentHash: Type.String({ minLength: 1 }),
    excerpt: Type.String({ minLength: 1 }),
    location: Type.Optional(Type.String({ minLength: 1 })),
    observation: Type.String({ minLength: 1 }),
    applicability: Type.String({ minLength: 1 }),
    confidence,
  }),
  inspect_evidence: empty,
  preview_blueprint: Type.Object({
    blueprint: Type.Record(Type.String(), Type.Unknown()),
    evidenceBindings: Type.Array(Type.Record(Type.String(), Type.Unknown())),
  }),
  submit_blueprint: Type.Object({
    blueprint: Type.Record(Type.String(), Type.Unknown()),
    evidenceBindings: Type.Array(Type.Record(Type.String(), Type.Unknown())),
  }),
  finish_benchmark_architecture: Type.Object({
    reason: Type.Union([Type.Literal("proposal_submitted"), Type.Literal("budget_exhausted")]),
    summary: Type.String({ minLength: 1 }),
  }),
} as const;

const descriptions: Record<BenchmarkArchitectToolName, string> = {
  inspect_brief:
    "Inspect task semantics, deployment risks, objectives, and row-free candidate summaries.",
  inspect_existing_benchmark:
    "Inspect immutable current-benchmark and qualification pins plus aggregate cohort summaries.",
  inspect_exposure_history:
    "Inspect aggregate exposure risk and retirement state; no rows or predictions are available.",
  search_web:
    "Search permitted sources for real-world failure modes and benchmark-design evidence.",
  fetch_page: "Fetch one permitted page as explicitly delimited untrusted evidence.",
  record_evidence: "Record a bounded source excerpt and its benchmark-design applicability.",
  inspect_evidence: "Inspect the source evidence already recorded in this run.",
  preview_blueprint:
    "Run an explicit blueprint through deterministic contract, support, disclosure, and freshness checks.",
  submit_blueprint: "Submit one complete deterministically valid benchmark blueprint for review.",
  finish_benchmark_architecture:
    "Finish only after a valid blueprint was submitted or budget ended.",
};

export function createBenchmarkArchitectTools(runId: string, executor: ToolExecutor): AgentTool[] {
  return (Object.keys(schemas) as BenchmarkArchitectToolName[]).map((name) => ({
    name,
    label: name,
    description: descriptions[name],
    parameters: schemas[name],
    executionMode: "sequential",
    async execute(callId, parameters, signal) {
      const result = await executor.execute({ runId, callId, name, arguments: parameters }, signal);
      return {
        content: [{ type: "text", text: JSON.stringify(result.content) }],
        details: result.details ?? {},
        terminate: name === "finish_benchmark_architecture" || (result.terminate ?? false),
      };
    },
  }));
}
