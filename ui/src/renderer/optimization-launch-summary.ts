import type { OptimizationAgentSettings } from "../optimization-agent-settings.js";
import type { OptimizationProviderLimits } from "../optimization-launch.js";
import type { LaunchBlocker } from "./optimization-setup-controller.js";
import { button, facts } from "./components.js";
import { h } from "./dom.js";

interface LaunchSummary {
  id: string;
  historical: boolean;
  model: string;
  dataset: string;
  benchmark: string;
  agent: string;
  generator: string;
  settings?: OptimizationAgentSettings;
  limits?: { advisor: OptimizationProviderLimits; generation: OptimizationProviderLimits };
  blockers: LaunchBlocker[];
  act(action: NonNullable<LaunchBlocker["action"]>): void;
  start?: HTMLButtonElement;
}

/** Presentation only: readiness and authority come from the controller/launch. */
export function optimizationLaunchSummary(options: LaunchSummary): HTMLElement {
  const { settings, historical } = options;
  const readinessId = `${options.id}-readiness`;
  options.start?.setAttribute("aria-describedby", readinessId);
  const diagnostic = settings?.mode === "quick_test" || settings?.training.maximumTrainingRows != null;
  const limits = historical ? options.limits : settings?.providerLimits ?? options.limits;
  const actionLabels = { models: "Open Models", datasets: "Open Datasets", benchmarks: "Open Evaluation", project: "Open Project settings", settings: "Review Advanced", refresh: "Retry checks" };
  return h("aside", { class: "optimization-launch-summary", "aria-label": historical ? "Saved launch summary" : "Launch summary" },
    h("h2", {}, historical ? "Saved launch" : "This run"),
    facts([["Baseline", options.model], ["Dataset", options.dataset], ["Evaluation", options.benchmark], ["Agent", options.agent], ["Generator", options.generator]]),
    settings ? h("div", { class: "launch-limits" },
      h("strong", {}, diagnostic ? "Diagnostic run" : "Standard run"),
      h("p", {}, `Up to ${settings.maximumIterations} iteration${settings.maximumIterations === 1 ? "" : "s"} · ${settings.maximumRowChanges.toLocaleString()} row changes total`),
      h("p", {}, `${settings.training.device.toUpperCase()} · ${settings.training.maximumEpochs} epoch${settings.training.maximumEpochs === 1 ? "" : "s"} · batch ${settings.training.batchSize}`),
      h("p", {}, `Training limit: ${settings.training.maximumSecondsPerIteration.toLocaleString()} seconds per iteration${settings.training.maximumTrainingRows == null ? " · all rows" : ` · up to ${settings.training.maximumTrainingRows.toLocaleString()} rows`}`),
      diagnostic ? h("p", { class: "diagnostic-notice" }, "Development only. No final holdout or promotion.") : null) : h("p", { class: "muted" }, "Run limits not recorded."),
    limits ? h("details", { class: "launch-provider-limits" }, h("summary", {}, "Provider ceilings for this run"),
      ...(["advisor", "generation"] as const).map(role => h("div", {}, h("strong", {}, role === "advisor" ? "Agent" : "Generator"),
        h("p", {}, `${limits[role].maximumRequests.toLocaleString()} requests · ${limits[role].maximumInputTokens.toLocaleString()} input tokens · ${limits[role].maximumOutputTokens.toLocaleString()} output tokens · $${(limits[role].maximumCostMicrousd / 1_000_000).toFixed(2)}`)))) : null,
    !historical ? h("div", { id: readinessId, class: "launch-readiness", "aria-live": "polite" },
      h("strong", {}, options.blockers.length ? "Before you start" : "Inputs ready"),
      options.blockers.length ? h("ul", {}, ...options.blockers.map(item => h("li", { "data-blocker": item.code }, h("span", {}, item.message),
        item.action ? button(actionLabels[item.action], () => options.act(item.action!), "ghost small") : null)))
        : h("p", { class: "muted" }, "Local selections checked. Files and native compatibility are verified at launch.")) : null,
    options.start ?? null);
}
