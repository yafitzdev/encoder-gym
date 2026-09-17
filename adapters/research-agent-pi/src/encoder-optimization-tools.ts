import type { AgentTool } from "@earendil-works/pi-agent-core";
import type { Tool } from "@earendil-works/pi-ai";
import { Type } from "typebox";

import type { EncoderOptimizationToolName, ToolExecutor } from "./protocol.js";

const identity = Type.String({ minLength: 1, maxLength: 128 });
const explanation = Type.String({ minLength: 1, maxLength: 2000 });
const schemas = {
  inspect_development_failures: Type.Object({
    offset: Type.Integer({ minimum: 0 }),
    limit: Type.Integer({ minimum: 1, maximum: 20 }),
  }),
  inspect_training_rows: Type.Object({
    offset: Type.Integer({ minimum: 0 }),
    limit: Type.Integer({ minimum: 1, maximum: 20 }),
    query: Type.Optional(Type.String({ maxLength: 200 })),
  }),
  propose_dataset_edits: Type.Object({
    summary: Type.String({ minLength: 1, maxLength: 400 }),
    stop: Type.Boolean(),
    removals: Type.Array(
      Type.Object({ rowId: identity, reason: explanation, evidenceIds: Type.Array(identity) }),
      { maxItems: 5000 },
    ),
    additions: Type.Array(
      Type.Object({
        templateRowId: identity,
        instruction: explanation,
        count: Type.Integer({ minimum: 1, maximum: 5000 }),
        evidenceIds: Type.Array(identity),
      }),
      { maxItems: 5000 },
    ),
  }),
};

const descriptions: Record<EncoderOptimizationToolName, string> = {
  inspect_development_failures:
    "Inspect persisted development failures for the pinned benchmark. Pages are byte-bounded and may return fewer items than limit; nextOffset identifies the next complete item. No sealed evidence is accessible.",
  inspect_training_rows:
    "Inspect a bounded page of the exact starting training dataset; optionally filter its task-visible text. Pages may return fewer items than limit to bound context; only returned items have been inspected, and nextOffset identifies the next complete item.",
  propose_dataset_edits:
    "Submit evidence-linked removals and targeted generation instructions, or stop without changes. Keep summary within 400 characters. Reference only inspected row and evidence IDs. The host validates every proposal; this tool cannot train, approve or change a benchmark.",
};

export function encoderOptimizationModelTools(tools: Tool[]): Tool[] {
  return tools.map((tool) => {
    if (!Object.hasOwn(schemas, tool.name)) {
      throw new Error("Unexpected encoder optimization tool");
    }
    return { ...tool, parameters: schemas[tool.name as EncoderOptimizationToolName] };
  });
}

export function createEncoderOptimizationTools(
  runId: string,
  executor: ToolExecutor,
  proposalOnly = false,
): AgentTool[] {
  const names: EncoderOptimizationToolName[] = proposalOnly
    ? ["propose_dataset_edits"]
    : (Object.keys(schemas) as EncoderOptimizationToolName[]);
  return names.map((name) => ({
    name,
    label: name,
    description: descriptions[name],
    // Pi validates/coerces before calling execute. Keep its transport schema
    // permissive so every attempt reaches the authoritative Rust validator and
    // append-only trace unchanged. The model receives the strict schema above.
    parameters: Type.Unknown(),
    executionMode: "sequential",
    async execute(callId, parameters, signal) {
      const result = await executor.execute({ runId, callId, name, arguments: parameters }, signal);
      return {
        content: [{ type: "text", text: JSON.stringify(result.content) }],
        details: result.details ?? {},
        terminate: result.terminate ?? false,
      };
    },
  }));
}
