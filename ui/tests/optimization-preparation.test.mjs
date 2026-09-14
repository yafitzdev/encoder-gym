import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { OptimizationPreparation } from "../dist/evidence/optimization-preparation.js";

test("Stop aborts the active setup command and fences later commands before a run exists", async () => {
  const sessions = new OptimizationPreparation(), token = randomUUID();
  let signal;
  const work = sessions.command("project", token, async value => {
    signal = value;
    await new Promise(resolve => value.addEventListener("abort", resolve, { once: true }));
    value.throwIfAborted();
  });
  sessions.stop("project", token);
  await assert.rejects(work, { name: "AbortError" });
  assert.equal(signal.aborted, true);
  let started = false;
  await assert.rejects(sessions.command("project", token, async () => { started = true; }));
  assert.equal(started, false);
  sessions.finish("project", token);
  await sessions.command("project", randomUUID(), async value => assert.equal(value.aborted, false));
});

test("Stop between setup commands is retained and cannot interrupt another project", async () => {
  const sessions = new OptimizationPreparation(), token = randomUUID();
  sessions.stop("project", token);
  await assert.rejects(sessions.command("project", token, async () => assert.fail("stopped work started")));
  await sessions.command("other", token, async signal => assert.equal(signal.aborted, false));
  await assert.rejects(sessions.command("project", randomUUID(), async () => {}), /Another setup/);
});
