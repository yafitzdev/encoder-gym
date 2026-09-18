import assert from "node:assert/strict";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { BrowserWindow } from "electron";
import type { ManagedBackend } from "./managed-backend.js";
import type { ProjectRegistry } from "./project-registry.js";

/** Offline acceptance only. Never substitutes a bridge/controller/backend method. */
export async function runAgentJourney(window: BrowserWindow, registry: ProjectRegistry, backend: ManagedBackend): Promise<void> {
  const root = process.env.ENCODER_GYM_AGENT_JOURNEY!, phase = process.env.ENCODER_GYM_AGENT_PHASE!;
  const fixture = JSON.parse(readFileSync(join(root, "fixture.json"), "utf8")), id: string = fixture.projectId;
  const output = join(root, "screenshots"); mkdirSync(output, { recursive: true });
  const agentOutcomes = (): string[] => {
    const database = new DatabaseSync(join(fixture.folder, "project.sqlite"), { readOnly: true });
    try { return database.prepare("SELECT o.metadata_json FROM optimization_agent_outcomes o JOIN optimization_agent_calls c ON c.id=o.call_id ORDER BY c.sequence").all().map(row => row.metadata_json as string); }
    finally { database.close(); }
  };
  registry.addManaged(await backend.open(fixture.folder));
  await new Promise<void>(resolve => { window.webContents.once("did-finish-load", () => resolve()); window.webContents.reload(); });
  const evaluate = (code: string) => window.webContents.executeJavaScript(code, true);
  const until = async (code: string) => evaluate(`new Promise((resolve,reject)=>{const deadline=Date.now()+240000;const poll=()=>{if(${code})resolve(true);else if(Date.now()>deadline)reject(Error('Timed out: '+${JSON.stringify(code)}+' '+document.body.innerText));else setTimeout(poll,80)};poll()})`);
  const click = async (selector: string) => evaluate(`(()=>{const button=document.querySelector(${JSON.stringify(selector)});if(!button||button.disabled)throw Error('Control unavailable: '+${JSON.stringify(selector)});button.click()})()`);
  const textButton = async (text: string) => evaluate(`(()=>{const button=[...document.querySelectorAll('#page button')].find(b=>b.textContent===${JSON.stringify(text)});if(!button||button.disabled)throw Error('Button unavailable: '+${JSON.stringify(text)});button.click()})()`);
  const choose = async (selector: string, value: string) => evaluate(`(()=>{const input=document.querySelector(${JSON.stringify(selector)});input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  const screenshot = async (name: string, width = 1440) => {
    window.setContentSize(width, 960); await evaluate("new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))");
    writeFileSync(join(output, name + ".png"), (await window.webContents.capturePage()).toPNG());
  };
  try {
    await until("document.querySelector('#nav-overview')"); await click("#nav-overview");
    if (["stop", "quick", "invalid"].includes(phase)) {
      await until("document.querySelector('#optimization-start:not(:disabled)')");
      await choose("#optimization-advanced-draft-edits", "8");
      await choose("#optimization-advanced-draft-device", "cpu");
      await choose("#optimization-advanced-draft-seconds", "120");
      if (phase !== "stop") {
        await choose("#optimization-advanced-draft-mode", "quick_test");
        await choose("#optimization-advanced-draft-device", "cpu");
      }
      assert.match(await evaluate("document.querySelector('#optimization-advisor').textContent"), /pinned-agent/);
      assert.match(await evaluate("document.querySelector('#optimization-generation').textContent"), /pinned-generator/);
      await click("#optimization-start");
      if (phase !== "stop") {
        const expected = "agent_completed";
        await until(`document.querySelector('[data-run-state=${expected}]') && !document.getElementById('overview-new-run').disabled`);
        const runs = await backend.optimizationLaunch.runs(id); assert.equal(runs.length, 1);
        const run = await backend.optimizationLaunch.show(id, runs[0]!.id), launch = (await backend.optimizationLaunch.list(id))[0]!;
        assert.equal(launch.scope.agentic?.mode, "quick_test"); assert.equal(launch.scope.limits.maximumFinalEvaluations, 0);
        const native = readFileSync(join(fixture.folder, "../runtime/native-invocations.log"), "utf8");
        if (phase === "quick") {
          assert.equal(run.iterations?.length, 1); assert.ok(run.iterations![0]!.trainingDatasetVersionId);
          await until(`document.getElementById('overview-${run.id}-report') && !document.getElementById('overview-${run.id}-report').disabled`);
          await click(`#overview-${run.id}-report`);
          assert.match(await evaluate("document.body.innerText"), /Diagnostic run/);
          assert.equal(await evaluate("document.querySelectorAll('[aria-label=\"Final acceptance\"]').length"), 0);
          assert.equal(native.split("tools.train_dense_triplet_router").length - 1, 1);
        } else {
          const rows = await backend.datasetVersions.query(id, { kind: "rows", versionId: run.iterations![0]!.trainingDatasetVersionId, offset: 0, limit: 20 });
          assert.equal(rows.kind, "rows"); if (rows.kind !== "rows") throw Error("Missing exact trainer rows");
          assert.equal(rows.page.total, 1); assert.equal(rows.page.rows[0]!.value.question, "Retain this useful example");
          assert.equal(rows.page.rows[0]!.value.evaluation_partition, "train");
          const database = new DatabaseSync(join(fixture.folder, "project.sqlite"), { readOnly: true });
          const outcomes = database.prepare("SELECT metadata_json FROM optimization_generation_outcomes").all().map(row => JSON.parse(row.metadata_json as string)); database.close();
          assert.equal(outcomes[0].admission.accepted.length, 0); assert.equal(outcomes[0].admission.rejected.length, 1);
        }
        await screenshot("agent-" + phase); console.log(`PASS connected ${phase === "quick" ? "Quick test uses the same engine and excludes final holdout/promotion" : "native admission rejects injected generation authority before training"}`); return;
      }
      const deadline = Date.now() + 90_000;
      while (!existsSync(join(root, "held.json"))) {
        assert.ok(Date.now() < deadline, "Agent HTTP request was not reached: " + await evaluate("document.body.innerText"));
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      await screenshot("agent-active");
      assert.ok(await evaluate("!!document.querySelector('.focus-status .spinner')"));
      await textButton("Stop");
      await until("document.querySelector('[data-run-state=agent_paused]') && [...document.querySelectorAll('#page button')].some(b=>b.textContent==='Resume'&&!b.disabled)");
      assert.equal(await evaluate("document.querySelectorAll('.focus-status .spinner').length"), 0);
      const runs = await backend.optimizationLaunch.runs(id); assert.equal(runs.length, 1); assert.equal(runs[0]!.state, "agent_paused");
      const outcomes = agentOutcomes(); assert.equal(outcomes.length, 2);
      const interrupted = JSON.parse(outcomes[1]!);
      assert.equal(interrupted.interrupted, true); assert.equal(interrupted.usage.inputTokens, null);
      assert.ok(interrupted.call.inputTokenCeiling > 0, "Unknown call retains its conservative charge");
      writeFileSync(join(root, "paused-outcomes.json"), JSON.stringify(outcomes));
      writeFileSync(join(root, "run.json"), JSON.stringify(runs[0]));
      await screenshot("agent-paused"); console.log("PASS rendered Optimize called actual Pi; Stop acknowledged and retained completed inspection"); return;
    }
    const before = JSON.parse(readFileSync(join(root, "run.json"), "utf8")), runId: string = before.id;
    await until(`document.getElementById('overview-run-${runId}')`);
    await evaluate(`(()=>{const b=document.getElementById('overview-run-${runId}');if(b.getAttribute('aria-expanded')==='false')b.click()})()`);
    if (phase === "resume") {
      await click(`#overview-${runId}-status`); await until("[...document.querySelectorAll('#page button')].some(b=>b.textContent==='Resume'&&!b.disabled)");
      const restored = await backend.optimizationLaunch.show(id, runId); assert.equal(restored.agentExecution?.headFingerprint, before.agentExecution.headFingerprint);
      await textButton("Resume"); await until("document.querySelector('[data-run-state=agent_completed]') && !document.querySelector('.focus-run-heading .spinner')");
      assert.equal(await evaluate("document.querySelectorAll('#nav-overview .spinner').length"), 0, "Durable completion ends sidebar activity even while IPC settles");
      const run = await backend.optimizationLaunch.show(id, runId); assert.equal(run.iterations?.length, 3);
      const completedOutcomes = agentOutcomes();
      for (const paused of JSON.parse(readFileSync(join(root, "paused-outcomes.json"), "utf8")) as string[]) {
        assert.ok(completedOutcomes.includes(paused), "Resume must preserve the exact saved outcome and charge; sequences are per iteration");
      }
      assert.equal(run.iterations![0]!.selected, true); assert.equal(run.iterations![2]!.noChange, true);
      assert.notEqual(run.iterations![0]!.modelId, run.iterations![1]!.modelId);
      assert.equal(await backend.optimizationFinal.read(id, runId), null);
      await click(`#overview-${runId}-report`); await choose(`#overview-${runId}-iteration`, "1");
      await until("document.querySelector('[data-iteration-report=\"1\"]')");
      await screenshot("agent-report"); await screenshot("agent-report-narrow", 390); window.setContentSize(1440, 960);
      assert.equal(run.iterations![1]!.developmentPassed, false, "Rejected candidates remain inspectable");
      for (const iteration of [1, 2]) {
        await choose(`#overview-${runId}-iteration`, String(iteration));
        await until(`document.querySelector('[data-iteration-report="${iteration}"]')`);
        for (const [label, selector] of [["View model", "#detail-panel"], ["Dataset changes", ".dataset-changes"], ["Evaluation reports", ".benchmark-results"]] as const) {
          await textButton(label);
          await until("!document.querySelector('#source-state')?.textContent.includes('Reading') && document.querySelector('#page h1')?.textContent!=='Overview'");
          await until(`document.querySelector(${JSON.stringify(selector)})`);
          if (label === "Dataset changes") {
            assert.equal(await evaluate("document.querySelectorAll('[data-change-kind=added]').length"), iteration === 1 ? 1 : 0);
            assert.equal(await evaluate("document.querySelectorAll('[data-change-kind=removed]').length"), 1);
          }
          assert.ok(!await evaluate("!!document.querySelector('#page [role=alert]')"), `Artifact navigation failed: ${label} (${selector})`);
          await screenshot(`agent-iteration-${iteration}-` + label.toLowerCase().replaceAll(" ", "-"));
          await click("#nav-overview"); await until(`document.getElementById('overview-${runId}-report')`); await click(`#overview-${runId}-report`);
        }
      }
      await click(`#overview-${runId}-status`); await choose(`#overview-${runId}-iteration`, "1");
      await until("document.querySelector('[data-key=preparing_data] button')");
      await click("[data-key=preparing_data] button");
      await until("document.querySelector('.focus-status')?.textContent.includes('Agent') && document.querySelector('.focus-status')?.textContent.includes('Generation')");
      await screenshot("agent-activity");
      assert.ok(!await evaluate("document.body.innerText.includes('NEVER_DISCLOSE_HOLDOUT')||document.body.innerText.includes('99999.125')"));
      writeFileSync(join(root, "completed.json"), JSON.stringify(run));
      writeFileSync(join(root, "before-final-native.txt"), readFileSync(join(fixture.folder, "../runtime/native-invocations.log")));
      console.log("PASS restarted Resume completed two evidence-dependent training cycles and no-change completion; ordinary artifact viewers and real Agent/Generation activity opened"); return;
    }
    // Reopened runs load their detailed history on demand. A visible root is
    // not yet a ready report; wait for the real renderer's enabled control.
    await until(`document.getElementById('overview-${runId}-report') && !document.getElementById('overview-${runId}-report').disabled`);
    await click(`#overview-${runId}-report`);
    if (phase === "final") {
      await until(`document.getElementById('agent-final-review-${runId}') && !document.getElementById('agent-final-review-${runId}').disabled`);
      await click(`#agent-final-review-${runId}`);
      await until(`document.getElementById('agent-final-authorize-${runId}') && !document.getElementById('agent-final-authorize-${runId}').disabled`);
      assert.equal(await backend.optimizationFinal.read(id, runId), null, "Review must not grant consent");
      await screenshot("agent-final-consent");
      const database = new DatabaseSync(join(fixture.folder, "project.sqlite"));
      database.exec("CREATE TRIGGER desktop_lost_final_receipt BEFORE INSERT ON optimization_agent_final_results BEGIN SELECT RAISE(ABORT,'injected final receipt loss'); END"); database.close();
      await click(`#agent-final-authorize-${runId}`);
      await until(`document.getElementById('agent-final-recover-${runId}') && !document.getElementById('agent-final-recover-${runId}').disabled`);
      const unknown = await backend.optimizationFinal.read(id, runId); assert.equal(unknown?.state, "outcome_unknown");
      writeFileSync(join(root, "unknown-final.json"), JSON.stringify(unknown));
      writeFileSync(join(root, "after-dispatch-native.txt"), readFileSync(join(fixture.folder, "../runtime/native-invocations.log")));
      const recovery = new DatabaseSync(join(fixture.folder, "project.sqlite")); recovery.exec("DROP TRIGGER desktop_lost_final_receipt"); recovery.close();
      await screenshot("agent-final-unknown"); console.log("PASS rendered consent consumed one grant; lost receipt is shown as unknown, not repeated"); return;
    }
    if (phase === "recover") {
      await until(`document.getElementById('agent-final-recover-${runId}') && !document.getElementById('agent-final-recover-${runId}').disabled`);
      const unknown = JSON.parse(readFileSync(join(root, "unknown-final.json"), "utf8"));
      assert.deepEqual(await backend.optimizationFinal.read(id, runId), unknown);
      await click(`#agent-final-recover-${runId}`);
      await until(`document.getElementById('agent-final-promote-${runId}') && !document.getElementById('agent-final-promote-${runId}').disabled`);
      const result = await backend.optimizationFinal.read(id, runId); assert.equal(result?.execution.result?.accepted, true);
      assert.equal(result?.execution.authorization.id, unknown.execution.authorization.id);
      assert.equal(result?.execution.result?.report.id, unknown.execution.dispatch.reportId);
      assert.deepEqual(readFileSync(join(fixture.folder, "../runtime/native-invocations.log")), readFileSync(join(root, "after-dispatch-native.txt")), "Recovery must not repeat native work");
      const workspace = await backend.openRegistered(id); assert.equal(workspace.modelCatalog!.baselineRevisions.length, 1, "Final acceptance is not promotion");
      await click(`#agent-final-promote-${runId}`);
      await until("document.querySelector('[aria-label=\"Final acceptance\"]')?.textContent.includes('Candidate promoted to baseline.')");
      await screenshot("agent-promoted");
      writeFileSync(join(root, "final.json"), JSON.stringify(result));
      writeFileSync(join(root, "after-final-native.txt"), readFileSync(join(fixture.folder, "../runtime/native-invocations.log")));
      console.log("PASS explicit rendered final consent, actual final CLI dispatch and separate manual baseline promotion"); return;
    }
    await until("document.querySelector('[aria-label=\"Final acceptance\"]')?.textContent.includes('Candidate promoted to baseline.')");
    assert.deepEqual(await backend.optimizationFinal.read(id, runId), JSON.parse(readFileSync(join(root, "final.json"), "utf8")));
    assert.deepEqual(readFileSync(join(fixture.folder, "../runtime/native-invocations.log")), readFileSync(join(root, "after-final-native.txt")));
    assert.equal((await backend.openRegistered(id)).modelCatalog!.baselineRevisions.length, 2);
    console.log("PASS second restart retained final result and one promotion without repeating scientific work");
  } catch (error) { await screenshot("failure-" + phase); throw error; }
}
