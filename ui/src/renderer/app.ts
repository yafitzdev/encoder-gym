import type { WorkspaceSnapshot } from "../workspace.js";
import { ProjectSelection, type OpenedProject, type ProjectCollection } from "../projects.js";
import type { Actions, Location, Page, ProjectActions } from "./actions.js";
import { candidateName, candidateRows, dateLabel, initialFilter, metricInfo, runName, setupId } from "./catalog.js";
import { button, empty, icon, tag } from "./components.js";
import { renderCandidate, renderCompare, renderRun, type DetailState } from "./detail-pages.js";
import { h } from "./dom.js";
import { renderModels, type ModelPageState } from "./models-page.js";
import { NavigationHistory } from "./state.js";
import { renderBaseline, renderBenchmarks, renderRuns } from "./workspace-pages.js";
import { renderProjectSettings, renderProjectState, renderWelcome } from "./project-pages.js";
import { previewBridge } from "./preview-bridge.js";
import { newProjectDialog, importDatasetDialog } from "./onboarding.js";
import { renderDatasets, renderManagedSettings } from "./managed-pages.js";

const element = (id: string): HTMLElement => { const found = document.getElementById(id); if (!found) throw new Error("Missing #" + id); return found; };
interface ProjectView { history: NavigationHistory; models: ModelPageState; details: Map<string, DetailState> }
const newView = (): ProjectView => ({ history: new NavigationHistory(), models: { filter: initialFilter(), selected: new Set(), suiteIndex: 0 }, details: new Map() });

