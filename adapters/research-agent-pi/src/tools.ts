import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "typebox";

import type { ResearchToolName, ToolExecutor } from "./protocol.js";

const confidence = Type.Union([Type.Literal("low"), Type.Literal("medium"), Type.Literal("high")]);

const schemas = {
  search_web: Type.Object({
    query: Type.String({ minLength: 1 }),
    sourceClasses: Type.Optional(Type.Array(Type.String({ minLength: 1 }))),
    maximumResults: Type.Integer({ minimum: 1, maximum: 100 }),
  }),
  fetch_page: Type.Object({
    url: Type.String({ minLength: 1 }),
    maximumBytes: Type.Integer({ minimum: 1 }),
  }),
  record_evidence: Type.Object({
    url: Type.String({ minLength: 1 }),
    title: Type.String({ minLength: 1 }),
    query: Type.String({ minLength: 1 }),
    sourceClass: Type.String({ minLength: 1 }),
    contentHash: Type.String({ minLength: 1 }),
    excerpt: Type.String({ minLength: 1, maxLength: 2000 }),
    location: Type.Optional(Type.String({ minLength: 1 })),
    observation: Type.String({ minLength: 1 }),
    applicability: Type.String({ minLength: 1 }),
    confidence,
  }),
  inspect_evidence: Type.Object({
    sourceClass: Type.Optional(Type.String({ minLength: 1 })),
    confidence: Type.Optional(confidence),
  }),
  draft_profile: Type.Object({
    profile: Type.Record(Type.String(), Type.Unknown()),
  }),
  finish_research: Type.Object({
    reason: Type.Union([Type.Literal("sufficient_evidence"), Type.Literal("budget_exhausted")]),
    summary: Type.String({ minLength: 1 }),
  }),
} as const;

const descriptions: Record<ResearchToolName, string> = {
  search_web: "Search permitted sources for evidence relevant to the research brief.",
  fetch_page: "Fetch one permitted search result as explicitly untrusted source content.",
  record_evidence: "Persist a bounded excerpt and derived observation with provenance.",
  inspect_evidence: "Inspect already-persisted evidence to identify gaps and conflicts.",
  draft_profile: "Submit the structured evidence-backed authenticity profile draft.",
  finish_research: "Finish after the profile is drafted or a hard research budget is exhausted.",
};

function tool(
  runId: string,
  name: ResearchToolName,
  schema: (typeof schemas)[ResearchToolName],
  executor: ToolExecutor,
): AgentTool {
  return {
    name,
    label: name,
    description: descriptions[name],
    parameters: schema,
    executionMode: "sequential",
    async execute(callId, parameters, signal) {
      const result = await executor.execute({ runId, callId, name, arguments: parameters }, signal);
      return {
        content: [{ type: "text", text: JSON.stringify(result.content) }],
        details: result.details ?? {},
        terminate: name === "finish_research" || (result.terminate ?? false),
      };
    },
  };
}

export function createResearchTools(runId: string, executor: ToolExecutor): AgentTool[] {
  return (Object.keys(schemas) as ResearchToolName[]).map((name) =>
    tool(runId, name, schemas[name], executor),
  );
}
