import assert from "node:assert/strict";
import test from "node:test";
import { trainingSnapshot } from "../dist/evidence/training-metrics.js";

const event = progress => ({at:"2026-09-19T10:00:00Z",progress});
test("telemetry distinguishes missing, zero, estimated remaining and pending checkpoint", () => {
  const events = [event({phase:"loading_model"}), event({phase:"training",completed:2,total:10,training:{elapsedSeconds:0,remainingSeconds:8}})];
  let value = trainingSnapshot(events, true);
  assert.equal(value.step, 2); assert.equal(value.elapsedSeconds, 0); assert.equal(value.remainingSeconds, 8); assert.equal(value.finalLoss, undefined);
  assert.equal(trainingSnapshot(events, false).remainingSeconds, undefined);
  events.push(event({phase:"training",completed:10,total:10,training:{elapsedSeconds:9,remainingSeconds:0}}));
  value = trainingSnapshot(events, true);
  assert.match(value.state, /verification still pending/); assert.equal(value.remainingSeconds, undefined);
  events.push(event({phase:"saving_checkpoint",training:{finalLoss:0,elapsedSeconds:12}}));
  value = trainingSnapshot(events, true);
  assert.equal(value.state, "Saving and verifying checkpoint"); assert.equal(value.finalLoss, 0); assert.equal(value.elapsedSeconds, 12);
  assert.equal(trainingSnapshot(events, false).state, "Recorded training observations");
  events.push(event({phase:"loading_model"}));
  value = trainingSnapshot(events, true);
  assert.equal(value.step, undefined); assert.equal(value.finalLoss, undefined); assert.equal(value.elapsedSeconds, undefined);
});