export function mount(): void {
  const bridge = window.encoderGym ?? previewBridge();
  if (!window.encoderGym) document.documentElement.classList.add("browser-preview");
  const main = element("page"), shell = element("app-shell"), selection = new ProjectSelection();
  let collection: ProjectCollection = { version: 1, selectedId: null, projects: [] };
  let opened: OpenedProject | undefined, loading = true, collectionError: string | undefined;
  const views = new Map<string, ProjectView>();
  let view = newView(), timer: ReturnType<typeof setTimeout>;
  const workspace = (): WorkspaceSnapshot | undefined => opened?.content.state === "ready" ? { ...opened.content.workspace, name: opened.project.name } : undefined;
  const notify = (message: string) => { const toast = element("toast"); toast.textContent = message; toast.hidden = false; clearTimeout(timer); timer = setTimeout(() => { toast.hidden = true; }, 6000); };
  const message = (error: unknown) => error instanceof Error ? error.message : String(error);
  const updateSidebar = () => element("sidebar-menu").setAttribute("aria-expanded", String(window.innerWidth <= 600 ? shell.classList.contains("mobile-sidebar-open") : !shell.classList.contains("sidebar-collapsed")));
  const focusHeading = () => main.querySelector<HTMLElement>("h1")?.focus({ preventScroll: true });

  function navigate(location: Location): void {
    element("toast").hidden = true;
    shell.classList.remove("mobile-sidebar-open"); updateSidebar();
    if (location.page === "models" && location.id) view.models.filter.setup = location.id;
    view.history.remember(location, main.scrollTop);
    render(); main.scrollTop = 0; focusHeading();
  }
  async function selectProject(id: string): Promise<void> {
    if (selection.selectedId) views.set(selection.selectedId, view);
    const changed = selection.selectedId !== id;
    view = views.get(id) ?? newView(); views.set(id, view);
    if (changed) { opened = undefined; element("toast").hidden = true; view.history.remember({ page: "models" }, 0); main.scrollTop = 0; }
    collection.selectedId = id; loading = true;
    const pending = selection.open(id, projectId => bridge.selectProject(projectId));
    render();
    try {
      const next = await pending;
      if (!next) return;
      opened = next; loading = false; render();
      if (changed) { main.scrollTop = 0; focusHeading(); }
    } catch (error) {
      if (selection.selectedId !== id) return;
      const project = collection.projects.find(p => p.id === id);
      opened = project ? { project, content: { state: "error", message: message(error) } } : undefined;
      loading = false; render();
    }
  }
  async function refreshCollection(): Promise<void> {
    loading = true; collectionError = undefined; render();
    try {
      collection = await bridge.getProjects();
      if (collection.selectedId) { await selectProject(collection.selectedId); return; }
      selection.invalidate(null); opened = undefined;
    } catch (error) { collectionError = message(error); }
    loading = false; render();
  }
  async function changeCollection(operation: () => Promise<ProjectCollection | null>): Promise<void> {
    loading = true; render();
    try {
      const next = await operation();
      if (!next) { loading = false; render(); return; }
      collection = next;
      if (next.selectedId) await selectProject(next.selectedId);
      else { selection.invalidate(null); opened = undefined; loading = false; render(); }
    } catch (error) { loading = false; render(); notify(message(error)); }
  }
  function projectDialog(removing: boolean): void {
    const project = collection.projects.find(p => p.id === selection.selectedId);
    if (!project) return;
    const dialog = element("project-dialog") as HTMLDialogElement;
    const input = h("input", { id: "project-name-input", class: "text-input", value: project.name, maxlength: "120", required: true }) as HTMLInputElement;
    const error = h("p", { class: "form-error", role: "alert" });
    const submit = button(removing ? "Remove entry" : "Save name", () => {}, removing ? "secondary" : "primary");
    submit.type = "submit";
    const form = h("form", { onSubmit: async (event: Event) => {
      event.preventDefault(); submit.disabled = true; error.textContent = "";
      try {
        collection = removing ? await bridge.forgetProject(project.id) : await bridge.renameProject(project.id, input.value);
        if (removing) views.delete(project.id);
        dialog.close();
        if (removing) {
          if (collection.selectedId) await selectProject(collection.selectedId);
          else { selection.invalidate(null); opened = undefined; loading = false; render(); }
        } else {
          if (opened?.project.id === project.id) opened = { ...opened, project: collection.projects.find(p => p.id === project.id)! };
          render();
        }
        notify(removing ? "Project entry removed. All project files are untouched." : "Project renamed.");
      } catch (failure) { error.textContent = message(failure); submit.disabled = false; }
    } },
      h("h2", { id: "project-dialog-title" }, removing ? "Remove " + project.name + "?" : "Rename project"),
      h("p", { class: "section-note" }, removing ? "Only the entry in Encoder Gym will be removed. No files, models, datasets, or experiment history will be deleted." : "This changes the name in Encoder Gym, not the folder name or historical experiment records."),
      removing ? null : h("label", { class: "form-field", for: "project-name-input" }, "Project name", input),
      error, h("div", { class: "dialog-actions" }, button("Cancel", () => dialog.close(), "secondary"), submit));
    dialog.replaceChildren(form); dialog.showModal();
    if (!removing) { input.focus(); input.select(); }
  }
  function help(key?: string): void {
    const dialog = element("help-dialog") as HTMLDialogElement, info = key ? metricInfo(key) : undefined;
    dialog.replaceChildren(h("div", { class: "dialog-heading" }, h("h2", { id: "help-title" }, info ? info.label : "Reading Encoder Gym"), button("Close", () => dialog.close(), "ghost small", "close")),
      info ? h("div", { class: "guide-content" }, tag(info.short), h("p", {}, info.description), h("p", {}, "Changes use this candidate's matching baseline. Passing also depends on the requirements recorded in its evaluation contract.")) :
        h("div", { class: "guide-content" }, ...[
          ["Project folder", "One encoder's workspace. Select a folder in the sidebar to see its baseline, candidates, and runs. Other projects remain separate."],
          ["Baseline", "The reference encoder you are trying to improve. Its scores appear alongside candidates tested using the same setup."],
          ["Candidate", "A trained or transformed model. Open its name to inspect results, requirements, and how it was produced."],
          ["Run", "An experiment that produces and evaluates candidates. It owns configuration, execution history, budgets, and provenance."],
          ["Evaluation setup", "The exact baseline, benchmarks, metric contract, and evaluation policy. Different setups are compared separately."],
          ["Development result", "A score gain can still miss a required threshold. Passing development is separate from final acceptance or replacing the baseline."]
        ].map(([term, description]) => h("section", {}, h("h3", {}, term), h("p", {}, description)))));
    dialog.showModal();
  }
  const projects: ProjectActions = {
    create: () => newProjectDialog(element("project-dialog") as HTMLDialogElement, bridge, async next => { collection = next; if (next.selectedId) await selectProject(next.selectedId); notify("Project created. The source checkpoint is unchanged."); }),
    openManaged: () => { void changeCollection(() => bridge.openManagedProject()); },
    importDataset: () => {
      const id = selection.selectedId; if (!id || !workspace()?.managed) return;
      importDatasetDialog(element("project-dialog") as HTMLDialogElement, bridge, id, async result => {
        if (selection.selectedId !== id) return;
        opened = result; navigate({ page: "datasets" }); notify("Dataset ready in this project. The original file is unchanged.");
      });
    },
    verify: () => {
      const id = selection.selectedId; if (!id || loading) return;
      loading = true; render();
      void bridge.verifyManagedProject(id).then(result => {
        if (selection.selectedId !== id) return;
        opened = result; loading = false; render(); notify("Project files and record counts verified.");
      }, error => {
        if (selection.selectedId !== id) return;
        const project = collection.projects.find(p => p.id === id)!;
        opened = { project, content: { state: "error", message: message(error) } }; loading = false; render();
      });
    },
    addFolder: () => { void changeCollection(() => bridge.addProjectFolder()); },
    openExample: () => { void changeCollection(() => bridge.openExample()); },
    select: id => { void selectProject(id); },
    rename: () => projectDialog(false), forget: () => projectDialog(true),
    relocate: () => { const id = selection.selectedId; if (id) void changeCollection(() => bridge.relocateProject(id)); },
  };
  const actions: Actions = {
    navigate, render, help, notify, connect: projects.addFolder,
    refresh: () => { if (selection.selectedId) void selectProject(selection.selectedId); else void refreshCollection(); },
    copy: value => { void bridge.copyText(value).then(() => notify("Copied to clipboard"), error => notify("Copy failed: " + message(error))); },
    compare: rows => {
      if (!rows.length || rows.length > 3 || rows.some(row => setupId(row.run) !== setupId(rows[0]!.run))) { notify("Select 1–3 candidates from the same evaluation setup."); return; }
      navigate({ page: "compare", candidateIds: rows.map(row => row.candidate.id) });
    },
  };
  const pages: [Page, string, string][] = [["models", "Models", "models"], ["datasets", "Datasets", "project"], ["runs", "Runs", "runs"], ["benchmarks", "Benchmarks", "benchmark"], ["project", "Project settings", "project"]];
  function render(): void {
    const current = view.history.current, data = workspace();
    const project = collection.projects.find(p => p.id === selection.selectedId);
    const activePage = ["candidate", "baseline", "compare"].includes(current.page) ? "models" : current.page === "run" ? "runs" : current.page;
    const focus = document.activeElement, focusId = focus?.id;
    const caret = focus instanceof HTMLInputElement && ["text", "search"].includes(focus.type) ? [focus.selectionStart, focus.selectionEnd] : undefined;
    const scroll = main.scrollTop;
    element("project-nav").replaceChildren(...collection.projects.map(p => h("section", { class: "project-folder" + (p.id === project?.id ? " selected-project" : "") },
      h("button", { type: "button", class: "project-folder-button", title: p.source.kind === "folder" ? p.source.path : "Recorded example", "data-project-id": p.id, "aria-expanded": String(p.id === project?.id), onClick: () => {
        if (p.id === selection.selectedId) navigate({ page: "models" }); else projects.select(p.id);
      } }, icon("project"), h("span", {}, p.name), p.source.kind === "example" ? h("small", {}, "Example") : !p.source.workspaceId ? h("small", {}, "Legacy") : null),
      p.id === project?.id ? h("div", { class: "project-pages" }, ...pages.filter(([page]) => page !== "datasets" || (p.source.kind === "folder" && p.source.workspaceId)).map(([page, label, symbol]) => h("button", { type: "button", class: "nav-item" + (page === activePage ? " active" : ""), "aria-current": page === activePage ? "page" : null, "data-page": page, onClick: () => navigate({ page }) }, icon(symbol), label,
        data && ["models", "runs"].includes(page) ? h("span", { class: "nav-count" }, page === "models" ? candidateRows(data).length + 1 : data.runs.length) : null))) : null)));
    const candidate = data ? candidateRows(data).find(r => r.candidate.id === current.id)?.candidate : undefined;
    const run = data?.runs.find(r => r.id === current.id);
    const title = !project ? "Projects" : current.page === "candidate" ? candidate ? candidateName(candidate) : "Candidate not found" :
      current.page === "run" ? run ? runName(run) : "Run not found" :
      pages.find(p => p[0] === current.page)?.[1] ?? (current.page === "baseline" ? "Baseline" : "Compare models");
    element("breadcrumb").textContent = project ? project.name + " / " + title : "Encoder Gym";
    document.title = title + " · Encoder Gym";
    element("source-state").replaceChildren(...(project ? [button(loading ? "Reading…" : opened?.content.state === "error" ? "Evidence unavailable" : data?.managed ? "Managed workspace" : data?.source === "recorded" ? "Recorded example" : data ? "Legacy journals" : "No records yet", () => navigate({ page: "project" }), "source-button"),
      ...(data ? [h("span", {}, dateLabel(data.capturedAt))] : [])] : []));
    const reload = element("reload-evidence") as HTMLButtonElement; reload.hidden = !project; reload.disabled = loading;
    let content: HTMLElement;
    if (collectionError) content = h("div", { class: "page-content" }, empty("Couldn't open the project collection", collectionError, button("Try again", () => { void refreshCollection(); }, "primary")));
    else if (!project) content = loading ? h("div", { class: "page-content", role: "status" }, "Opening project collection…") : renderWelcome(projects);
    else if (current.page === "project") content = data?.managed ? renderManagedSettings(project, data.managed, actions, projects) : renderProjectSettings(project, opened, actions, projects);
    else if (!data) content = renderProjectState(project, opened, actions, projects, current.page);
    else {
      const detailKey = (current.id ?? "") + ":" + (current.runId ?? "");
      const exactCandidate = current.runId ? data.runs.find(r => r.id === current.runId)?.candidates.find(c => c.id === current.id) : candidateRows(data).find(r => r.candidate.id === current.id)?.candidate;
      const detail = view.details.get(detailKey) ?? { failedOnly: exactCandidate?.development.some(d => d.checks.some(g => !g.passed)) ?? false };
      if (current.id) view.details.set(detailKey, detail);
      if (current.page === "models") content = renderModels(data, view.models, actions);
      else if (current.page === "datasets" && data.managed) content = renderDatasets(data.managed, actions, projects);
      else if (current.page === "candidate") content = renderCandidate(data, current.id ?? "", current.tab ?? "results", detail, actions, current.runId);
      else if (current.page === "run") content = renderRun(data, current.id ?? "", current.tab ?? "overview", actions);
      else if (current.page === "runs") content = renderRuns(data, actions);
      else if (current.page === "benchmarks") content = renderBenchmarks(data, actions);
      else if (current.page === "baseline") content = renderBaseline(data, actions);
      else content = renderCompare(data, candidateRows(data).filter(r => current.candidateIds?.includes(r.candidate.id)), actions);
    }
    main.replaceChildren(content); main.scrollTop = scroll;
    if (focusId) { const target = document.getElementById(focusId); target?.focus({ preventScroll: true }); if (caret && target instanceof HTMLInputElement) target.setSelectionRange(caret[0] ?? null, caret[1] ?? null); }
    (element("navigate-back") as HTMLButtonElement).disabled = !project || !view.history.canNavigate(-1);
    (element("navigate-forward") as HTMLButtonElement).disabled = !project || !view.history.canNavigate(1);
  }
  for (const [id, offset] of [["navigate-back", -1], ["navigate-forward", 1]] as const) element(id).addEventListener("click", () => { const entry = view.history.move(offset, main.scrollTop); if (entry) { render(); main.scrollTop = entry.scroll; focusHeading(); } });
  element("reload-evidence").addEventListener("click", actions.refresh);
  element("open-guide").addEventListener("click", () => help());
  element("add-project").addEventListener("click", projects.create);
  element("open-project").addEventListener("click", projects.openManaged);
  element("open-legacy-project").addEventListener("click", projects.addFolder);
  const readPreference = (key: string) => { try { return localStorage.getItem(key); } catch { return null; } };
  const savePreference = (key: string, value: string) => { try { localStorage.setItem(key, value); } catch { /* Preferences must not prevent using the app. */ } };
  let theme = readPreference("encoder-gym:theme") === "light" ? "light" : "dark";
  const setTheme = () => { document.documentElement.dataset.theme = theme; const b = element("theme-toggle"); b.replaceChildren(icon(theme === "dark" ? "sun" : "moon")); b.setAttribute("aria-label", theme === "dark" ? "Use light theme" : "Use dark theme"); };
  element("theme-toggle").addEventListener("click", () => { theme = theme === "dark" ? "light" : "dark"; savePreference("encoder-gym:theme", theme); setTheme(); }); setTheme();
  element("sidebar-menu").addEventListener("click", () => { shell.classList.toggle(window.innerWidth <= 600 ? "mobile-sidebar-open" : "sidebar-collapsed"); updateSidebar(); });
  window.addEventListener("resize", updateSidebar); updateSidebar();
  const resizer = element("sidebar-resizer");
  const setWidth = (value: number) => { const width = Math.min(340, Math.max(200, value)); shell.style.setProperty("--sidebar-width", width + "px"); resizer.setAttribute("aria-valuenow", String(width)); savePreference("encoder-gym:sidebar-width", String(width)); };
  const storedWidth = Number(readPreference("encoder-gym:sidebar-width")); if (storedWidth >= 200 && storedWidth <= 340) setWidth(storedWidth);
  resizer.setAttribute("aria-valuemin", "200"); resizer.setAttribute("aria-valuemax", "340");
  resizer.addEventListener("keydown", event => { const e = event as KeyboardEvent; if (!["ArrowLeft", "ArrowRight"].includes(e.key)) return; e.preventDefault(); setWidth(parseFloat(getComputedStyle(shell).getPropertyValue("--sidebar-width")) + (e.key === "ArrowRight" ? 8 : -8)); });
  resizer.addEventListener("pointerdown", event => { const e = event as PointerEvent; resizer.setPointerCapture(e.pointerId); });
  resizer.addEventListener("pointermove", event => { const e = event as PointerEvent; if (resizer.hasPointerCapture(e.pointerId)) setWidth(e.clientX); });
  for (const b of document.querySelectorAll<HTMLButtonElement>("[data-window-action]")) b.addEventListener("click", () => { void bridge.windowAction(b.dataset.windowAction as "minimize" | "maximize" | "close"); });
  document.addEventListener("keydown", event => {
    if (document.querySelector("dialog[open]")) return;
    if (event.key === "/" && !(event.target instanceof HTMLInputElement)) { const search = document.getElementById("candidate-search"); if (search) { event.preventDefault(); search.focus(); } }
    if (event.altKey && ["ArrowLeft", "ArrowRight"].includes(event.key)) { event.preventDefault(); element(event.key === "ArrowLeft" ? "navigate-back" : "navigate-forward").click(); }
  });
  void refreshCollection();
}
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount); else mount();
