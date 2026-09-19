import type { OptimizationAgentSettings } from "../optimization-agent-settings.js";
import type { OptimizationProviderLimits } from "../optimization-launch.js";
import { h } from "./dom.js";

interface Options {
  id: string;
  settings?: OptimizationAgentSettings;
  limits?: { advisor: OptimizationProviderLimits; generation: OptimizationProviderLimits };
  historical: boolean;
  disabled: boolean;
  open: boolean;
  toggle(open: boolean): void;
  change(settings: OptimizationAgentSettings): void;
  mode(mode: OptimizationAgentSettings["mode"]): void;
  error?: string;
}

/** A view of the core-owned settings. Preview and dispatch validate the same shape. */
export function optimizationSettings(options: Options): HTMLElement {
  const { settings, historical, disabled } = options;
  const id = (key: string) => `${options.id}-${key}`;
  const field = (label: string, control: HTMLElement, hint?: string) => h("label", { class: "advanced-field", for: control.id },
    h("span", {}, label), control, hint ? h("small", { class: "muted" }, hint) : null);
  const edit = (change: (value: OptimizationAgentSettings) => void): void => {
    if (!settings || disabled) return;
    const next = structuredClone(settings); change(next); options.change(next);
  };
  const number = (key: string, label: string, value: number, maximum: number, change: (value: number) => void, minimum = 1, scale = 1, locked = false, hint?: string) =>
    field(label, h("input", { id: id(key), type: "number", min: minimum / scale, max: maximum / scale, step: 1 / scale,
      value: Number.isFinite(value) ? value / scale : "", disabled: disabled || locked,
      onChange: (event: Event) => {
        const value = (event.target as HTMLInputElement).valueAsNumber * scale;
        change(Math.abs(value - Math.round(value)) < 0.000001 ? Math.round(value) : NaN);
      } }), hint);
  const select = (key: string, label: string, value: string, choices: [string, string][], change: (value: string) => void) =>
    field(label, h("select", { id: id(key), value, disabled, onChange: (event: Event) => change((event.target as HTMLSelectElement).value) },
      ...choices.map(([value, text]) => h("option", { value }, text))));
  const quick = settings?.mode === "quick_test";
  const training = settings?.training;
  const budgets = settings?.providerLimits ?? options.limits;
  const roleBudgets = (role: "advisor" | "generation") => {
    if (!budgets || !options.limits) return null;
    const labels: [keyof OptimizationProviderLimits, string, number][] = [
      ["maximumRequests", "Requests", 1], ["maximumInputTokens", "Input tokens", 1],
      ["maximumOutputTokens", "Output tokens", 1], ["maximumCostMicrousd", "Spend ceiling (USD)", 1_000_000],
    ];
    return h("fieldset", {}, h("legend", {}, role === "advisor" ? "Agent · whole run" : "Data generation · whole run"),
      h("div", { class: "advanced-grid" }, ...labels.map(([key, label, scale]) => number(`${role}-${key}`, label, budgets[role][key], options.limits![role][key], value => edit(next => {
        next.providerLimits ??= structuredClone(budgets);
        next.providerLimits[role][key] = value;
      }), key === "maximumCostMicrousd" ? 0 : 1, scale, false,
      historical ? undefined : `Project ceiling: ${options.limits![role][key] / scale}`))));
  };
  return h("details", { id: options.id, class: "optimization-advanced", open: options.open,
    onToggle: (event: Event) => options.toggle((event.target as HTMLDetailsElement).open) },
  h("summary", {}, "Advanced", quick ? " · Quick test" : ""),
  settings && training ? h("fieldset", { disabled, class: "advanced-settings" },
    historical ? h("p", { class: "muted" }, "Recorded launch settings · read only") : null,
    select("mode", "Run mode", settings.mode, [["standard", "Standard"], ["quick_test", "Quick test"]], value => options.mode(value as OptimizationAgentSettings["mode"])),
    quick ? h("p", { class: "diagnostic-notice" }, "Quick test is diagnostic: one iteration, up to four Agent turns, eight edits and 64 training rows. No final holdout or promotion. The two-minute ceiling covers training, not total runtime.") : null,
    field("Agent objective (optional)", h("textarea", { id: id("objective"), value: settings.objective, rows: 3, maxlength: 4000, disabled,
      onInput: (event: Event) => edit(next => { next.objective = (event.target as HTMLTextAreaElement).value; }) }), "Cannot change evidence access, acceptance rules or execution budgets."),
    h("div", { class: "advanced-grid" },
      number("iterations", "Maximum iterations", settings.maximumIterations, quick ? 1 : 10, value => edit(next => { next.maximumIterations = value; }), 1, 1, quick),
      number("turns", "Agent turns per iteration", settings.maximumAgentTurnsPerIteration, quick ? 4 : 32, value => edit(next => { next.maximumAgentTurnsPerIteration = value; }), settings.analysisProtocol === 2 ? 3 : 1),
      number("edits", "Row additions + removals · whole run", settings.maximumRowChanges, quick ? 8 : 5_000, value => edit(next => { next.maximumRowChanges = value; })),
      number("concurrency", "Generation requests in flight", settings.generationConcurrency, 16, value => edit(next => { next.generationConcurrency = value; }))),
    h("fieldset", {}, h("legend", {}, "Training"), h("div", { class: "advanced-grid" },
      select("device", "Device", training.device, [["auto", "Auto"], ["cpu", "CPU"], ["cuda", "CUDA"]], value => edit(next => { next.training.device = value as typeof training.device; })),
      number("epochs", "Maximum epochs", training.maximumEpochs, quick ? 1 : 10, value => edit(next => { next.training.maximumEpochs = value; }), 1, 1, quick),
      number("batch", "Batch size", training.batchSize, 256, value => edit(next => { next.training.batchSize = value; })),
      number("learning-rate", "Learning rate", training.learningRateNanos, 1_000_000, value => edit(next => { next.training.learningRateNanos = value; }), 1, 1_000_000_000),
      number("seconds", "Training seconds per iteration", training.maximumSecondsPerIteration, quick ? 120 : 21_600, value => edit(next => { next.training.maximumSecondsPerIteration = value; })),
      quick ? number("rows", "Maximum training rows", training.maximumTrainingRows ?? 64, 64, value => edit(next => { next.training.maximumTrainingRows = value; })) : null),
      h("p", { class: "muted" }, training.maximumTrainingRows === null ? "Train on the complete qualified dataset." : `Diagnostic training sample: at most ${training.maximumTrainingRows} rows, after full-population qualification.`, " Auto records the resolved device. Interrupted training consumes its allowance.")),
    roleBudgets("advisor"), roleBudgets("generation"),
    !budgets ? h("p", { class: "muted" }, "Assign both providers in project settings to set per-run ceilings.") : null,
    h("p", { class: "muted" }, "Provider ceilings include all iterations, retries and uncertain reservations. They are not actual usage or spend; unpriced costs remain unknown."),
    options.error ? h("p", { class: "danger", role: "alert" }, options.error) : null)
    : h("p", { class: "muted" }, historical ? "No Agent settings were recorded for this launch." : "Loading run settings…"));
}
