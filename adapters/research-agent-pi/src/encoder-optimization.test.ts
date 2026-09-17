import assert from "node:assert/strict";
import { createServer } from "node:http";
import test from "node:test";

import { PiResearchAgent } from "./agent.js";
import { createEncoderOptimizationTools } from "./encoder-optimization-tools.js";
import { configureProjectPayload, projectProvider } from "./project-provider.js";
import type { PiRunEvent, PiRunRequest } from "./protocol.js";

function request(baseUrl: string): PiRunRequest {
  return {
    protocolVersion: 1,
    capabilitySet: "encoder_optimization_v1",
    runId: "test-run",
    runSpecificationFingerprint: "test-authority",
    provider: "project-connection",
    model: "key-visible-custom-model",
    systemPrompt: "Inspect the supplied training dataset.",
    initialPrompt: "Analyze this development result.",
    maxModelTurns: 1,
    openaiCompatible: { baseUrl, maximumOutputTokens: 256 },
  };
}

test("selected project endpoint and model execute real Pi tool calls without catalog substitution", async () => {
  const requests: Record<string, unknown>[] = [];
  const server = createServer(async (incoming, outgoing) => {
    assert.equal(incoming.url, "/v1/chat/completions");
    assert.equal(incoming.headers.authorization, "Bearer fixture-not-a-secret");
    const chunks: Buffer[] = [];
    for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
    requests.push(JSON.parse(Buffer.concat(chunks).toString()));
    outgoing.writeHead(200, { "content-type": "text/event-stream" });
    const chunk = (delta: unknown, finish: string | null = null) =>
      `data: ${JSON.stringify({ id: "response-1", object: "chat.completion.chunk", model: "key-visible-custom-model", choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;
    outgoing.write(
      chunk({ role: "assistant", content: "I will inspect the failed development cases first." }),
    );
    outgoing.write(
      chunk({
        tool_calls: [
          {
            index: 0,
            id: "tool-1",
            type: "function",
            function: { name: "inspect_development_failures", arguments: '{"offset":0,"limit":5}' },
          },
        ],
      }),
    );
    outgoing.write(chunk({}, "tool_calls"));
    outgoing.write(
      `data: ${JSON.stringify({ choices: [], usage: { prompt_tokens: 35, completion_tokens: 12, total_tokens: 47 } })}\n\n`,
    );
    outgoing.end("data: [DONE]\n\n");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  const input = request(`http://127.0.0.1:${address.port}/v1`);
  const events: PiRunEvent[] = [];
  const tools: string[] = [];
  process.env.ENCODER_OPTIMIZATION_FIXTURE_KEY = "fixture-not-a-secret";
  input.apiKeyEnv = "ENCODER_OPTIMIZATION_FIXTURE_KEY";
  const agent = new PiResearchAgent(
    {
      async execute(call) {
        tools.push(call.name);
        return { content: { failures: [] } };
      },
    },
    (event) => {
      events.push(event);
    },
  );
  try {
    await agent.run(input);
    assert.equal(requests.length, 1, "one reserved turn must never dispatch another model request");
    assert.equal(requests[0]?.model, input.model);
    assert.equal(requests[0]?.max_tokens, 256);
    assert.deepEqual(tools, ["inspect_development_failures"]);
    const exposed = requests[0]?.tools as { function: { name: string } }[];
    assert.deepEqual(exposed.map((tool) => tool.function.name).sort(), [
      "inspect_development_failures",
      "inspect_training_rows",
      "propose_dataset_edits",
    ]);
    assert.ok(
      events.some(
        (event) => event.type === "agent_text" && event.text.includes("failed development cases"),
      ),
    );
    assert.ok(
      events.some(
        (event) =>
          event.type === "turn_completed" && event.inputTokens === 35 && event.outputTokens === 12,
      ),
    );
  } finally {
    delete process.env.ENCODER_OPTIMIZATION_FIXTURE_KEY;
    server.closeAllConnections();
    await new Promise<void>((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  }
});

test("a project provider error is not retried invisibly inside the SDK", async () => {
  let calls = 0;
  const server = createServer((_incoming, outgoing) => {
    calls += 1;
    outgoing.writeHead(429, { "content-type": "application/json" });
    outgoing.end(JSON.stringify({ error: { message: "fixture throttle" } }));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  try {
    const agent = new PiResearchAgent({
      async execute() {
        return { content: {} };
      },
    });
    await assert.rejects(
      agent.run(request(`http://127.0.0.1:${address.port}/v1`)),
      /fixture throttle/,
    );
    assert.equal(calls, 1);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  }
});

test("missing selected credentials fail before provider dispatch", async () => {
  const input = request("http://127.0.0.1:1/v1");
  input.apiKeyEnv = "ENCODER_OPTIMIZATION_MISSING_FIXTURE_KEY";
  const agent = new PiResearchAgent({
    async execute() {
      return { content: {} };
    },
  });
  await assert.rejects(agent.run(input), /no available API key/);
});

test("proposal-only capability exposes no inspection tools", () => {
  const tools = createEncoderOptimizationTools(
    "test-run",
    {
      async execute() {
        return { content: {} };
      },
    },
    true,
  );
  assert.deepEqual(
    tools.map((tool) => tool.name),
    ["propose_dataset_edits"],
  );
});

test("proposal-only project call requires the one exposed proposal tool", async () => {
  let payload: Record<string, unknown> | undefined;
  const server = createServer(async (incoming, outgoing) => {
    const chunks: Buffer[] = [];
    for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
    payload = JSON.parse(Buffer.concat(chunks).toString());
    const event = (delta: unknown, finish: string | null = null) =>
      `data: ${JSON.stringify({ id: "proposal-response", object: "chat.completion.chunk", model: "key-visible-custom-model", choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;
    outgoing.writeHead(200, { "content-type": "text/event-stream" });
    outgoing.write(
      event({
        role: "assistant",
        tool_calls: [
          {
            index: 0,
            id: "proposal-1",
            type: "function",
            function: {
              name: "propose_dataset_edits",
              arguments:
                '{"summary":"No justified edit remains.","stop":true,"removals":[],"additions":[]}',
            },
          },
        ],
      }),
    );
    outgoing.write(event({}, "tool_calls"));
    outgoing.write(
      `data: ${JSON.stringify({ choices: [], usage: { prompt_tokens: 20, completion_tokens: 8, total_tokens: 28 } })}\n\n`,
    );
    outgoing.end("data: [DONE]\n\n");
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  const input = request(`http://127.0.0.1:${address.port}/v1`);
  input.capabilitySet = "encoder_optimization_proposal_v1";
  const calls: string[] = [];
  const agent = new PiResearchAgent({
    async execute(call) {
      calls.push(call.name);
      return { content: { accepted: true }, terminate: true };
    },
  });
  try {
    await agent.run(input);
    assert.deepEqual(calls, ["propose_dataset_edits"]);
    assert.deepEqual(payload?.tool_choice, {
      type: "function",
      function: { name: "propose_dataset_edits" },
    });
    const exposed = payload?.tools as { function: { name: string } }[];
    assert.deepEqual(
      exposed.map((tool) => tool.function.name),
      ["propose_dataset_edits"],
    );
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
  }
});

test("official DeepSeek connections use Responses with a required proposal tool", async () => {
  const input = request("https://api.deepseek.com");
  input.model = "deepseek-flash";
  input.capabilitySet = "encoder_optimization_proposal_v1";
  input.apiKeyEnv = "ENCODER_OPTIMIZATION_DEEPSEEK_FIXTURE_KEY";
  process.env.ENCODER_OPTIMIZATION_DEEPSEEK_FIXTURE_KEY = "fixture-not-a-secret";
  try {
    const { model, models } = projectProvider(input);
    assert.equal(model.api, "openai-responses");
    assert.equal(model.reasoning, true);
    assert.equal(model.compat?.supportsDeveloperRole, false);
    let payload: Record<string, unknown> | undefined;
    let requestUrl: string | undefined;
    const proposalArguments = {
      summary: "No justified edit remains.",
      stop: true,
      removals: [],
      additions: [],
    };
    const functionCall = {
      type: "function_call",
      id: "function-1",
      call_id: "call-1",
      name: "propose_dataset_edits",
      arguments: JSON.stringify(proposalArguments),
      status: "completed",
    };
    const tools = createEncoderOptimizationTools(
      "test-run",
      {
        async execute() {
          return { content: {} };
        },
      },
      true,
    );
    const stream = models.streamSimple(
      model,
      {
        systemPrompt: "Use the one supplied tool.",
        messages: [{ role: "user", content: "Submit the result.", timestamp: Date.now() }],
        tools,
      },
      {
        maxRetries: 0,
        onPayload: (candidate) => configureProjectPayload(input, candidate),
        async fetch(fetchInput, init) {
          requestUrl = fetchInput instanceof Request ? fetchInput.url : String(fetchInput);
          payload = JSON.parse(String(init?.body));
          const response = {
            id: "response-1",
            object: "response",
            status: "completed",
            model: "deepseek-flash",
            output: [functionCall],
            usage: {
              input_tokens: 20,
              input_tokens_details: { cached_tokens: 0 },
              output_tokens: 8,
              output_tokens_details: { reasoning_tokens: 0 },
              total_tokens: 28,
            },
          };
          const events = [
            { type: "response.created", sequence_number: 0, response },
            {
              type: "response.output_item.added",
              sequence_number: 1,
              output_index: 0,
              item: { ...functionCall, arguments: "", status: "in_progress" },
            },
            {
              type: "response.function_call_arguments.done",
              sequence_number: 2,
              output_index: 0,
              item_id: functionCall.id,
              arguments: functionCall.arguments,
            },
            {
              type: "response.output_item.done",
              sequence_number: 3,
              output_index: 0,
              item: functionCall,
            },
            { type: "response.completed", sequence_number: 4, response },
          ];
          const body = `${events
            .map((event) => `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`)
            .join("")}data: [DONE]\n\n`;
          return new Response(body, {
            status: 200,
            headers: { "content-type": "text/event-stream" },
          });
        },
      },
    );
    let result: { stopReason: string; content: unknown[] } | undefined;
    for await (const event of stream) {
      if (event.type === "done") result = event.message;
      if (event.type === "error") assert.fail(event.error.errorMessage);
    }
    assert.equal(requestUrl, "https://api.deepseek.com/responses");
    assert.deepEqual(payload?.reasoning, { effort: "none" });
    assert.equal(payload?.tool_choice, "required");
    assert.equal(payload?.thinking, undefined);
    assert.equal(payload?.reasoning_effort, undefined);
    assert.equal(payload?.messages, undefined);
    assert.ok(Array.isArray(payload?.input));
    const exposed = payload?.tools as { name: string }[];
    assert.deepEqual(
      exposed.map((tool) => tool.name),
      ["propose_dataset_edits"],
    );
    assert.equal(result?.stopReason, "toolUse");
    assert.deepEqual(result?.content, [
      {
        type: "toolCall",
        id: "call-1|function-1",
        name: "propose_dataset_edits",
        arguments: proposalArguments,
      },
    ]);
  } finally {
    delete process.env.ENCODER_OPTIMIZATION_DEEPSEEK_FIXTURE_KEY;
  }

  const generic = request("https://provider.example/v1");
  const { model: genericModel } = projectProvider(generic);
  assert.equal(genericModel.api, "openai-completions");
  assert.equal(genericModel.reasoning, false);
});
