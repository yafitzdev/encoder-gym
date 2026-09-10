import type { WorkspaceSnapshot } from "../workspace.js";
import type { ManagedOptimizationReport, ManagedOptimizationResult, ManagedRunStatus } from "../managed-control.js";
import { ProjectSelection, type OpenedProject, type ProjectCollection } from "../projects.js";
import type { Actions, Location, Page, ProjectActions } from "./actions.js";
import { candidateName, candidateRows, dateLabel, initialFilter, metricInfo, runName, setupId } from "./catalog.js";
import { button, failureNotice, icon, tag } from "./components.js";
import { renderCandidate, renderCompare, renderRun, type DetailState } from "./detail-pages.js";
import { h } from "./dom.js";
import { renderModels, type ModelPageState } from "./models-page.js";
import { NavigationHistory } from "./state.js";
import { renderBaseline, renderBenchmarks, renderRuns } from "./workspace-pages.js";
import { renderProjectSettings, renderProjectState, renderWelcome } from "./project-pages.js";
import { previewBridge } from "./preview-bridge.js";
import { newProjectDialog, importDatasetDialog } from "./onboarding.js";
import { renderDatasets, renderManagedSettings, type ProviderPageActions, type ProviderPageState } from "./managed-pages.js";
import { renderOptimization, type OptimizationPageActions, type OptimizationPageState } from "./optimization-page.js";
import { providerDialog } from "./provider-dialog.js";
import { scientificRuntimeDialog } from "./scientific-runtime-dialog.js";

const element = (id: string): HTMLElement => { const found = document.getElementById(id); if (!found) throw new Error("Missing #" + id); return found; };
interface ProjectView { history: NavigationHistory; models: ModelPageState; details: Map<string, DetailState>; optimization: OptimizationPageState; providers: ProviderPageState }
const newView = (): ProjectView => ({ history: new NavigationHistory(), models: { filter: initialFilter(), selected: new Set(), suiteIndex: 0 }, details: new Map(), optimization: { loading: false }, providers: { loading: false } });

