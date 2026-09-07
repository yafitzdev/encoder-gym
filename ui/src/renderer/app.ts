import recorded from "../evidence/nomos-snapshot.json";
import type { WorkspaceSnapshot } from "../workspace.js";
import type { Actions, Location, Page } from "./actions.js";
import { candidateName, candidateRows, dateLabel, initialFilter, metricInfo, runName, setupId, type CandidateRow } from "./catalog.js";
import { button, icon, tag } from "./components.js";
import { renderCandidate, renderCompare, renderRun, type DetailState } from "./detail-pages.js";
import { h } from "./dom.js";
import { renderModels, type ModelPageState } from "./models-page.js";
import { NavigationHistory } from "./state.js";
import { renderBaseline, renderBenchmarks, renderProject, renderRuns } from "./workspace-pages.js";

const requireElement = (id: string) => { const element = document.getElementById(id); if (!element) throw new Error(`Missing #${id}`); return element; };

export function mount(): void {
  let workspace = recorded as WorkspaceSnapshot;
  const main = requireElement("page");
  const nav = requireElement("project-nav");
  const source = requireElement("source-state");
  const history = new NavigationHistory();
  const modelState: ModelPageState = { filter: initialFilter(), selected: new Set(), suiteIndex: 0 };
  const detailState = new Map<string, DetailState>();
  let compared: CandidateRow[] = [];
  let loading = false;
  let notificationTimer: ReturnType<typeof setTimeout>;
  const notify = (text: string) => { const toast = requireElement("toast"); toast.textContent = text; toast.hidden = false; clearTimeout(notificationTimer); notificationTimer = setTimeout(() => { toast.hidden = true; }, 6500); };
  const copy = async (text: string) => {
    try { if (window.encoderGym) await window.encoderGym.copyText(text); else await navigator.clipboard.writeText(text); notify("Copied to clipboard"); }
    catch { notify("Clipboard is unavailable. Select and copy the text directly."); }
  };
  function help(key?: string): void {
    const dialog = requireElement("help-dialog") as HTMLDialogElement;
    const info = key ? metricInfo(key) : undefined;
    dialog.replaceChildren(h("div", { class: "dialog-heading" }, h("h2", { id: "help-title" }, info ? info.label : "Reading Encoder Gym"), button("Close", () => dialog.close(), "ghost small", "close")),
      info ? h("div", { class: "guide-content" }, tag(info.short), h("p", {}, info.description), h("p", {}, "The change is measured against the matching baseline evaluation. The recorded check also considers the required threshold.")) : h("div", { class: "guide-content" },
        ...[["Baseline", "The encoder you are trying to improve. It stays visible as the reference for every comparison."], ["Candidate", "A trained or transformed model. Select its name to see exactly how it compares and which checks it passed."], ["Run", "One bounded experiment that creates and evaluates candidates. Open a run for training configuration, execution history, and budgets."], ["Evaluation setup", "The exact benchmarks, metric contract, and agent policy. Candidates from different setups are grouped separately."], ["Result", "Green numbers show an improvement. Passing development requires all recorded checks; replacing the baseline also requires separate final acceptance."]].map(([term, description]) => h("section", {}, h("h3", {}, term), h("p", {}, description))),
        h("div", { class: "guide-route" }, "Start with the baseline → scan candidates → open a result.")),
    );
    dialog.showModal();
  }
  function navigate(location: Location): void {
    requireElement("toast").hidden = true;
    requireElement("app-shell").classList.remove("mobile-sidebar-open");
    if (location.page === "models" && location.id) modelState.filter.setup = location.id;
    history.remember(location, main.scrollTop);
    render(); main.scrollTop = 0;
    main.querySelector<HTMLElement>("h1")?.focus({ preventScroll: true });
  }
  async function load(choose: boolean): Promise<void> {
    if (loading) return;
    if (!window.encoderGym?.loadWorkspace) { notify("The browser is showing the recorded preview. Open the desktop app to connect a local workspace."); return; }
    loading = true; updateSource();
    try {
      const next = choose ? await window.encoderGym.chooseWorkspace() : await window.encoderGym.loadWorkspace();
      if (!next) return;
      workspace = next; compared = []; modelState.selected.clear(); modelState.filter = initialFilter();
      if (["candidate", "run", "compare"].includes(history.current.page)) navigate({ page: "models" }); else render();
      notify(`Loaded ${candidateRows(workspace).length} candidates from local journals.`);
    } catch (error) { notify(`Could not load workspace: ${error instanceof Error ? error.message : String(error)}`); }
    finally { loading = false; updateSource(); }
  }
  const actions: Actions = {
    navigate, render: () => render(), copy: text => { void copy(text); }, help, notify,
    refresh: () => { void load(false); }, connect: () => { void load(true); },
    compare: rows => {
      if (!rows.length || rows.length > 3 || rows.some(r => setupId(r.run) !== setupId(rows[0]!.run))) { notify("Select 1–3 candidates from the same evaluation setup."); return; }
      compared = rows; navigate({ page: "compare" });
    },
  };
  function updateSource(): void {
    source.replaceChildren(button(loading ? "Reading evidence…" : workspace.source === "local" ? "Local journals" : "Recorded snapshot", () => navigate({ page: "project" }), "source-button"), h("span", {}, dateLabel(workspace.capturedAt)));
    requireElement("workspace-name").textContent = workspace.name;
    requireElement("workspace-mode").textContent = workspace.source === "local" ? "Connected · read only" : "Evidence preview";
    const reload = requireElement("reload-evidence") as HTMLButtonElement;
    reload.disabled = loading; reload.setAttribute("aria-label", loading ? "Reading evidence" : "Reload workspace evidence");
  }
  function render(): void {
    const current = history.current;
    const focus = document.activeElement as HTMLInputElement | null;
    const focusId = focus?.id;
    const selection = focus instanceof HTMLInputElement && ["text", "search"].includes(focus.type) ? [focus.selectionStart, focus.selectionEnd] : null;
    const scroll = main.scrollTop;
    const activePage = ["candidate", "baseline", "compare"].includes(current.page) ? "models" : current.page === "run" ? "runs" : current.page;
    const names: [Page, string, string][] = [["models", "Models", "models"], ["runs", "Runs", "runs"], ["benchmarks", "Benchmarks", "benchmark"], ["project", "Project", "project"]];
    nav.replaceChildren(...names.map(([page, label, symbol]) => h("button", { class: "nav-item" + (page === activePage ? " active" : ""), type: "button", "aria-current": page === activePage ? "page" : null, onClick: () => navigate({ page }) }, icon(symbol), label, page === "models" ? h("span", { class: "nav-count" }, candidateRows(workspace).length) : page === "runs" ? h("span", { class: "nav-count" }, workspace.runs.length) : null)));
    const foundCandidate = candidateRows(workspace).find(r => r.candidate.id === current.id);
    const foundRun = workspace.runs.find(r => r.id === current.id);
    const pageTitle = current.page === "candidate" ? foundCandidate ? candidateName(foundCandidate.candidate) : "Candidate" : current.page === "run" ? foundRun ? runName(foundRun) : "Run" : names.find(n => n[0] === current.page)?.[1] ?? (current.page === "baseline" ? "Baseline" : "Compare models");
    requireElement("breadcrumb").textContent = workspace.name + " / " + pageTitle;
    document.title = `${pageTitle} · Encoder Gym`;
    const detailKey = `${current.id ?? ""}:${current.runId ?? ""}`;
    const exactCandidate = current.runId ? workspace.runs.find(r => r.id === current.runId)?.candidates.find(c => c.id === current.id) : foundCandidate?.candidate;
    const detail = detailState.get(detailKey) ?? { failedOnly: exactCandidate?.development.some(d => d.checks.some(g => !g.passed)) ?? false };
    if (current.id) detailState.set(detailKey, detail);
    let content: HTMLElement;
    if (current.page === "models") content = renderModels(workspace, modelState, actions);
    else if (current.page === "candidate") content = renderCandidate(workspace, current.id ?? "", current.tab ?? "results", detail, actions, current.runId);
    else if (current.page === "run") content = renderRun(workspace, current.id ?? "", current.tab ?? "overview", actions);
    else if (current.page === "runs") content = renderRuns(workspace, actions);
    else if (current.page === "benchmarks") content = renderBenchmarks(workspace, actions);
    else if (current.page === "baseline") content = renderBaseline(workspace, actions);
    else if (current.page === "compare") content = renderCompare(workspace, compared, actions);
    else content = renderProject(workspace, actions);
    main.replaceChildren(content); main.scrollTop = scroll;
    if (focusId) { const target = document.getElementById(focusId); target?.focus({ preventScroll: true }); if (selection && target instanceof HTMLInputElement) target.setSelectionRange(selection[0] ?? null, selection[1] ?? null); }
    (requireElement("navigate-back") as HTMLButtonElement).disabled = !history.canNavigate(-1);
    (requireElement("navigate-forward") as HTMLButtonElement).disabled = !history.canNavigate(1);
    updateSource();
  }
  for (const [id, offset] of [["navigate-back", -1], ["navigate-forward", 1]] as const) requireElement(id).addEventListener("click", () => { const entry = history.move(offset, main.scrollTop); if (entry) { render(); main.scrollTop = entry.scroll; } });
  requireElement("reload-evidence").addEventListener("click", actions.refresh);
  requireElement("open-guide").addEventListener("click", () => help());
  let theme = localStorage.getItem("encoder-gym:theme") ?? "dark";
  const setTheme = () => { document.documentElement.dataset.theme = theme; const b = requireElement("theme-toggle"); b.replaceChildren(icon(theme === "dark" ? "sun" : "moon")); b.setAttribute("aria-label", theme === "dark" ? "Use light theme" : "Use dark theme"); };
  requireElement("theme-toggle").addEventListener("click", () => { theme = theme === "dark" ? "light" : "dark"; localStorage.setItem("encoder-gym:theme", theme); setTheme(); }); setTheme();
  const shell = requireElement("app-shell");
  const updateSidebar = () => requireElement("sidebar-menu").setAttribute("aria-expanded", String(window.innerWidth <= 600 ? shell.classList.contains("mobile-sidebar-open") : !shell.classList.contains("sidebar-collapsed")));
  requireElement("sidebar-menu").addEventListener("click", () => { shell.classList.toggle(window.innerWidth <= 600 ? "mobile-sidebar-open" : "sidebar-collapsed"); updateSidebar(); });
  window.addEventListener("resize", updateSidebar); updateSidebar();
  const resizer = requireElement("sidebar-resizer");
  resizer.addEventListener("keydown", event => { const e = event as KeyboardEvent; if (!["ArrowLeft", "ArrowRight"].includes(e.key)) return; e.preventDefault(); const width = parseFloat(getComputedStyle(shell).getPropertyValue("--sidebar-width")); shell.style.setProperty("--sidebar-width", `${Math.min(320, Math.max(176, width + (e.key === "ArrowRight" ? 8 : -8)))}px`); });
  resizer.addEventListener("pointerdown", event => { const e = event as PointerEvent; resizer.setPointerCapture(e.pointerId); });
  resizer.addEventListener("pointermove", event => { const e = event as PointerEvent; if (resizer.hasPointerCapture(e.pointerId)) shell.style.setProperty("--sidebar-width", `${Math.min(320, Math.max(176, e.clientX))}px`); });
  for (const b of document.querySelectorAll<HTMLButtonElement>("[data-window-action]")) b.addEventListener("click", () => { void window.encoderGym?.windowAction(b.dataset.windowAction as "minimize" | "maximize" | "close"); });
  document.addEventListener("keydown", event => {
    if (event.key === "/" && !(event.target instanceof HTMLInputElement) && !document.querySelector("dialog[open]")) { const search = document.getElementById("candidate-search"); if (search) { event.preventDefault(); search.focus(); } }
    if (event.altKey && ["ArrowLeft", "ArrowRight"].includes(event.key)) { event.preventDefault(); requireElement(event.key === "ArrowLeft" ? "navigate-back" : "navigate-forward").click(); }
  });
  render();
}
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount); else mount();
