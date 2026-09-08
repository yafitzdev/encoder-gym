import test from "node:test";
import assert from "node:assert/strict";
import { NavigationHistory } from "../dist/evidence/navigation.js";

test("returning to a collection restores its scroll and focused model across detail tabs", () => {
  const history = new NavigationHistory();
  history.remember({ page: "candidate", id: "one" }, 420, "candidate-one");
  history.remember({ page: "candidate", id: "one", tab: "training" }, 100);
  const entry = history.returnTo("models", 50);
  assert.deepEqual(entry, { location: { page: "models" }, scroll: 420, focusId: "candidate-one" });
  assert.equal(history.canNavigate(-1), false);
  assert.equal(history.canNavigate(1), true);
});

test("bookmarking a project preserves its current page without adding history", () => {
  const history = new NavigationHistory();
  history.remember({ page: "datasets" }, 0);
  history.capture(180, "dataset-details");
  assert.deepEqual(history.bookmark, { location: { page: "datasets" }, scroll: 180, focusId: "dataset-details" });
  assert.equal(history.move(-1, 180).location.page, "models");
  assert.equal(history.canNavigate(-1), false);
  assert.equal(history.returnTo("runs", 0), undefined);
});

test("a new navigation branch discards forward history without mixing instances", () => {
  const first = new NavigationHistory(), second = new NavigationHistory();
  first.remember({ page: "runs" }, 60);
  first.move(-1, 130);
  first.remember({ page: "datasets" }, 60);
  assert.equal(first.canNavigate(1), false);
  assert.equal(second.current.page, "models");
});
