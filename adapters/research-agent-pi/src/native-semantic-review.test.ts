import assert from "node:assert/strict";
import test from "node:test";

import type { Tool } from "@earendil-works/pi-ai";

import { PiResearchAgent } from "./agent.js";
import { nativeSemanticReviewModelTools } from "./native-semantic-review-tools.js";
import { configureProjectPayload } from "./project-provider.js";
import { PROTOCOL_VERSION, type PiRunRequest } from "./protocol.js";

function request(capabilitySet: PiRunRequest["capabilitySet"]): PiRunRequest {
  return {
    protocolVersion: PROTOCOL_VERSION,
    capabilitySet,
    runId: "00000000-0000-4000-8000-000000000001",
    runSpecificationFingerprint: `sha256:${"1".repeat(64)}`,
    provider: "fake",
    model: "scripted-reviewer",
    systemPrompt: "Review the exact native assessment request.",
    initialPrompt: JSON.stringify({ rows: [{ rowId: "row-1" }] }),
    maxModelTurns: 1,
  };
}

test("blind native review exposes one strict bounded result schema", async () => {
  const input = request("encoder_optimization_native_blind_v1");
  const arguments_ = {
    assessments: [
      {
        rowId: "row-1",
        rowFingerprint: `sha256:${"2".repeat(64)}`,
        requestFingerprint: `sha256:${"3".repeat(64)}`,
        supportedCandidateIds: ["read"],
        ambiguous: false,
        contextConsistent: true,
        issueCodes: [],
        rationale: "The question explicitly asks to retrieve the saved object.",
      },
    ],
  };
  input.scriptedTurns = [
    { toolCalls: [{ name: "submit_native_blind_assessments", arguments: arguments_ }] },
  ];
  const calls: unknown[] = [];
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call);
      return { content: { recorded: true }, terminate: true };
    },
  });

  await agent.run(input);

  assert.equal(calls.length, 1);
  assert.deepEqual((calls[0] as { arguments: unknown }).arguments, arguments_);
  const tools = nativeSemanticReviewModelTools([
    { name: "submit_native_blind_assessments" } as Tool,
  ]);
  const schema = JSON.parse(JSON.stringify(tools[0]?.parameters));
  assert.equal(schema.additionalProperties, false);
  assert.equal(schema.properties.assessments.maxItems, 8);
  assert.equal(schema.properties.assessments.items.additionalProperties, false);
  assert.deepEqual(schema.properties.assessments.items.required.sort(), [
    "ambiguous",
    "contextConsistent",
    "issueCodes",
    "rationale",
    "requestFingerprint",
    "rowFingerprint",
    "rowId",
    "supportedCandidateIds",
  ]);
});

test("target-fit native review forces its sole exact tool for project chat payloads", () => {
  const input = request("encoder_optimization_native_target_fit_v1");
  input.provider = "project-provider";
  input.openaiCompatible = { baseUrl: "https://provider.example/v1", maximumOutputTokens: 2048 };

  const configured = configureProjectPayload(input, { messages: [], tools: [] }) as Record<
    string,
    unknown
  >;

  assert.deepEqual(configured.tool_choice, {
    type: "function",
    function: { name: "submit_native_target_fit_assessments" },
  });
  const tools = nativeSemanticReviewModelTools([
    { name: "submit_native_target_fit_assessments" } as Tool,
  ]);
  const schema = JSON.parse(JSON.stringify(tools[0]?.parameters));
  assert.equal(schema.properties.assessments.maxItems, 8);
  assert.ok(schema.properties.assessments.items.required.includes("targetFits"));
  assert.ok(schema.properties.assessments.items.required.includes("blindAssessmentFingerprint"));
});

test("native review capability rejects unrelated adapter tools", async () => {
  const input = request("encoder_optimization_native_blind_v1");
  input.scriptedTurns = [{ toolCalls: [{ name: "propose_dataset_edits", arguments: {} }] }];
  const calls: unknown[] = [];
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call);
      return { content: {} };
    },
  });

  await agent.run(input);

  assert.deepEqual(calls, []);
});
