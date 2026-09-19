// Real Electron -> IPC -> production CLI -> real Pi -> loopback model server.
// Only provider responses and the native scientific executable are deterministic.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, writeFileSync, appendFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawn, execFileSync } from "node:child_process";
import electron from "electron";

const root = mkdtempSync(join(tmpdir(), "encoder-gym-agent-journey-"));
const profile = join(root, "profile"), project = join(root, "fixture"), calls = [];
let held = false, serverError, currentPhase;
const server = createServer(async (request, response) => {
  try {
    assert.equal(request.url, "/v1/chat/completions");
    let body = ""; for await (const chunk of request) body += chunk;
    assert.ok(!body.includes("NEVER_DISCLOSE_HOLDOUT"));
    assert.ok(!body.includes("99999.125") && !body.includes("99999.225"));
    const message = JSON.parse(body);
    if (message.model === "pinned-generator") {
      calls.push({ model: message.model, phase: currentPhase });
      appendFileSync(join(root, "provider-calls.jsonl"), JSON.stringify(calls.at(-1)) + "\n");
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end(JSON.stringify({ choices: [{ message: { role: "assistant", content: JSON.stringify({ rows: [{ question: "Search the exact technical reference", ...(currentPhase === "invalid" ? { evaluation_partition: "sealed" } : {}) }] }) } }], usage: { prompt_tokens: 120, completion_tokens: 40, total_tokens: 160 } }));
      return;
    }
    assert.equal(message.model, "pinned-agent"); assert.equal(message.stream, true);
    const user = message.messages.findLast(value => value.role === "user");
    const input = JSON.parse(typeof user.content === "string" ? user.content : user.content.map(part => part.text ?? "").join(""));
    const turns = input.previousTurns.filter(turn => !turn.interrupted), later = input.scope.iteration > 1;
    calls.push({ model: message.model, phase: currentPhase, iteration: input.scope.iteration, previous: turns.length, evidence: input.scope.developmentEvidenceFingerprint });
    appendFileSync(join(root, "provider-calls.jsonl"), JSON.stringify(calls.at(-1)) + "\n");
    if (!held && turns.length === 1) {
      held = true; writeFileSync(join(root, "held.json"), JSON.stringify(calls.at(-1))); return;
    }
    let name, args;
    if (!turns.length) { name = "inspect_dataset_landscape"; args = { offset: 0, limit: 20 }; }
    else if (turns.length === 1) {
      const clusters = turns[0].tools[0].result.items;
      const cluster = clusters.find(item => item.content.cluster?.dimension === "expected_capability" && item.content.cluster?.value === "search") ?? clusters[0];
      assert.ok(cluster);
      if (later) {
        const examples = cluster.content.development.flatMap(value => value.sampledFailureDiagnostics.examples);
        assert.ok(examples.some(example => example.expectedRank === 3 && /candidate regression/.test(example.questionPreview)));
      }
      name = "inspect_dataset_clusters"; args = { clusterIds: [cluster.id], examplesPerCluster: 4 };
    } else {
      const clusters = turns[0].tools[0].result.items;
      const evidence = clusters.find(item => item.content.cluster?.dimension === "expected_capability" && item.content.cluster?.value === "search") ?? clusters[0];
      const rows = turns[1].tools[0].result.items;
      assert.ok(evidence && rows.length);
      const row = rows.find(item => item.content.question.includes(later ? "Retain" : "Search")) ?? rows[0];
      name = "propose_dataset_edits";
      if (input.scope.iteration === 3) {
        args = { summary: "The candidate regression does not justify another dataset change.", stop: true, removals: [], additions: [] };
      } else {
        args = { summary: "Shift search coverage by one row using the weak inspected cluster.", stop: false,
          removals: [{ rowId: row.id, reason: "Ambiguous wording conflicts with the weak cluster.", evidenceIds: [evidence.id] }],
          additions: [{ templateRowId: row.id, instruction: "Add one specific search request for the weak cluster.", count: 1, evidenceIds: [evidence.id] }] };
      }
      if (later && input.scope.iteration !== 3) {
        assert.equal(input.scope.maximumRowChanges, 6);
        args = { summary: "Changed candidate evidence identifies conflicting retain coverage.", stop: false,
          removals: [{ rowId: row.id, reason: "Remove the conflict found in candidate evidence.", evidenceIds: [evidence.id] }], additions: [] };
      }
    }
    response.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache" });
    const chunk = (delta, finish_reason = null, usage) => response.write("data: " + JSON.stringify({ id: "offline-call", object: "chat.completion.chunk", created: 1, model: message.model, choices: [{ index: 0, delta, finish_reason }], ...(usage ? { usage } : {}) }) + "\n\n");
    chunk({ role: "assistant", tool_calls: [{ index: 0, id: "offline-tool", type: "function", function: { name, arguments: JSON.stringify(args) } }] });
    chunk({}, "tool_calls", { prompt_tokens: 150, completion_tokens: 70, total_tokens: 220 });
    response.end("data: [DONE]\n\n");
  } catch (error) { serverError = error; console.error(error); response.writeHead(500); response.end("Offline fixture assertion failed"); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
try {
  const binary = resolve("../target/debug/synth-benchmark-fixture" + (process.platform === "win32" ? ".exe" : ""));
  const fixture = JSON.parse(execFileSync(binary, ["--agent-project", project, `http://127.0.0.1:${server.address().port}/v1`], { encoding: "utf8", windowsHide: true, maxBuffer: 16 * 1024 * 1024 }));
  writeFileSync(join(root, "fixture.json"), JSON.stringify(fixture));
  for (const phase of ["stop", "resume", "final", "recover", "verify", "quick", "invalid"]) {
    currentPhase = phase;
    if (["quick", "invalid"].includes(phase)) {
      const next = JSON.parse(execFileSync(binary, ["--agent-project", join(root, phase), `http://127.0.0.1:${server.address().port}/v1`], { encoding: "utf8", windowsHide: true, maxBuffer: 16 * 1024 * 1024 }));
      writeFileSync(join(root, "fixture.json"), JSON.stringify(next));
    }
    const child = spawn(electron, [".", "--smoke-test", "--smoke-agent"], { stdio: "inherit", windowsHide: true,
      env: { ...process.env, ENCODER_GYM_SMOKE_PROFILE: profile, ENCODER_GYM_AGENT_JOURNEY: root, ENCODER_GYM_AGENT_PHASE: phase } });
    const timeout = setTimeout(() => child.kill(), 600_000);
    const code = await new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", resolve); }); clearTimeout(timeout);
    if (serverError) throw serverError;
    assert.equal(code, 0, `Electron ${phase} failed; evidence: ${root}`);
  }
  const adaptive = calls.filter(call => ["stop", "resume"].includes(call.phase));
  assert.equal(adaptive.filter(call => call.model === "pinned-generator").length, 1);
  assert.equal(adaptive.filter(call => call.model === "pinned-agent").length, 10);
  for (const iteration of [1, 2, 3]) assert.equal(adaptive.filter(call => call.iteration === iteration && call.previous === 0).length, 1, "Completed inspection must not repeat");
  assert.notEqual(adaptive.find(call => call.iteration === 1).evidence, adaptive.find(call => call.iteration === 2).evidence);
  assert.equal(calls.filter(call => ["final", "recover", "verify"].includes(call.phase)).length, 0, "Final evidence cannot start adaptive provider calls");
  for (const phase of ["quick", "invalid"]) {
    assert.equal(calls.filter(call => call.phase === phase && call.model === "pinned-agent").length, 3);
    assert.equal(calls.filter(call => call.phase === phase && call.model === "pinned-generator").length, 1);
  }
  console.log(`PASS connected Agent journey with real Pi, two training iterations, Stop/restart/Resume, explicit final consent and promotion. Evidence: ${root}`);
} finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