export function mount(): void {
  const bridge = window.encoderGym ?? previewBridge();
  if (!window.encoderGym) document.documentElement.classList.add("browser-preview");
  const main = element("page"), shell = element("app-shell"), selection = new ProjectSelection();
  let collection: ProjectCollection = { version: 1, selectedId: null, projects: [] };
  let opened: OpenedProject | undefined, loading = true, collectionError: string | undefined;
  let collectionBusy = false, operationError: unknown, loadingMessage = "Opening project…";
  const views = new Map<string, ProjectView>();
  let view = newView(), timer: ReturnType<typeof setTimeout>;
  const workspace = (): WorkspaceSnapshot | undefined => opened?.content.state === "ready" ? { ...opened.content.workspace, name: opened.project.name } : undefined;
  const notify = (message: string) => { const toast = element("toast"); toast.textContent = message; toast.hidden = false; clearTimeout(timer); timer = setTimeout(() => { toast.hidden = true; }, 6000); };
  const message = (error: unknown) => error instanceof Error ? error.message : String(error);
  const runStatus = (value: ManagedOptimizationResult): ManagedRunStatus => {
    if (!value || typeof value !== "object" || typeof (value as Partial<ManagedRunStatus>).run_id !== "string" || typeof (value as Partial<ManagedRunStatus>).state !== "string") throw new Error("The optimizer returned an unreadable run status.");
    return value as ManagedRunStatus;
  };
  const runReport = (value: ManagedOptimizationResult): ManagedOptimizationReport => {
    const report = value as Partial<ManagedOptimizationReport>;
    if (!report || report.schema_version !== 1 || typeof report.run_id !== "string" || !Array.isArray(report.candidate_results) || !Array.isArray(report.known_evidence_limits)) throw new Error("The optimizer returned an unreadable run report.");
    return report as ManagedOptimizationReport;
  };
  const updateSidebar = () => element("sidebar-menu").setAttribute("aria-expanded", String(window.innerWidth <= 600 ? shell.classList.contains("mobile-sidebar-open") : !shell.classList.contains("sidebar-collapsed")));
  const focusHeading = () => main.querySelector<HTMLElement>("h1")?.focus({ preventScroll: true });
  const contentFocus = () => main.contains(document.activeElement) ? (document.activeElement as HTMLElement)?.id : undefined;
  const restorePlace = () => {
    const entry = view.history.bookmark;
    main.scrollTop = entry.scroll;
    const target = entry.focusId ? document.getElementById(entry.focusId) : null;
    if (target && main.contains(target)) target.focus({ preventScroll: true }); else focusHeading();
  };

  function navigate(location: Location): void {
    element("toast").hidden = true;
    shell.classList.remove("mobile-sidebar-open"); updateSidebar();
    if (location.page === "models" && location.id) view.models.filter.setup = location.id;
    view.history.remember(location, main.scrollTop, contentFocus());
    render(); main.scrollTop = 0; focusHeading();
    if (location.page === "project" && workspace()?.managed && !view.providers.status) void refreshProviders();
    if (["datasets", "runs", "benchmarks"].includes(location.page) && workspace()?.managed && !view.optimization.readiness && !view.optimization.loading) void refreshOptimization();
  }
  async function selectProject(id: string): Promise<void> {
    if (collectionBusy) return;
    if (selection.selectedId && collection.projects.some(project => project.id === selection.selectedId)) { view.history.capture(main.scrollTop, contentFocus()); views.set(selection.selectedId, view); }
    const changed = selection.selectedId !== id;
    view = views.get(id) ?? newView(); views.set(id, view);
    if (changed) { opened = undefined; operationError = undefined; element("toast").hidden = true; main.scrollTop = 0; }
    collection.selectedId = id; loading = true; loadingMessage = "Reading project files…";
    const pending = selection.open(id, projectId => bridge.selectProject(projectId));
    render();
    try {
      const next = await pending;
      if (!next) return;
      opened = next; loading = false; render();
      if (view.history.current.page === "project" && next.content.state === "ready" && next.content.workspace.managed && !view.providers.status) void refreshProviders();
      if (["datasets", "runs", "benchmarks"].includes(view.history.current.page) && next.content.state === "ready" && next.content.workspace.managed && !view.optimization.readiness && !view.optimization.loading) void refreshOptimization();
      if (changed) restorePlace();
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
    if (collectionBusy || loading) return;
    collectionBusy = true; operationError = undefined; loadingMessage = "Opening the selected folder…"; render();
    try {
      const next = await operation();
      collectionBusy = false;
      if (!next) { render(); return; }
      collection = next;
      if (next.selectedId) await selectProject(next.selectedId);
      else { selection.invalidate(null); opened = undefined; loading = false; render(); }
    } catch (error) { collectionBusy = false; loading = false; operationError = error; render(); main.scrollTop = 0; }
  }
  function projectDialog(removing: boolean): void {
    if (loading || collectionBusy) return;
    const project = collection.projects.find(p => p.id === selection.selectedId);
    if (!project) return;
    const dialog = element("project-dialog") as HTMLDialogElement;
    const input = h("input", { id: "project-name-input", class: "text-input", value: project.name, maxlength: "120", required: true }) as HTMLInputElement;
    const error = h("div", { class: "form-error", role: "alert" });
    let saving = false;
    const cancel = button("Cancel", () => dialog.close(), "secondary");
    dialog.oncancel = event => { if (saving) event.preventDefault(); };
    const submit = button(removing ? "Remove entry" : "Save name", () => {}, removing ? "secondary" : "primary");
    submit.type = "submit";
    const form = h("form", { onSubmit: async (event: Event) => {
      event.preventDefault(); if (saving) return;
      saving = true; submit.disabled = true; cancel.disabled = true; input.disabled = true; error.replaceChildren();
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
        focusHeading(); notify(removing ? "Project entry removed. All project files are untouched." : "Project renamed.");
      } catch (failure) { error.replaceChildren(failureNotice(failure)); }
      finally { saving = false; submit.disabled = false; cancel.disabled = false; input.disabled = false; if (dialog.open) (removing ? cancel : input).focus(); }
    } },
      h("h2", { id: "project-dialog-title" }, removing ? "Remove " + project.name + "?" : "Rename project"),
      removing ? h("p", { class: "section-note" }, "Removes the library entry only. Project files stay intact.") : null,
      removing ? null : h("label", { class: "form-field", for: "project-name-input" }, "Project name", input),
      error, h("div", { class: "dialog-actions" }, cancel, submit));
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
    create: () => { if (loading || collectionBusy) return; newProjectDialog(element("project-dialog") as HTMLDialogElement, bridge, async next => { collection = next; if (next.selectedId) await selectProject(next.selectedId); notify("Project created"); }); },
    openManaged: () => { void changeCollection(() => bridge.openManagedProject()); },
    importDataset: () => {
      const id = selection.selectedId; if (!id || loading || collectionBusy || !workspace()?.managed) return;
      importDatasetDialog(element("project-dialog") as HTMLDialogElement, bridge, id, async result => {
        if (selection.selectedId !== id) return;
        opened = result; navigate({ page: "datasets" }); notify("Dataset imported");
      });
    },
    verify: () => {
      const id = selection.selectedId; if (!id || loading) return;
      loading = true; loadingMessage = "Verifying all model and dataset contents… Large projects can take a moment."; render();
      void selection.open(id, projectId => bridge.verifyManagedProject(projectId)).then(result => {
        if (!result) return;
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
  async function refreshOptimization(): Promise<void> {
    const id = selection.selectedId;
    if (!id || !workspace()?.managed || view.optimization.loading) return;
    const state = view.optimization; state.loading = true; state.error = undefined; state.errorTitle = undefined; render();
    try {
      state.readiness = await bridge.managedReadiness(id, state.manifest?.token);
      if (state.manifest) state.manifest = { ...state.manifest, readiness: state.readiness };
      else {
        state.prepared = state.readiness.preparedOptimization;
      }
      const existing = state.manifest?.readiness.launchPreview?.existingRun ?? state.prepared?.launchPreview.existingRun ?? state.readiness.launchPreview?.existingRun;
      if (existing) state.run = runStatus(await bridge.managedOptimize(id, { action: "status", runId: existing.runId }));
      else if (!state.prepared) state.run = undefined;
    } catch (error) { state.errorTitle = "Could not refresh launch readiness"; state.error = message(error); }
    finally { state.loading = false; if (selection.selectedId === id) render(); }
  }
  async function chooseOptimizationManifest(): Promise<void> {
    const id = selection.selectedId;
    if (!id || view.optimization.loading) return;
    const state = view.optimization; state.loading = true; state.error = undefined; state.errorTitle = undefined; render();
    try {
      const selected = await bridge.chooseOptimizationManifest(id);
      if (selected && selection.selectedId === id) { state.manifest = selected; state.prepared = undefined; state.readiness = selected.readiness; state.run = undefined; }
    } catch (error) { state.errorTitle = "Could not inspect the selected definition"; state.error = message(error); }
    finally { state.loading = false; if (selection.selectedId === id) render(); }
  }
  async function prepareOptimization(): Promise<void> {
    const id = selection.selectedId;
    if (!id || view.optimization.loading) return;
    const state = view.optimization; state.loading = true; state.executing = "prepare"; state.error = undefined; state.errorTitle = undefined; render();
    try {
      const prepared = await bridge.prepareOptimization(id);
      if (selection.selectedId === id) {
        state.prepared = prepared; state.manifest = undefined; state.run = undefined;
        if (state.readiness) state.readiness = { ...state.readiness, launchPreview: prepared.launchPreview, preparedOptimization: prepared };
      }
    } catch (error) { state.errorTitle = "Could not prepare the approved run"; state.error = message(error); }
    finally { state.loading = false; state.executing = undefined; if (selection.selectedId === id) render(); }
  }
  async function optimize(request: Parameters<typeof bridge.managedOptimize>[1]): Promise<void> {
    const id = selection.selectedId;
    if (!id || view.optimization.loading) return;
    const state = view.optimization;
    state.loading = true; state.executing = request.action; state.error = undefined; state.errorTitle = undefined;
    if (request.action !== "status") state.report = undefined;
    render();
    let polling = request.action === "resume";
    const poll = async (): Promise<void> => {
      if (request.action !== "resume") return;
      while (polling) {
        await new Promise(resolve => setTimeout(resolve, 1500));
        if (!polling || selection.selectedId !== id) return;
        try {
          const received = runStatus(await bridge.managedOptimize(id, { action: "status", runId: request.runId }));
          if (!polling || selection.selectedId !== id) return;
          state.run = received;
          if (selection.selectedId === id) render();
        } catch { /* The executing command owns the user-facing failure. */ }
      }
    };
    void poll();
    try {
      state.run = runStatus(await bridge.managedOptimize(id, request));
      if (request.action !== "status" && selection.selectedId === id) {
        try {
          const latest = await bridge.selectProject(id);
          if (selection.selectedId === id) opened = latest;
        } catch (error) {
          state.errorTitle = "The run changed, but its evidence could not be reloaded";
          state.error = message(error);
        }
      }
    }
    catch (error) {
      state.errorTitle = ({ start: "Could not reserve the optimization run", resume: "Could not execute this stage", "authorize-sealed": "Could not authorize final acceptance", cancel: "Could not cancel the run" } as Record<string, string>)[request.action] ?? "Could not update the optimization run";
      state.error = message(error);
    }
    finally { polling = false; state.loading = false; state.executing = undefined; if (selection.selectedId === id) render(); }
  }
  async function promoteAccepted(): Promise<void> {
    const id = selection.selectedId, run = view.optimization.run, managed = workspace()?.managed;
    const expectedBaselineRevisionId = managed?.modelCatalog?.activeBaselineRevisionId;
    if (!id || !run || !expectedBaselineRevisionId || view.optimization.loading) return;
    if (!window.confirm("Promote this sealed-accepted checkpoint to the project baseline? The current baseline remains in immutable history.")) return;
    const state = view.optimization; state.loading = true; state.executing = "promote"; state.error = undefined; state.errorTitle = undefined; render();
    try {
      const latest = await bridge.promoteAccepted(id, { runId: run.run_id, expectedBaselineRevisionId });
      if (selection.selectedId === id) {
        opened = latest;
        notify("Accepted checkpoint promoted. Reconnect the scientific runtime before the next run.");
      }
    } catch (error) {
      state.errorTitle = "Could not promote the accepted checkpoint";
      state.error = message(error);
    } finally {
      state.loading = false; state.executing = undefined; if (selection.selectedId === id) render();
    }
  }
  async function loadOptimizationReport(): Promise<void> {
    const id = selection.selectedId, runId = view.optimization.run?.run_id;
    if (!id || !runId || view.optimization.loading) return;
    const state = view.optimization; state.loading = true; state.executing = "report"; state.error = undefined; state.errorTitle = undefined; render();
    try {
      state.report = runReport(await bridge.managedOptimize(id, { action: "report", runId }));
    } catch (error) {
      state.errorTitle = "Could not load the complete run report";
      state.error = message(error);
    } finally {
      state.loading = false; state.executing = undefined; if (selection.selectedId === id) render();
    }
  }
  const optimizationActions: OptimizationPageActions = {
    refresh: () => { void refreshOptimization(); },
    prepare: () => { void prepareOptimization(); },
    chooseManifest: () => { void chooseOptimizationManifest(); },
    start: () => {
      const prepared = view.optimization.prepared;
      const existing = prepared?.launchPreview.existingRun ?? view.optimization.manifest?.readiness.launchPreview?.existingRun ?? view.optimization.readiness?.launchPreview?.existingRun;
      if (existing) void optimize({ action: "status", runId: existing.runId });
      else if (prepared) void optimize({ action: "start", manifestToken: prepared.token });
      else if (view.optimization.manifest) void optimize({ action: "start", manifestToken: view.optimization.manifest.token });
    },
    resume: () => { const id = view.optimization.run?.run_id; if (id) void optimize({ action: "resume", runId: id }); },
    authorizeSealed: () => {
      const id = view.optimization.run?.run_id;
      if (!id) return;
      if (!window.confirm("Authorize exactly one sealed evaluation for this selected candidate? Its aggregate result will decide final acceptance. Sealed rows remain hidden and this authorization cannot be reused.")) return;
      void optimize({ action: "authorize-sealed", runId: id });
    },
    loadReport: () => { void loadOptimizationReport(); },
    promote: () => { void promoteAccepted(); },
    cancel: () => {
      const id = view.optimization.run?.run_id;
      if (!id) return;
      const reason = window.prompt("Why are you cancelling this run? The reason is persisted in its audit history.");
      if (reason?.trim()) void optimize({ action: "cancel", runId: id, reason: reason.trim() });
    },
    openSettings: () => navigate({ page: "project" }), openData: () => navigate({ page: "datasets" }), openRuns: () => navigate({ page: "runs" }),
    upgrade: () => {
      const id = selection.selectedId; if (!id || view.optimization.loading) return;
      view.optimization.loading = true; view.optimization.error = undefined; render();
      void bridge.upgradeManagedProject(id).then(result => {
        if (selection.selectedId !== id) return;
        opened = result; view.optimization.loading = false; void refreshOptimization();
      }, error => { if (selection.selectedId === id) { view.optimization.loading = false; view.optimization.error = message(error); render(); } });
    },
  };
  async function refreshProviders(): Promise<void> {
    const id = selection.selectedId;
    if (!id || !workspace()?.managed || view.providers.loading) return;
    const state = view.providers; state.loading = true; state.error = undefined; render();
    try { state.status = await bridge.managedProviders(id); }
    catch (error) { state.error = message(error); }
    finally { state.loading = false; if (selection.selectedId === id) render(); }
  }
  const providerActions: ProviderPageActions = {
    refresh: () => { void refreshProviders(); },
    configure: () => {
      const id = selection.selectedId;
      if (!id || view.providers.loading) return;
      providerDialog(element("project-dialog") as HTMLDialogElement, view.providers.status, async submission => {
        view.providers.status = await bridge.configureManagedProviders(id, submission.settings);
        for (const role of ["generation", "advisor", "evaluator"] as const) {
          const secret = submission.credentials[role];
          if (secret) view.providers.status = await bridge.setProviderCredential(id, role, secret);
        }
        if (selection.selectedId === id) { notify("Providers saved"); render(); }
      });
    },
    remove: role => {
      const id = selection.selectedId; if (!id || view.providers.loading) return;
      view.providers.loading = true; view.providers.error = undefined; render();
      void bridge.removeProviderCredential(id, role).then(result => { if (selection.selectedId === id) { view.providers.status = result; view.providers.loading = false; render(); notify("Saved credential removed."); } }, error => { if (selection.selectedId === id) { view.providers.loading = false; view.providers.error = message(error); render(); } });
    },
  };
  const runtimeActions = {
    configure: () => {
      const id = selection.selectedId;
      if (!id || loading || !workspace()?.managed) return;
      scientificRuntimeDialog(element("project-dialog") as HTMLDialogElement, bridge, id, result => {
        if (selection.selectedId !== id) return;
        opened = result; view.optimization = { loading: false }; render();
        notify("Scientific runtime connected. No training or evaluation was run.");
      });
    },
    upgrade: () => {
      const id = selection.selectedId;
      if (!id || loading || !workspace()?.managed) return;
      loading = true; loadingMessage = "Initializing immutable model history from the verified baseline…"; render();
      void bridge.upgradeManagedProject(id).then(result => {
        if (selection.selectedId !== id) return;
        opened = result; loading = false; view.optimization = { loading: false }; render();
        notify("Model history initialized. The checkpoint and datasets are unchanged.");
      }, error => {
        if (selection.selectedId !== id) return;
        loading = false; operationError = error; render();
      });
    },
  };
  const actions: Actions = {
    navigate, render, help, notify, connect: projects.addFolder,
    backTo: page => { if (view.history.returnTo(page, main.scrollTop)) { render(); restorePlace(); } else navigate({ page }); },
    refresh: () => { if (selection.selectedId) void selectProject(selection.selectedId); else void refreshCollection(); },
    copy: value => { void bridge.copyText(value).then(() => notify("Copied to clipboard"), error => notify("Copy failed: " + message(error))); },
    compare: rows => {
      if (!rows.length || rows.length > 3 || rows.some(row => setupId(row.run) !== setupId(rows[0]!.run))) { notify("Select 1–3 candidates from the same evaluation setup."); return; }
      navigate({ page: "compare", candidateIds: rows.map(row => row.candidate.id) });
    },
    prepareOptimization: () => { navigate({ page: "optimization" }); if (!view.optimization.readiness) void refreshOptimization(); },
  };
  const pages: [Page, string, string][] = [["models", "Models", "models"], ["datasets", "Data", "dataset"], ["runs", "Runs", "runs"], ["benchmarks", "Evaluation", "benchmark"], ["project", "Project settings", "settings"]];
  function render(): void {
    const current = view.history.current, data = workspace();
    const project = collection.projects.find(p => p.id === selection.selectedId);
    const linkedOptimizationIds = new Set(data?.runs.flatMap(run => run.optimizationId ? [run.optimizationId] : []) ?? []);
    const runCount = (data?.runs.filter(run => !run.optimizationId).length ?? 0) + linkedOptimizationIds.size + (view.optimization.run && !linkedOptimizationIds.has(view.optimization.run.run_id) ? 1 : 0);
    const activePage = ["candidate", "baseline", "compare"].includes(current.page) ? "models" : ["run", "optimization"].includes(current.page) ? "runs" : current.page;
    const focus = document.activeElement, focusId = focus?.id;
    const caret = focus instanceof HTMLInputElement && ["text", "search"].includes(focus.type) ? [focus.selectionStart, focus.selectionEnd] : undefined;
    const scroll = main.scrollTop;
    element("project-nav").replaceChildren(...collection.projects.map(p => h("section", { class: "project-folder" + (p.id === project?.id ? " selected-project" : "") },
      h("button", { type: "button", id: "project-" + p.id, disabled: collectionBusy, class: "project-folder-button", title: p.source.kind === "folder" ? p.source.path : "Recorded example", "data-project-id": p.id, "aria-expanded": String(p.id === project?.id), onClick: () => {
        if (p.id === selection.selectedId) navigate({ page: "models" }); else projects.select(p.id);
      } }, icon("project"), h("span", {}, p.name), p.source.kind === "example" ? h("small", {}, "Example") : !p.source.workspaceId ? h("small", {}, "Legacy") : null),
      p.id === project?.id ? h("div", { class: "project-pages" }, ...pages.filter(([page]) => page !== "datasets" || (p.source.kind === "folder" && p.source.workspaceId)).map(([page, label, symbol]) => h("button", { type: "button", id: "nav-" + page, disabled: collectionBusy, class: "nav-item" + (page === activePage ? " active" : ""), "aria-current": page === activePage ? "page" : null, "data-page": page, onClick: () => navigate({ page }) }, icon(symbol), label,
        data && ["models", "runs"].includes(page) ? h("span", { class: "nav-count" }, page === "models" ? candidateRows(data).length + 1 : runCount) : null))) : null)));
    const candidate = data ? candidateRows(data).find(r => r.candidate.id === current.id)?.candidate : undefined;
    const run = data?.runs.find(r => r.id === current.id);
    const title = !project ? "Projects" : current.page === "candidate" ? candidate ? candidateName(candidate) : "Candidate not found" :
      current.page === "run" ? run ? runName(run) : "Run not found" :
      pages.find(p => p[0] === current.page)?.[1] ?? (current.page === "baseline" ? "Baseline" : current.page === "optimization" ? "Start optimization" : "Compare models");
    element("breadcrumb").textContent = project ? project.name + " / " + title : "Encoder Gym";
    document.title = title + " · Encoder Gym";
    element("source-state").replaceChildren(...(project ? [button(loading ? "Reading…" : opened?.content.state === "error" ? "Evidence unavailable" : data?.managed ? "Managed workspace" : data?.source === "recorded" ? "Recorded example" : data ? "Legacy journals" : "No records yet", () => navigate({ page: "project" }), "source-button"),
      ...(data ? [h("span", {}, dateLabel(data.capturedAt))] : [])] : []));
    const reload = element("reload-evidence") as HTMLButtonElement; reload.hidden = !project; reload.disabled = loading || collectionBusy;
    for (const id of ["add-project", "open-project", "open-legacy-project"]) (element(id) as HTMLButtonElement).disabled = loading || collectionBusy || !!collectionError;
    let content: HTMLElement;
    if (collectionError) content = h("div", { class: "page-content" }, h("h1", { tabindex: "-1" }, "Project library unavailable"), h("section", { class: "project-recovery", role: "alert" }, failureNotice(collectionError), button("Try again", () => { void refreshCollection(); }, "primary")));
    else if (!project) content = loading ? h("div", { class: "page-content", role: "status" }, "Opening project collection…") : renderWelcome(projects);
    else if (current.page === "project") content = data?.managed ? renderManagedSettings(project, data.managed, view.providers, providerActions, runtimeActions, actions, projects) : renderProjectSettings(project, opened, actions, projects);
    else if (!data) content = renderProjectState(project, opened, actions, projects, current.page);
    else {
      const detailKey = (current.id ?? "") + ":" + (current.runId ?? "");
      const exactCandidate = current.runId ? data.runs.find(r => r.id === current.runId)?.candidates.find(c => c.id === current.id) : candidateRows(data).find(r => r.candidate.id === current.id)?.candidate;
      const detail = view.details.get(detailKey) ?? { failedOnly: exactCandidate?.development.some(d => d.checks.some(g => !g.passed)) ?? false };
      if (current.id) view.details.set(detailKey, detail);
      if (current.page === "models") content = renderModels(data, view.models, actions);
      else if (current.page === "datasets" && data.managed) content = renderDatasets(data.managed, actions, projects, view.optimization.readiness);
      else if (current.page === "optimization" && data.managed) content = renderOptimization(data.managed, view.optimization, optimizationActions);
      else if (current.page === "candidate") content = renderCandidate(data, current.id ?? "", current.tab ?? "results", detail, actions, current.runId);
      else if (current.page === "run") content = renderRun(data, current.id ?? "", current.tab ?? "overview", actions);
      else if (current.page === "runs") content = renderRuns(data, actions, view.optimization.run);
      else if (current.page === "benchmarks") content = renderBenchmarks(data, actions, view.optimization.readiness);
      else if (current.page === "baseline") content = renderBaseline(data, actions);
      else content = renderCompare(data, candidateRows(data).filter(r => current.candidateIds?.includes(r.candidate.id)), actions);
    }
    if (operationError !== undefined) content.prepend(h("section", { id: "operation-error", role: "alert", class: "operation-failure" }, failureNotice(operationError), button("Dismiss", () => { operationError = undefined; render(); focusHeading(); }, "ghost small")));
    content.inert = loading || collectionBusy;
    main.setAttribute("aria-busy", String(loading || collectionBusy));
    main.replaceChildren(...(loading || collectionBusy ? [h("div", { class: "workspace-progress", role: "status" }, loadingMessage)] : []), content); main.scrollTop = scroll;
    if (focusId) { const target = document.getElementById(focusId); target?.focus({ preventScroll: true }); if (caret && target instanceof HTMLInputElement) target.setSelectionRange(caret[0] ?? null, caret[1] ?? null); }
    (element("navigate-back") as HTMLButtonElement).disabled = !project || !view.history.canNavigate(-1);
    (element("navigate-forward") as HTMLButtonElement).disabled = !project || !view.history.canNavigate(1);
  }
  for (const [id, offset] of [["navigate-back", -1], ["navigate-forward", 1]] as const) element(id).addEventListener("click", () => { const entry = view.history.move(offset, main.scrollTop); if (entry) { render(); restorePlace(); } });
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
