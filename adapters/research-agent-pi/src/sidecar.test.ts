import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

import type { OutputMessage, PiRunRequest } from "./protocol.js";
import { PROTOCOL_VERSION } from "./protocol.js";
import { PI_PACKAGE_VERSION, ProtocolSession } from "./sidecar.js";

const require = createRequire(import.meta.url);

function request(): PiRunRequest {
  return {
    protocolVersion: PROTOCOL_VERSION,
    runId: "run-protocol",
    runSpecificationFingerprint: "sha256:run",
    provider: "fake",
    model: "scripted",
    systemPrompt: "Research authentic messages.",
    initialPrompt: "Start.",
    maxModelTurns: 3,
    scriptedTurns: [
      {
        toolCalls: [
          {
            name: "search_web",
            arguments: { query: "authentic examples", maximumResults: 2 },
          },
        ],
      },
      {
        toolCalls: [
          {
            name: "draft_profile",
            arguments: { claims: [], profile: { summary: "Short and noisy" } },
          },
        ],
      },
      {
        toolCalls: [
          {
            name: "finish_research",
            arguments: { reason: "sufficient_evidence", summary: "done" },
          },
        ],
      },
    ],
  };
}

test("JSONL session handshakes and proxies every tool through its host", async () => {
  const output: OutputMessage[] = [];
  let session: ProtocolSession;
  session = new ProtocolSession((message) => {
    output.push(message);
    if (message.type === "tool_request") {
      queueMicrotask(() => {
        session.handle({
          type: "tool_result",
          callId: message.callId,
          result: { content: { persisted: true } },
        });
      });
    }
  });

  await session.ready();
  session.handle({ type: "start", request: request() });
  await session.waitForIdle();

  assert.deepEqual(output[0], {
    type: "ready",
    protocolVersion: PROTOCOL_VERSION,
    piPackageVersion: PI_PACKAGE_VERSION,
  });
  assert.deepEqual(
    output.filter((message) => message.type === "tool_request").map((message) => message.name),
    ["search_web", "draft_profile", "finish_research"],
  );
  assert.equal(output.at(-1)?.type, "completed");
});

test("unknown tool results are rejected instead of corrupting another call", () => {
  const session = new ProtocolSession(() => undefined);
  assert.throws(
    () =>
      session.handle({
        type: "tool_result",
        callId: "missing",
        result: { content: {} },
      }),
    /unknown or completed tool call/,
  );
});

test("reported Pi version matches the exact installed runtime", () => {
  const packageMetadata = require("@earendil-works/pi-agent-core/package.json") as {
    version: string;
  };
  assert.equal(PI_PACKAGE_VERSION, packageMetadata.version);
});
