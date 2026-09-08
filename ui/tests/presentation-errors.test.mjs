import test from "node:test";
import assert from "node:assert/strict";
import { describeFailure } from "../dist/evidence/presentation-errors.js";

test("workspace errors explain recovery without displaying transport noise", () => {
  const raw = "Error invoking remote method 'encoder-gym:open-managed': Error: Error: This is not an Encoder Gym workspace.\nCaused by: Cannot read C:\\private\\encoder-gym.json";
  const result = describeFailure(raw);
  assert.equal(result.title, "This folder is not a Gym project");
  assert.match(result.recovery, /New project/);
  assert.equal(result.detail, raw);
  assert.doesNotMatch(result.title + result.recovery, /private|remote method/);
});

test("held-out training rejection explains the action without weakening the gate", () => {
  const result = describeFailure(new Error("Row 1 declares a non-training partition."));
  assert.equal(result.title, "This file contains held-out data");
  assert.match(result.recovery, /actual purpose/);
  assert.match(result.detail, /Row 1/);
});

test("missing folders differ from unknown failures and wrong project identity", () => {
  assert.match(describeFailure("The system cannot find the file specified. (os error 2)").recovery, /Locate folder/);
  assert.match(describeFailure("This folder has a different project identity.").title, /different project/);
  const unknown = describeFailure(new Error("Unexpected native failure at C:\\data\\project.sqlite"));
  assert.equal(unknown.title, "The operation couldn't finish");
  assert.match(unknown.recovery, /technical details/i);
  assert.match(unknown.detail, /Unexpected native failure/);
});
