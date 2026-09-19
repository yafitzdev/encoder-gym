import type { InputRunActivityEntry } from "./input-run-activity.js";
import { h } from "./dom.js";

export function trainingSnapshot(events: InputRunActivityEntry[], live: boolean) {
  // A fresh trainer invocation must not inherit an earlier attempt's loss/ETA.
  const start = events.findLastIndex(event => event.progress.phase === "loading_model");
  const current = events.slice(Math.max(0, start));
  const step = current.findLast(event => event.progress.phase === "training" && event.progress.completed !== undefined);
  const saved = current.findLast(event => event.progress.phase === "saving_checkpoint" && event.progress.training);
  const latest = current.findLast(event => ["loading_model", "preparing_batches", "training", "saving_checkpoint"].includes(event.progress.phase));
  const saving = latest?.progress.phase === "saving_checkpoint";
  const stepsFinished = step && step.progress.completed === step.progress.total;
  return {
    step: step?.progress.completed, total: step?.progress.total,
    elapsedSeconds: saved?.progress.training?.elapsedSeconds ?? step?.progress.training?.elapsedSeconds,
    remainingSeconds: live && !saving && !stepsFinished ? step?.progress.training?.remainingSeconds : undefined,
    finalLoss: saved?.progress.training?.finalLoss,
    at: saved?.at ?? step?.at,
    state: !live ? "Recorded training observations" : saving ? "Saving and verifying checkpoint"
      : stepsFinished ? "Steps finished · checkpoint verification still pending"
      : step ? "Training in progress" : "Preparing training",
  };
}

export function trainingMetricsPanel(events: InputRunActivityEntry[], live: boolean): HTMLElement {
  const value = trainingSnapshot(events, live);
  const duration = (seconds?: number) => seconds === undefined ? "Not reported" : seconds >= 3600 ? `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m ${seconds % 60}s` : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  const metric = (label: string, text: string) => h("div", {}, h("dt", {}, label), h("dd", {}, text));
  return h("section", { class: "training-metrics", "aria-label": "Training metrics" },
    h("h3", {}, value.state),
    h("dl", {}, metric("Optimizer steps", value.step === undefined ? "Not reported" : `${value.step.toLocaleString()} of ${value.total!.toLocaleString()}`),
      metric("Reported elapsed", duration(value.elapsedSeconds)), metric("Estimated remaining", duration(value.remainingSeconds)),
      metric("Final mean loss", value.finalLoss === undefined ? "Not reported" : value.finalLoss.toLocaleString(undefined, { maximumSignificantDigits: 6 }))),
    h("p", { class: "muted" }, "Training observations are not evaluation results. Timing excludes stages the trainer does not report; estimates may change."),
    value.at ? h("p", { class: "muted" }, "Last reported ", h("time", { datetime: value.at }, new Date(value.at).toLocaleString())) : null);
}
