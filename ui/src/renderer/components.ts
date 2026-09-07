import { h, type Child } from "./dom.js";
import { delta, metricInfo, score } from "./catalog.js";

const paths: Record<string, string[]> = {
  models: ["M4 7 12 3l8 4-8 4-8-4Z", "m4 12 8 4 8-4", "m4 17 8 4 8-4"],
  runs: ["M8 3v4M16 3v4M4 11h16", "M5 5h14v16H5z", "m9 16 2 2 4-4"],
  benchmark: ["M4 20h16M7 16v-5M12 16V5M17 16V8"],
  project: ["M3 6h7l2 3h9v11H3z"],
  arrow: ["M5 12h14m-5-5 5 5-5 5"],
  back: ["M19 12H5m5-5-5 5 5 5"],
  search: ["M15 15l5 5", "M10.5 3a7.5 7.5 0 1 0 0 15 7.5 7.5 0 0 0 0-15"],
  check: ["m5 12 4 4L19 6"],
  lock: ["M6 10h12v11H6z", "M8 10V6a4 4 0 0 1 8 0v4"],
  help: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20", "M9 8a3 3 0 1 1 4 3c-1 1-1 2-1 3M12 17h.01"],
  copy: ["M9 9h11v12H9z", "M5 15H3V3h12v2"],
  close: ["m6 6 12 12M18 6 6 18"],
  sun: ["M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8", "M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5"],
  moon: ["M20 15.5A9 9 0 0 1 8.5 4 9 9 0 1 0 20 15.5Z"],
  refresh: ["M20 7v5h-5M4 17v-5h5", "M6 7a7 7 0 0 1 12 0l2 5M4 12l2 5a7 7 0 0 0 12 0"],
  sidebar: ["M3 4h18v16H3zM9 4v16"],
};
export function icon(name: string): HTMLElement { return h("svg", { viewBox: "0 0 24 24", "aria-hidden": "true", class: "icon" }, ...(paths[name] ?? paths.models!).map(d => h("path", { d }))); }
export function button(label: string, action: () => void, kind = "secondary", symbol?: string): HTMLButtonElement {
  return h("button", { type: "button", class: `button ${kind}`, onClick: action }, symbol ? icon(symbol) : null, label) as HTMLButtonElement;
}
export function tag(label: string, tone = "neutral"): HTMLElement { return h("span", { class: `tag ${tone}` }, label); }
export function status(label: string, tone = "neutral"): HTMLElement { return h("span", { class: `status ${tone}` }, h("span", { class: "status-dot", "aria-hidden": "true" }), label); }
export function pageHeader(title: string, description: string, action?: Child): HTMLElement {
  return h("header", { class: "page-heading" }, h("div", {}, h("h1", { tabindex: "-1" }, title), h("p", {}, description)), action ?? null);
}
export function sectionHeader(title: string, extra?: Child): HTMLElement { return h("div", { class: "section-heading" }, h("h2", {}, title), extra ?? null); }
export function empty(title: string, description: string, action?: Child): HTMLElement { return h("div", { class: "empty-state" }, icon("search"), h("h2", {}, title), h("p", {}, description), action ?? null); }
export function metricHeader(key: string, help: (key: string) => void, direction?: string): HTMLElement {
  const info = metricInfo(key);
  return h("button", { type: "button", class: "metric-heading", onClick: () => help(key), "aria-label": `About ${info.label}` }, info.label, h("span", { class: "metric-subtitle" }, info.short + (direction === "lower_is_better" ? " ↓" : direction === "higher_is_better" ? " ↑" : "")));
}
export function scoreStack(value: number | undefined, baseline: number | undefined, key: string, direction = "higher_is_better"): HTMLElement {
  const diff = value !== undefined && baseline !== undefined ? value - baseline : undefined;
  const tone = diff === undefined || Math.abs(diff) < 1e-12 ? "muted" : (direction === "lower_is_better" ? -diff : diff) > 0 ? "success" : "danger";
  return h("div", { class: "score-stack" }, h("span", { class: "score" }, score(value, key)), h("small", { class: tone }, delta(diff, key)));
}
export function facts(entries: [string, Child][]): HTMLElement { return h("dl", { class: "facts" }, ...entries.map(([key, value]) => h("div", {}, h("dt", {}, key), h("dd", {}, value)))); }
export function copyField(value: string, copy: (value: string) => void): HTMLElement { return h("div", { class: "copy-field" }, h("code", {}, value), button("Copy", () => copy(value), "ghost small", "copy")); }
export function details(title: string, content: Child, open = false): HTMLElement { return h("details", { class: "disclosure", open }, h("summary", {}, title), h("div", { class: "disclosure-content" }, content)); }
export function selectControl(id: string, label: string, options: [string, string][], value: string, change: (value: string) => void): HTMLElement {
  return h("label", { class: "select-control", for: id }, h("span", {}, label), h("select", { id, value, onChange: (e: Event) => change((e.target as HTMLSelectElement).value) }, ...options.map(([v, text]) => h("option", { value: v }, text))));
}
export function tabs(items: [string, string][], current: string, change: (tab: string) => void): HTMLElement {
  return h("div", { class: "tabs", role: "tablist", "aria-label": "Detail sections" }, ...items.map(([key, label]) => h("button", {
    id: `tab-${key}`, class: key === current ? "active" : "", type: "button", role: "tab", "aria-selected": String(key === current), "aria-controls": "detail-panel", tabindex: key === current ? "0" : "-1",
    onClick: () => change(key), onKeyDown: (e: KeyboardEvent) => {
      const i = items.findIndex(t => t[0] === key);
      if (["ArrowRight", "ArrowLeft", "Home", "End"].includes(e.key)) {
        e.preventDefault(); const next = e.key === "Home" ? items[0] : e.key === "End" ? items.at(-1) : items[(i + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length];
        if (next) { change(next[0]); document.getElementById(`tab-${next[0]}`)?.focus(); }
      }
    },
  }, label)));
}
