import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "typebox";

import type { SupervisorToolName, ToolExecutor } from "./protocol.js";

const expectedImprovement = Type.Object({
  metric: Type.String({ minLength: 1 }),
  minimum_delta_basis_points: Type.Integer({ minimum: 0, maximum: 10_000 }),
});

const diagnosisCause = Type.Union([
  Type.Literal("prompt_guidance"),
  Type.Literal("semantic_conflict"),
  Type.Literal("dimension_strategy_conflict"),
  Type.Literal("generator_inadequacy"),
  Type.Literal("evaluator_disagreement"),
  Type.Literal("insufficient_evidence"),
  Type.Literal("repetition_mode_collapse"),
  Type.Literal("authenticity_failure"),
  Type.Literal("strategy_failure"),
  Type.Literal("not_safely_repairable"),
]);

const replacement = {
  replacement_guidance: Type.Array(Type.String({ minLength: 1 })),
  expected_improvements: Type.Array(expectedImprovement),
} as const;

const schemas = {
  inspect_quality_contract: Type.Object({}),
  inspect_quality_window: Type.Object({}),
  inspect_failure_breakdown: Type.Object({}),
  inspect_current_prompt_guidance: Type.Object({}),
  preview_prompt_revision: Type.Object(replacement),
  submit_prompt_revision: Type.Object({
    cause: diagnosisCause,
    summary: Type.String({ minLength: 1 }),
    ...replacement,
  }),
  finish_supervision: Type.Object({
    outcome: Type.Union([Type.Literal("revision_submitted"), Type.Literal("escalate")]),
    summary: Type.String({ minLength: 1 }),
    cause: Type.Optional(diagnosisCause),
  }),
} as const;

const descriptions: Record<SupervisorToolName, string> = {
  inspect_quality_contract:
    "Inspect immutable thresholds, budgets, revision limits, and evaluator independence.",
  inspect_quality_window: "Inspect the exact aggregate quality window that triggered the pause.",
  inspect_failure_breakdown:
    "Inspect aggregate failed criteria and issue-code counts for the paused scope.",
  inspect_current_prompt_guidance:
    "Inspect only the replaceable guidance and protected-field fingerprints.",
  preview_prompt_revision:
    "Validate one bounded guidance-only repair and its measurable expected improvements.",
  submit_prompt_revision:
    "Persist one diagnosis and bounded guidance-only revision for operator authorization.",
  finish_supervision:
    "Finish after submitting a valid revision or explicitly escalating an unsafe repair.",
};

export function createSupervisorTools(runId: string, executor: ToolExecutor): AgentTool[] {
  return (Object.keys(schemas) as SupervisorToolName[]).map((name) => ({
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
        terminate: name === "finish_supervision" || (result.terminate ?? false),
      };
    },
  }));
}
