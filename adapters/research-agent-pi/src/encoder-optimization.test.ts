import assert from "node:assert/strict";
import { createServer } from "node:http";
import test from "node:test";

import { PiResearchAgent } from "./agent.js";
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
