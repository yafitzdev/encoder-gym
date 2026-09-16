import { randomUUID } from "node:crypto";
import { fingerprint, setupFixture } from "./optimization-setup-fixture.mjs";

export function finalFixture() {
  const base = setupFixture(), identity = () => ({ id: randomUUID(), fingerprint });
  const time = offset => new Date(Date.UTC(2026, 8, 16, 10, 0, offset)).toISOString();
  const scope = {
    run: identity(), launch: identity(), executionHead: fingerprint, adaptiveClosedAt: time(0),
    completion: identity(), iteration: identity(), trainingBindingFingerprint: fingerprint,
    developmentResult: identity(), comparisonBaselineRevision: { id: base.baseline.id, fingerprint }, benchmark: identity(),
    scientificProject: identity(), protocol: identity(), candidate: identity(), model: identity(), dataset: base.version,
    finalSuite: "sealed-final", baselineFinalReport: identity(), metricContractFingerprint: fingerprint,
    maximumEvaluationSeconds: 120, fingerprint,
  };
  const authorization = { id: randomUUID(), scope, authorizedBy: "local-operator", createdAt: time(1), fingerprint };
  const dispatch = { authorization: { id: authorization.id, fingerprint }, reportId: randomUUID(), assessmentId: randomUUID(), createdAt: time(2), fingerprint };
  const receipt = {
    dispatch: { id: authorization.id, fingerprint }, report: { id: dispatch.reportId, fingerprint },
    assessment: { id: dispatch.assessmentId, fingerprint }, accepted: true,
    reportCreatedAt: time(3), createdAt: time(4), fingerprint,
  };
  const run = {
    run: { id: scope.run.id, projectId: base.projectId, launch: scope.launch, setup: identity(), createdAt: time(0), fingerprint },
    state: "agent_completed", attempt: 1, materializationAttempt: 0, experimentAttempt: 0, executionAttempt: 0, finalAttempt: 0,
    lastSequence: 2, headFingerprint: fingerprint, updatedAt: time(0),
    agentExecution: { state: "completed", attemptId: randomUUID(), attempts: 1, completion: scope.completion, lastSequence: 2, headFingerprint: fingerprint, updatedAt: time(0) },
  };
  const view = (state = "completed") => structuredClone({ state, execution: {
    authorization, dispatch: state === "authorized" ? null : dispatch, result: state === "completed" ? receipt : null,
  } });
  return { ...base, scope, run, runId: scope.run.id, authorization, dispatch, receipt, view, time };
}
