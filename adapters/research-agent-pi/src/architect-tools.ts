import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "typebox";

import type { ArchitectToolName, ToolExecutor } from "./protocol.js";

const confidence = Type.Union([Type.Literal("low"), Type.Literal("medium"), Type.Literal("high")]);

const schemas = {
  inspect_dataset: Type.Object({}),
  inspect_semantics: Type.Object({}),
  inspect_authenticity: Type.Object({}),
  inspect_coverage: Type.Object({}),
  inspect_development_evidence: Type.Object({}),
  preview_allocation: Type.Object({
    allocations: Type.Array(
      Type.Object({
        cell: Type.Object({
          label: Type.String({ minLength: 1 }),
          dimensions: Type.Record(Type.String(), Type.String({ minLength: 1 })),
        }),
        target: Type.Integer({ minimum: 0 }),
      }),
    ),
  }),
  estimate_cost: Type.Object({ additionalRows: Type.Integer({ minimum: 0 }) }),
  submit_proposal: Type.Object({
    summary: Type.String({ minLength: 1 }),
    allocations: Type.Array(Type.Record(Type.String(), Type.Unknown())),
    strategies: Type.Array(Type.Record(Type.String(), Type.Unknown())),
    tradeoffs: Type.Array(Type.String({ minLength: 1 })),
    uncertainties: Type.Array(Type.String({ minLength: 1 })),
  }),
  finish_architecture: Type.Object({
    reason: Type.Union([Type.Literal("proposal_submitted"), Type.Literal("budget_exhausted")]),
    summary: Type.String({ minLength: 1 }),
    confidence,
  }),
} as const;

const descriptions: Record<ArchitectToolName, string> = {
  inspect_dataset: "Inspect the pinned task, labels, dimensions, priorities, and total-row budget.",
  inspect_semantics: "Inspect the pinned semantic definitions available to this run.",
  inspect_authenticity: "Inspect the approved authenticity guidance available to this run.",
  inspect_coverage: "Inspect persisted accepted coverage and hard constraints for every cell.",
  inspect_development_evidence:
    "Inspect eligible aggregate development diagnostics; sealed evidence is never available.",
  preview_allocation:
    "Run an explicit candidate allocation through the deterministic allocator without persisting a plan.",
  estimate_cost: "Estimate request, token, and optional monetary usage from declared assumptions.",
  submit_proposal:
    "Submit one complete per-cell allocation and scoped generation-strategy proposal.",
  finish_architecture:
    "Finish only after a valid proposal was submitted or a hard budget was exhausted.",
};

export function createArchitectTools(runId: string, executor: ToolExecutor): AgentTool[] {
  return (Object.keys(schemas) as ArchitectToolName[]).map((name) => ({
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
        terminate: name === "finish_architecture" || (result.terminate ?? false),
      };
    },
  }));
}
