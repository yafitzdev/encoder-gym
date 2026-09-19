import type { AgentTool } from "@earendil-works/pi-agent-core";
import type { Tool } from "@earendil-works/pi-ai";
import { Type } from "typebox";

import type { NativeSemanticReviewToolName, ToolExecutor } from "./protocol.js";

const identity = Type.String({ minLength: 1, maxLength: 128 });
const fingerprint = Type.String({ pattern: "^sha256:[0-9a-f]{64}$" });
const issueCode = Type.Union([
  Type.Literal("no_supported_candidate"),
  Type.Literal("ambiguous_question"),
  Type.Literal("context_inconsistent"),
  Type.Literal("candidate_semantics_insufficient"),
  Type.Literal("unsupported_question"),
  Type.Literal("target_mismatch"),
  Type.Literal("other"),
]);
const issues = Type.Array(issueCode, { maxItems: 8, uniqueItems: true });
const rationale = Type.String({ minLength: 1, maxLength: 1_000 });

const schemas: Record<NativeSemanticReviewToolName, ReturnType<typeof Type.Object>> = {
  submit_native_blind_assessments: Type.Object(
    {
      assessments: Type.Array(
        Type.Object(
          {
            rowId: identity,
            rowFingerprint: fingerprint,
            requestFingerprint: fingerprint,
            supportedCandidateIds: Type.Array(identity, {
              maxItems: 128,
              uniqueItems: true,
            }),
            ambiguous: Type.Boolean(),
            contextConsistent: Type.Boolean(),
            issueCodes: issues,
            rationale,
          },
          { additionalProperties: false },
        ),
        { minItems: 1, maxItems: 8 },
      ),
    },
    { additionalProperties: false },
  ),
  submit_native_target_fit_assessments: Type.Object(
    {
      assessments: Type.Array(
        Type.Object(
          {
            rowId: identity,
            rowFingerprint: fingerprint,
            requestFingerprint: fingerprint,
            blindAssessmentFingerprint: fingerprint,
            targetFits: Type.Boolean(),
            issueCodes: issues,
            rationale,
          },
          { additionalProperties: false },
        ),
        { minItems: 1, maxItems: 8 },
      ),
    },
    { additionalProperties: false },
  ),
};

const descriptions: Record<NativeSemanticReviewToolName, string> = {
  submit_native_blind_assessments:
    "Submit one label-blind semantic assessment for every supplied row. Select only legal candidate IDs from that row, flag ambiguity and context inconsistency explicitly, and copy the exact row and request fingerprints. This tool cannot admit rows.",
  submit_native_target_fit_assessments:
    "Submit one target-fit assessment for every supplied row, independent of inherited labels. Decide whether the question fits its stated repair target and copy the exact row, request and blind-assessment fingerprints. This tool cannot admit rows.",
};

export function nativeSemanticReviewModelTools(tools: Tool[]): Tool[] {
  return tools.map((tool) => {
    if (!Object.hasOwn(schemas, tool.name)) {
      throw new Error("Unexpected native semantic review tool");
    }
    return {
      ...tool,
      parameters: schemas[tool.name as NativeSemanticReviewToolName],
    };
  });
}

export function createNativeSemanticReviewTools(
  runId: string,
  executor: ToolExecutor,
  pass: "blind" | "target_fit",
): AgentTool[] {
  const name: NativeSemanticReviewToolName =
    pass === "blind" ? "submit_native_blind_assessments" : "submit_native_target_fit_assessments";
  return [
    {
      name,
      label: name,
      description: descriptions[name],
      // The host owns authoritative normalization and durable evidence. Keep
      // transport permissive so malformed model output reaches that boundary.
      parameters: Type.Unknown(),
      executionMode: "sequential",
      async execute(callId, parameters, signal) {
        const result = await executor.execute(
          { runId, callId, name, arguments: parameters },
          signal,
        );
        return {
          content: [{ type: "text", text: JSON.stringify(result.content) }],
          details: result.details ?? {},
          terminate: result.terminate ?? false,
        };
      },
    },
  ];
}
