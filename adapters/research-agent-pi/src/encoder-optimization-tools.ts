import type { AgentTool } from "@earendil-works/pi-agent-core";
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
    "Inspect persisted development failures for the pinned benchmark. No sealed evidence is accessible.",
  inspect_training_rows:
    "Inspect a bounded page of the exact starting training dataset; optionally filter its task-visible text.",
  propose_dataset_edits:
    "Submit evidence-linked removals and targeted generation instructions, or stop without changes. Reference only inspected row and evidence IDs. The host validates every proposal; this tool cannot train, approve or change a benchmark.",
};

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
    parameters: schemas[name],
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
