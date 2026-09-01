import assert from "node:assert/strict";
import test from "node:test";

import { PiResearchAgent } from "./agent.js";
import { PROTOCOL_VERSION, type PiRunEvent, type PiRunRequest } from "./protocol.js";

function request(): PiRunRequest {
  return {
    protocolVersion: PROTOCOL_VERSION,
    capabilitySet: "authenticity_research_v1",
    runId: "run-1",
    runSpecificationFingerprint: "sha256:run",
    provider: "fake",
    model: "scripted-research",
    systemPrompt: "Study authentic support-chat style.",
    initialPrompt: "Begin the bounded research run.",
    maxModelTurns: 4,
    scriptedTurns: [
      {
        text: "I will gather evidence first.",
        toolCalls: [
          {
            name: "search_web",
            arguments: {
              query: "real support chat language",
              maximumResults: 3,
            },
          },
        ],
      },
      {
        text: "The first result reveals a gap, so I will inspect it.",
        toolCalls: [
          {
            name: "fetch_page",
            arguments: { url: "https://example.test/support", maximumBytes: 10000 },
          },
        ],
      },
      {
        toolCalls: [
          {
            name: "draft_profile",
            arguments: { claims: [], profile: { summary: "Messages use short fragments." } },
          },
        ],
      },
      {
        toolCalls: [
          {
            name: "finish_research",
            arguments: { reason: "sufficient_evidence", summary: "Profile drafted." },
          },
        ],
      },
    ],
  };
}

test("Pi executes an iterative research loop through only the six owned tools", async () => {
  const calls: string[] = [];
  const events: PiRunEvent[] = [];
  const agent = new PiResearchAgent(
    {
      async execute(call) {
        calls.push(call.name);
        return { content: { accepted: true } };
      },
    },
    (event) => {
      events.push(event);
    },
  );

  await agent.run(request());

  assert.deepEqual(calls, ["search_web", "fetch_page", "draft_profile", "finish_research"]);
  assert.equal(events.filter((event) => event.type === "turn_completed").length, 4);
  assert.equal(events.at(-1)?.type, "agent_finished");
});

test("the adapter refuses scripts for a real provider", async () => {
  const input = request();
  input.provider = "openai";
  const agent = new PiResearchAgent({
    async execute() {
      return { content: {} };
    },
  });
  await assert.rejects(agent.run(input), /allowed only with the fake provider/);
});

test("a hard turn ceiling stops a script before later tools", async () => {
  const calls: string[] = [];
  const input = request();
  input.maxModelTurns = 2;
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call.name);
      return { content: {} };
    },
  });
  await agent.run(input);
  assert.deepEqual(calls, ["search_web", "fetch_page"]);
});

test("cancellation aborts an in-flight host tool", async () => {
  const input = request();
  input.maxModelTurns = 1;
  input.scriptedTurns = input.scriptedTurns?.slice(0, 1) ?? [];
  let sawAbort = false;
  let agentReportedAbort = false;
  let agent: PiResearchAgent;
  agent = new PiResearchAgent(
    {
      execute(_call, signal) {
        return new Promise((_resolve, reject) => {
          signal?.addEventListener(
            "abort",
            () => {
              sawAbort = true;
              reject(new Error("host tool aborted"));
            },
            { once: true },
          );
          queueMicrotask(() => agent.cancel(input.runId));
        });
      },
    },
    (event) => {
      if (event.type === "agent_finished") agentReportedAbort = event.aborted;
    },
  );

  await agent.run(input);
  assert.equal(sawAbort, true);
  assert.equal(agentReportedAbort, true);
});

