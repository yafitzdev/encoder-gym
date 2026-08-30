import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import test from "node:test";
import { fileURLToPath } from "node:url";

import type { OutputMessage, PiRunRequest } from "./protocol.js";
import { PROTOCOL_VERSION } from "./protocol.js";

test("compiled sidecar exchanges one JSON object per line", { timeout: 10_000 }, async () => {
  const child = spawn(process.execPath, [fileURLToPath(new URL("./main.js", import.meta.url))], {
    stdio: ["pipe", "pipe", "pipe"],
    windowsHide: true,
  });
  const output: OutputMessage[] = [];
  const request: PiRunRequest = {
    protocolVersion: PROTOCOL_VERSION,
    capabilitySet: "authenticity_research_v1",
    runId: "process-run",
    runSpecificationFingerprint: "sha256:process",
    provider: "fake",
    model: "scripted",
    systemPrompt: "Research style.",
    initialPrompt: "Start.",
    maxModelTurns: 2,
    scriptedTurns: [
      {
        toolCalls: [
          {
            name: "draft_profile",
            arguments: { claims: [], profile: { summary: "Natural fragments" } },
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

  const completed = new Promise<void>((resolve, reject) => {
    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      const message = JSON.parse(line) as OutputMessage;
      output.push(message);
      if (message.type === "ready") {
        child.stdin.write(`${JSON.stringify({ type: "start", request })}\n`);
      } else if (message.type === "tool_request") {
        child.stdin.write(
          `${JSON.stringify({
            type: "tool_result",
            callId: message.callId,
            result: { content: { accepted: true } },
          })}\n`,
        );
      } else if (message.type === "completed") {
        child.stdin.end();
        resolve();
      } else if (message.type === "failed") {
        reject(new Error(message.message));
      }
    });
  });

  await completed;
  await new Promise<void>((resolve, reject) => {
    child.once("exit", (code) => (code === 0 ? resolve() : reject(new Error(`exit ${code}`))));
  });

  assert.deepEqual(
    output.filter((message) => message.type === "tool_request").map((message) => message.name),
    ["draft_profile", "finish_research"],
  );
});