test("the Dataset Architect iterates through only its application-owned tools", async () => {
  const calls: string[] = [];
  const input = request();
  input.capabilitySet = "dataset_architect_v1";
  input.maxModelTurns = 4;
  input.scriptedTurns = [
    {
      text: "I will inspect the declared planning space first.",
      toolCalls: [{ name: "inspect_dataset", arguments: {} }],
    },
    {
      text: "I will validate a candidate rather than assume it is feasible.",
      toolCalls: [{ name: "preview_allocation", arguments: { allocations: [] } }],
    },
    {
      toolCalls: [
        {
          name: "submit_proposal",
          arguments: {
            summary: "Candidate",
            allocations: [],
            strategies: [],
            tradeoffs: [],
            uncertainties: [],
          },
        },
      ],
    },
    {
      toolCalls: [
        {
          name: "finish_architecture",
          arguments: {
            reason: "proposal_submitted",
            summary: "Proposal submitted.",
            confidence: "medium",
          },
        },
      ],
    },
  ];
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call.name);
      return { content: { accepted: true } };
    },
  });

  await agent.run(input);

  assert.deepEqual(calls, [
    "inspect_dataset",
    "preview_allocation",
    "submit_proposal",
    "finish_architecture",
  ]);
});

test("the Benchmark Architect exposes only row-free research and blueprint tools", async () => {
  const calls: string[] = [];
  const input = request();
  input.capabilitySet = "benchmark_architect_v1";
  input.maxModelTurns = 1;
  input.scriptedTurns = [
    {
      toolCalls: [
        { name: "inspect_brief", arguments: {} },
        { name: "inspect_existing_benchmark", arguments: {} },
        { name: "inspect_exposure_history", arguments: {} },
        {
          name: "search_web",
          arguments: {
            query: "real support classification failures",
            sourceClasses: [],
            maximumResults: 3,
          },
        },
        { name: "fetch_page", arguments: { url: "https://example.com/study", maximumBytes: 1000 } },
        {
          name: "record_evidence",
          arguments: {
            key: "risk-study",
            url: "https://example.com/study",
            title: "Study",
            query: "real support classification failures",
            sourceClass: "study",
            contentHash: "sha256:page",
            excerpt: "Observed failure",
            observation: "Boundary language overlaps.",
            applicability: "Create a boundary cohort.",
            confidence: "medium",
          },
        },
        { name: "inspect_evidence", arguments: {} },
        { name: "preview_blueprint", arguments: { blueprint: {}, evidenceBindings: [] } },
        { name: "submit_blueprint", arguments: { blueprint: {}, evidenceBindings: [] } },
        {
          name: "finish_benchmark_architecture",
          arguments: { reason: "proposal_submitted", summary: "Submitted." },
        },
      ],
    },
  ];
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call.name);
      return { content: { accepted: true } };
    },
  });

  await agent.run(input);

  assert.deepEqual(calls, [
    "inspect_brief",
    "inspect_existing_benchmark",
    "inspect_exposure_history",
    "search_web",
    "fetch_page",
    "record_evidence",
    "inspect_evidence",
    "preview_blueprint",
    "submit_blueprint",
    "finish_benchmark_architecture",
  ]);
});

test("the generation supervisor exposes exactly its seven bounded tools", async () => {
  const calls: string[] = [];
  const input = request();
  input.capabilitySet = "generation_quality_supervisor_v1";
  input.maxModelTurns = 1;
  input.scriptedTurns = [
    {
      toolCalls: [
        { name: "inspect_quality_contract", arguments: {} },
        { name: "inspect_quality_window", arguments: {} },
        { name: "inspect_failure_breakdown", arguments: {} },
        { name: "inspect_current_prompt_guidance", arguments: {} },
        {
          name: "preview_prompt_revision",
          arguments: {
            replacement_guidance: ["Add concrete situational detail."],
            expected_improvements: [{ metric: "qualified_rate", minimum_delta_basis_points: 1000 }],
          },
        },
        {
          name: "submit_prompt_revision",
          arguments: {
            cause: "repetition_mode_collapse",
            summary: "Rows repeat one synthetic shortcut.",
            replacement_guidance: ["Add concrete situational detail."],
            expected_improvements: [{ metric: "qualified_rate", minimum_delta_basis_points: 1000 }],
          },
        },
        {
          name: "finish_supervision",
          arguments: { outcome: "revision_submitted", summary: "Repair submitted." },
        },
      ],
    },
  ];
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call.name);
      return { content: { accepted: true } };
    },
  });

  await agent.run(input);

  assert.deepEqual(calls, [
    "inspect_quality_contract",
    "inspect_quality_window",
    "inspect_failure_breakdown",
    "inspect_current_prompt_guidance",
    "preview_prompt_revision",
    "submit_prompt_revision",
    "finish_supervision",
  ]);
});
