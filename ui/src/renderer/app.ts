import type { WorkspaceSnapshot } from "../workspace.js";
import type { ManagedOptimizationReport, ManagedOptimizationResult, ManagedRunStatus } from "../managed-control.js";
import { ProjectSelection, type OpenedProject, type ProjectCollection } from "../projects.js";
import type { Actions, Location, Page, ProjectActions } from "./actions.js";
import { candidateName, candidateRows, dateLabel, initialFilter, metricInfo, runName, setupId } from "./catalog.js";
import { button, disclosureIndicator, failureNotice, icon, spinner, tag } from "./components.js";
import { renderCompare, renderRun, type DetailState } from "./detail-pages.js";
import { h, replaceView } from "./dom.js";
import { renderModels, type ModelPageState } from "./models-page.js";
import { renderModel } from "./model-view.js";
import { findModel, modelInventory } from "./model-inventory.js";
import { NavigationHistory } from "./state.js";
import { inputRunById, inputRunName, renderBenchmarks, renderRuns } from "./workspace-pages.js";
import { renderProjectSettings, renderProjectState, renderWelcome } from "./project-pages.js";
import { previewBridge } from "./preview-bridge.js";
import { newProjectDialog } from "./onboarding.js";
import { renderManagedSettings, type ProviderPageActions, type ProviderPageState } from "./managed-pages.js";
import { DatasetController } from "./dataset-controller.js";
import { BenchmarkController } from "./benchmark-controller.js";
import { OptimizationSetupController } from "./optimization-setup-controller.js";
import { renderBenchmarkPage } from "./benchmark-page.js";
import { renderDatasetPage } from "./dataset-pages.js";
import { renderOptimization, type OptimizationPageActions, type OptimizationPageState } from "./optimization-page.js";
import { updateRunClocks } from "./run-progress.js";
import { providerDialog } from "./provider-dialog.js";
import { scientificRuntimeDialog } from "./scientific-runtime-dialog.js";
import { renderActivity, type ActivityPageActions, type ActivityPageState } from "./activity-page.js";
import { InputRunsController } from "./input-runs-controller.js";
import { renderOverview, newOverviewState, type OverviewState } from "./overview-page.js";

const element = (id: string): HTMLElement => { const found = document.getElementById(id); if (!found) throw new Error("Missing #" + id); return found; };
interface ProjectView { history: NavigationHistory; overview: OverviewState; models: ModelPageState; details: Map<string, DetailState>; optimization: OptimizationPageState; providers: ProviderPageState; activity: ActivityPageState; baselineBusy: boolean; datasets?: DatasetController; benchmarks?: BenchmarkController; setup?: OptimizationSetupController; inputRuns?: InputRunsController }
const newView = (): ProjectView => ({ history: new NavigationHistory(), overview: newOverviewState(), models: { filter: initialFilter(), selected: new Set(), suiteIndex: 0 }, details: new Map(), optimization: { loading: false }, providers: { loading: false }, activity: { loading: false }, baselineBusy: false });

export function mount(): void {
  const bridge = window.encoderGym ?? previewBridge();
  if (!window.encoderGym) document.documentElement.classList.add("browser-preview");
  const main = element("page"), shell = element("app-shell"), selection = new ProjectSelection();
  let collection: ProjectCollection = { version: 1, selectedId: null, projects: [] };
  let opened: OpenedProject | undefined, loading = true, collectionError: string | undefined;
  let collectionBusy = false, operationError: unknown, loadingMessage = "Opening project…";
  const views = new Map<string, ProjectView>();
  const collapsedProjects = new Set<string>();
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
    if (workspace()?.managed) {
      if (location.page === "runs" || location.page === "run") {
        if (location.id) view.overview.expanded = location.id;
        location = { page: "overview" };
      } else if (location.page === "optimization" && location.tab === "setup") {
        if (!view.setup?.running && !view.inputRuns?.runningId) {
          if (view.setup?.run) view.inputRuns?.retain(view.setup.run);
          view.setup?.newDraft(); view.overview.draft = true; view.overview.expanded = "draft"; view.overview.tabs.set("draft", "setup");
        }
        location = { page: "overview" };
      }
    }
    element("toast").hidden = true;
    shell.classList.remove("mobile-sidebar-open"); updateSidebar();
    if (location.page === "models" && location.id) view.models.filter.setup = location.id;
    view.history.remember(location, main.scrollTop, contentFocus());
    render(); main.scrollTop = 0; focusHeading();
    if (location.page === "project" && workspace()?.managed && !view.providers.status) void refreshProviders();
    if (location.page === "activity" && workspace()?.managed) void refreshActivity();
    if ((["runs", "overview"].includes(location.page) || location.page === "benchmarks" && !view.optimization.readiness) && workspace()?.managed && !view.optimization.loading) void refreshOptimization();
  }
  async function selectProject(id: string): Promise<void> {
    if (collectionBusy) return;
    if (selection.selectedId && collection.projects.some(project => project.id === selection.selectedId)) { view.history.capture(main.scrollTop, contentFocus()); views.set(selection.selectedId, view); }
    const changed = selection.selectedId !== id;
    const firstVisit = !views.has(id);
    view = views.get(id) ?? newView(); views.set(id, view);
    if (changed) { opened = undefined; operationError = undefined; element("toast").hidden = true; main.scrollTop = 0; }
    collection.selectedId = id; loading = true; loadingMessage = "Reading project files…";
    const pending = selection.open(id, projectId => bridge.selectProject(projectId));
    render();
    try {
      const next = await pending;
      if (!next) return;
      opened = next;
      if (firstVisit && next.content.state === "ready" && next.content.workspace.managed) view.history.remember({ page: "overview" }, 0);
      loading = false; render();
      if (view.history.current.page === "project" && next.content.state === "ready" && next.content.workspace.managed && !view.providers.status) void refreshProviders();
      if (view.history.current.page === "activity" && next.content.state === "ready" && next.content.workspace.managed) void refreshActivity();
      if (["runs", "overview", "benchmarks"].includes(view.history.current.page) && next.content.state === "ready" && next.content.workspace.managed && !view.optimization.readiness && !view.optimization.loading) void refreshOptimization();
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
    if (!id || !workspace()?.managed) return;
    if (view.optimization.executing === "resume") { await pollOptimization(id, view.optimization); return; }
    if (view.optimization.loading) return;
    const state = view.optimization; state.loading = true; state.error = undefined; state.errorTitle = undefined; render();
    state.revision = (state.revision ?? 0) + 1;
    try {
      state.readiness = await bridge.managedReadiness(id, state.manifest?.token);
      if (state.manifest) state.manifest = { ...state.manifest, readiness: state.readiness };
      else {
        state.prepared = state.readiness.preparedOptimization;
      }
      const existing = state.manifest?.readiness.launchPreview?.existingRun ?? state.prepared?.launchPreview.existingRun ?? state.readiness.launchPreview?.existingRun;
      if (existing) {
        state.run = runStatus(await bridge.managedOptimize(id, { action: "status", runId: existing.runId }));
        state.statusCheckedAt = Date.now(); state.statusError = undefined;
      }
      else if (!state.prepared) state.run = undefined;
    } catch (error) { state.errorTitle = "Could not refresh launch readiness"; state.error = message(error); }
    finally { state.revision = (state.revision ?? 0) + 1; state.loading = false; if (selection.selectedId === id) render(); }
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
    state.revision = (state.revision ?? 0) + 1;
    state.loading = true; state.executing = request.action; state.operationStartedAt = Date.now(); state.error = undefined; state.errorTitle = undefined;
    if (request.action !== "status") state.report = undefined;
    render();
    try {
      state.run = runStatus(await bridge.managedOptimize(id, request));
      state.revision = (state.revision ?? 0) + 1;
      state.statusCheckedAt = Date.now(); state.statusError = undefined;
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
    finally {
      state.revision = (state.revision ?? 0) + 1;
      state.loading = false; state.executing = undefined; state.operationStartedAt = undefined;
      if (selection.selectedId === id) render();
    }
  }
  async function pollOptimization(id: string, state: OptimizationPageState): Promise<void> {
    const run = state.run;
    if (!run || state.statusPending) return;
    const revision = state.revision;
    state.statusPending = true;
    try {
      const result = await bridge.managedOptimize(id, { action: "status", runId: run.run_id });
      if (state.revision !== revision || state.run?.run_id !== run.run_id) return;
      state.run = runStatus(result); state.statusCheckedAt = Date.now(); state.statusError = undefined;
    } catch (error) {
      if (state.revision === revision) state.statusError = message(error);
    } finally {
      state.statusPending = false;
      if (selection.selectedId === id && view.optimization === state) render();
    }
  }
  // Observation survives navigation and also discovers externally started workers.
  window.setInterval(() => {
    const id = selection.selectedId, state = view.optimization;
    if (!id || !state.run || state.loading && state.executing !== "resume" || !["optimization", "runs"].includes(view.history.current.page)
      || view.history.current.page === "optimization" && view.history.current.tab === "setup"
      || ["completed", "failed", "cancelled"].includes(state.run.state)) return;
    void pollOptimization(id, state);
  }, 2000);
  window.setInterval(() => updateRunClocks(main), 1000);
  async function promoteAccepted(runId = view.optimization.run?.run_id, modelName = "accepted candidate"): Promise<void> {
    const id = selection.selectedId, managed = workspace()?.managed;
    const expectedBaselineRevisionId = managed?.modelCatalog?.activeBaselineRevisionId;
    if (!id || !runId || !expectedBaselineRevisionId || view.baselineBusy) return;
    if (!window.confirm(`Make ${modelName} the baseline?`)) return;
    const state = view.optimization, fromRun = ["run", "optimization"].includes(view.history.current.page) && state.run?.run_id === runId;
    view.baselineBusy = true;
    if (fromRun) { state.loading = true; state.executing = "promote"; state.error = undefined; state.errorTitle = undefined; }
    render();
    try {
      const latest = await bridge.promoteAccepted(id, { runId, expectedBaselineRevisionId });
      if (selection.selectedId === id) {
        opened = latest;
        notify("Baseline updated. Reconnect the runtime before optimizing.");
      }
    } catch (error) {
      if (fromRun) { state.errorTitle = "Could not update the baseline"; state.error = message(error); }
      else operationError = error;
    } finally {
      view.baselineBusy = false;
      if (fromRun) { state.loading = false; state.executing = undefined; }
      if (selection.selectedId === id) render();
    }
  }
  async function restoreBaseline(targetRevisionId: string, modelName: string): Promise<void> {
    const id = selection.selectedId, expectedBaselineRevisionId = workspace()?.managed?.modelCatalog?.activeBaselineRevisionId;
    if (!id || !expectedBaselineRevisionId || view.baselineBusy) return;
    if (!window.confirm(`Restore ${modelName} as the baseline?`)) return;
    view.baselineBusy = true; operationError = undefined; render();
    try {
      const latest = await bridge.restoreBaseline(id, { targetRevisionId, expectedBaselineRevisionId });
      if (selection.selectedId === id) { opened = latest; notify("Baseline restored. Reconnect the runtime before optimizing."); }
    } catch (error) { operationError = error; }
    finally { view.baselineBusy = false; if (selection.selectedId === id) render(); }
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
  async function refreshActivity(): Promise<void> {
    const id = selection.selectedId;
    if (!id || !workspace()?.managed || view.activity.loading) return;
    const state = view.activity;
    state.loading = true; state.error = undefined; render();
    try { state.log = await bridge.projectActivity(id, 30); }
    catch (error) { state.error = message(error); }
    finally { state.loading = false; if (selection.selectedId === id) render(); }
  }
  async function exportActivity(): Promise<void> {
    const id = selection.selectedId;
    if (!id || !workspace()?.managed || view.activity.exporting) return;
    const state = view.activity;
    state.exporting = true; state.error = undefined; render();
    try {
      const result = await bridge.exportProjectActivity(id);
      if (result) notify(`Exported ${result.events} events.`);
    } catch (error) { state.error = message(error); }
    finally { state.exporting = false; if (selection.selectedId === id) render(); }
  }
  const activityActions: ActivityPageActions = {
    refresh: () => { void refreshActivity(); },
    export: () => { void exportActivity(); },
    copy: value => { void bridge.copyText(value).then(() => notify("Copied to clipboard"), error => notify("Copy failed: " + message(error))); },
  };
  const actions: Actions = {
    navigate, render, help, notify, connect: projects.addFolder,
    backTo: page => { if (view.history.returnTo(page, main.scrollTop)) { render(); restorePlace(); } else navigate({ page }); },
    refresh: () => { view.datasets?.refresh(); view.benchmarks?.refresh(); view.setup?.refresh(); view.inputRuns?.refresh(); if (selection.selectedId) void selectProject(selection.selectedId); else void refreshCollection(); },
    copy: value => { void bridge.copyText(value).then(() => notify("Copied to clipboard"), error => notify("Copy failed: " + message(error))); },
    compare: rows => {
      if (!rows.length || rows.length > 3 || rows.some(row => setupId(row.run) !== setupId(rows[0]!.run))) { notify("Select 1–3 candidates from the same evaluation setup."); return; }
      navigate({ page: "compare", candidateIds: rows.map(row => row.candidate.id) });
    },
    prepareOptimization: () => { navigate({ page: "optimization", tab: "setup" }); },
    get baselineBusy() { return view.baselineBusy; },
    promoteModel: (runId, name) => { void promoteAccepted(runId, name); },
    restoreBaseline: (targetRevisionId, name) => { void restoreBaseline(targetRevisionId, name); },
  };
  const pages: [Page, string, string][] = [["overview", "Overview", "optimize"], ["models", "Models", "models"], ["datasets", "Data", "dataset"], ["runs", "Runs", "runs"], ["benchmarks", "Evaluation", "benchmark"], ["activity", "Activity", "activity"], ["project", "Project settings", "settings"]];
  function render(): void {
    const current = view.history.current, data = workspace();
    const project = collection.projects.find(p => p.id === selection.selectedId);
    if (project && data?.managed) {
      const id = project.id;
      const owner = view;
      view.datasets ??= new DatasetController(id, data.managed, bridge, {
        render: () => { if (selection.selectedId === id) render(); },
        navigate: location => { if (selection.selectedId === id) navigate(location); },
        imported: managed => { if (selection.selectedId === id && opened?.content.state === "ready") opened = { ...opened, content: { state: "ready", workspace: { ...opened.content.workspace, managed } } }; },
        dialog: () => element("project-dialog") as HTMLDialogElement, copy: actions.copy,
      });
      view.datasets.sync(data.managed);
      view.benchmarks ??= new BenchmarkController(id, data.managed, bridge, {
        render: () => { if (selection.selectedId === id) render(); },
        navigate: location => { if (selection.selectedId === id) navigate(location); },
        dialog: () => element("project-dialog") as HTMLDialogElement,
      });
      view.benchmarks.sync(data.managed);
      view.inputRuns ??= new InputRunsController(id, bridge, () => { if (selection.selectedId === id) render(); }, (managed, snapshot) => {
        if (selection.selectedId === id && opened?.content.state === "ready") opened = { ...opened, content: { state: "ready", workspace: snapshot ?? { ...opened.content.workspace, managed } } };
      });
      view.setup ??= new OptimizationSetupController(id, data.managed, bridge, () => { if (selection.selectedId === id) render(); }, (managed, snapshot) => {
        if (selection.selectedId === id && opened?.content.state === "ready") opened = { ...opened, content: { state: "ready", workspace: snapshot ?? { ...opened.content.workspace, managed } } };
        owner.inputRuns?.refresh();
      });
      view.setup.sync(data.managed);
    }
    const projectRunIds = new Set(view.inputRuns?.runs?.map(run => run.id) ?? []);
    const linkedOptimizationIds = new Set(data?.runs.flatMap(run => run.optimizationId ? [run.optimizationId] : []) ?? []);
    const runCount = projectRunIds.size + (data?.runs.filter(run => !run.optimizationId || !projectRunIds.has(run.optimizationId)).length ?? 0)
      + (view.optimization.run && !linkedOptimizationIds.has(view.optimization.run.run_id) && !projectRunIds.has(view.optimization.run.run_id) ? 1 : 0);
    const activePage = data?.managed && ["overview", "runs", "run", "optimization"].includes(current.page) ? "overview" : ["model", "candidate", "baseline", "compare"].includes(current.page) ? "models" : current.page === "run" ? "runs" : current.page === "dataset" ? "datasets" : current.page;
    const projectRunActive = !!view.setup?.preparationId || !!view.setup?.running || !!view.setup?.saving || !!view.setup?.initializingEvaluation || !!view.inputRuns?.runningId;
    const focus = document.activeElement, focusId = focus?.id;
    const caret = focus instanceof HTMLInputElement && ["text", "search"].includes(focus.type) ? [focus.selectionStart, focus.selectionEnd] : undefined;
    const scroll = main.scrollTop;
    const disclosurePage = `${project?.id}:${current.page}:${current.id ?? ""}:${view.optimization.run?.run_id ?? ""}`;
    const openDisclosures = main.dataset.disclosurePage === disclosurePage
      ? new Set([...main.querySelectorAll<HTMLDetailsElement>("details[open]")].map(node => node.querySelector("summary")?.textContent)) : new Set();
    const projectNav = element("project-nav");
    const nextProjectNav = document.createDocumentFragment();
    nextProjectNav.append(...collection.projects.map(p => {
      const expanded = p.id === project?.id && !collapsedProjects.has(p.id);
      const projectView = views.get(p.id), backgroundActive = !!projectView?.setup?.preparationId || !!projectView?.setup?.running || !!projectView?.setup?.saving || !!projectView?.setup?.initializingEvaluation || !!projectView?.inputRuns?.runningId;
      return h("section", { class: "project-folder" + (p.id === project?.id ? " selected-project" : "") + (expanded ? " expanded-project" : "") },
      h("button", { type: "button", id: "project-" + p.id, disabled: collectionBusy, class: "project-folder-button", title: p.source.kind === "folder" ? p.source.path : "Recorded example", "data-project-id": p.id, "aria-expanded": String(expanded), onClick: () => {
        if (p.id === selection.selectedId) { collapsedProjects.has(p.id) ? collapsedProjects.delete(p.id) : collapsedProjects.add(p.id); render(); }
        else { collapsedProjects.delete(p.id); projects.select(p.id); }
      } }, disclosureIndicator(expanded), icon("project"), h("span", {}, p.name), !expanded && backgroundActive ? h("span", { role: "status", "aria-label": "Optimization running" }, spinner(`project-folder:${p.id}`)) : null, p.source.kind === "example" ? h("small", {}, "Example") : !p.source.workspaceId ? h("small", {}, "Legacy") : null),
      expanded ? h("div", { class: "project-pages" }, ...pages.filter(([page]) => page === "runs" ? !(p.source.kind === "folder" && p.source.workspaceId) : !["datasets", "activity", "overview"].includes(page) || (p.source.kind === "folder" && p.source.workspaceId)).map(([page, label, symbol]) => h("button", { type: "button", id: "nav-" + page, disabled: collectionBusy, class: "nav-item" + (page === activePage ? " active" : ""), "aria-current": page === activePage ? "page" : null, "data-page": page, onClick: () => navigate({ page }) }, icon(symbol), label,
        page === "overview" && projectRunActive ? h("span", { class: "nav-meta", role: "status", "aria-label": "Optimization running" }, spinner(`overview-nav:${p.id}`)) : data && ["models", "runs"].includes(page) ? h("span", { class: "nav-meta" }, h("span", { class: "nav-count" }, page === "models" ? modelInventory(data).length : runCount)) : null))) : null);
    }));
    replaceView(projectNav, nextProjectNav);
    const candidate = data ? candidateRows(data).find(r => r.candidate.id === current.id)?.candidate : undefined;
    const run = data?.runs.find(r => r.id === current.id);
    const projectRun = inputRunById(view.inputRuns, view.setup, current.id);
    const title = !project ? "Projects" : current.page === "model" ? (data ? findModel(data, current.id ?? "")?.name : undefined) ?? "Model" : current.page === "candidate" ? candidate ? candidateName(candidate) : "Candidate not found" :
      current.page === "run" ? projectRun && data ? inputRunName(data, projectRun, view.inputRuns, view.setup) : run ? runName(run) : "Run not found" : current.page === "dataset" ? view.datasets?.find(current.id)?.entry.dataset.name ?? "Dataset" :
      pages.find(p => p[0] === current.page)?.[1] ?? (current.page === "baseline" ? "Baseline" : current.page === "optimization" ? current.tab === "setup" ? "Optimize" : view.optimization.run ? "Run" : "New run" : "Compare models");
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
    else if (current.page === "activity" && data?.managed) content = renderActivity(view.activity, activityActions);
    else if (!data) content = renderProjectState(project, opened, actions, projects, current.page);
    else {
      const detailKey = (current.id ?? "") + ":" + (current.runId ?? "");
      const exactCandidate = current.runId ? data.runs.find(r => r.id === current.runId)?.candidates.find(c => c.id === current.id) : candidateRows(data).find(r => r.candidate.id === current.id)?.candidate;
      const detail = view.details.get(detailKey) ?? { failedOnly: exactCandidate?.development.some(d => d.checks.some(g => !g.passed)) ?? false };
      if (current.id) view.details.set(detailKey, detail);
      if ((["overview", "runs", "run"].includes(current.page) || current.page === "optimization" && current.tab === "setup") && data.managed && view.setup && view.inputRuns) content = renderOverview(data, view.overview, view.setup, view.inputRuns, actions, view.optimization.run);
      else if (current.page === "models") content = renderModels(data, view.models, actions);
      else if (["datasets", "dataset"].includes(current.page) && data.managed && view.datasets) content = renderDatasetPage(data, current, view.datasets, actions);
      else if (current.page === "optimization" && data.managed) content = renderOptimization(data.managed, view.optimization, optimizationActions);
      else if (current.page === "model" || current.page === "candidate") content = renderModel(data, current.id ?? "", current.tab ?? "overview", detail, actions, current.runId);
      else if (current.page === "run") content = renderRun(data, current.id ?? "", current.tab ?? "overview", actions);
      else if (current.page === "runs") content = renderRuns(data, actions, view.optimization.run, data.managed ? view.inputRuns : undefined, data.managed ? view.setup : undefined);
      else if (current.page === "benchmarks") content = data.managed && view.benchmarks ? renderBenchmarkPage(data, current, view.benchmarks, actions) : renderBenchmarks(data, actions);
      else if (current.page === "baseline") content = renderModel(data, data.baseline.id, current.tab ?? "overview", detail, actions);
      else content = renderCompare(data, candidateRows(data).filter(r => current.candidateIds?.includes(r.candidate.id)), actions);
    }
    if (operationError !== undefined) content.prepend(h("section", { id: "operation-error", role: "alert", class: "operation-failure" }, failureNotice(operationError), button("Dismiss", () => { operationError = undefined; render(); focusHeading(); }, "ghost small")));
    content.inert = loading || collectionBusy;
    main.setAttribute("aria-busy", String(loading || collectionBusy));
    const nextMain = document.createDocumentFragment();
    if (loading || collectionBusy) nextMain.append(h("div", { class: "workspace-progress", role: "status" }, loadingMessage));
    nextMain.append(content);
    replaceView(main, nextMain); main.scrollTop = scroll;
    main.dataset.disclosurePage = disclosurePage;
    for (const disclosure of main.querySelectorAll<HTMLDetailsElement>("details")) {
      if (openDisclosures.has(disclosure.querySelector("summary")?.textContent)) disclosure.open = true;
    }
    updateRunClocks(main);
    if (focusId) { const target = document.getElementById(focusId); target?.focus({ preventScroll: true }); if (caret && target instanceof HTMLInputElement) target.setSelectionRange(caret[0] ?? null, caret[1] ?? null); }
    (element("navigate-back") as HTMLButtonElement).disabled = !project || !view.history.canNavigate(-1);
    (element("navigate-forward") as HTMLButtonElement).disabled = !project || !view.history.canNavigate(1);
    if (data?.managed && view.datasets && ["datasets", "dataset"].includes(current.page) && !loading && !collectionBusy) {
      const controller = view.datasets;
      queueMicrotask(() => { if (view.datasets === controller && !loading && !collectionBusy && workspace()?.managed) void controller.ensure(current); });
    }
    if (data?.managed && view.benchmarks && current.page === "benchmarks" && !loading && !collectionBusy) {
      const controller = view.benchmarks;
      queueMicrotask(() => { if (view.benchmarks === controller && !loading && !collectionBusy && workspace()?.managed) void controller.ensure(current); });
    }
    if (data?.managed && view.setup && ((current.page === "optimization" && current.tab === "setup") || current.page === "overview" || current.page === "runs" || current.page === "run") && !loading && !collectionBusy) {
      const controller = view.setup;
      queueMicrotask(() => { if (view.setup === controller && !loading && !collectionBusy && workspace()?.managed) void controller.ensure(); });
    }
    if (data?.managed && view.inputRuns && ["overview", "runs", "run"].includes(current.page) && !loading && !collectionBusy) {
      const controller = view.inputRuns;
      queueMicrotask(() => { if (view.inputRuns === controller && !loading && !collectionBusy && workspace()?.managed) void controller.ensure(); });
    }
  }
  for (const [id, offset] of [["navigate-back", -1], ["navigate-forward", 1]] as const) element(id).addEventListener("click", () => { const entry = view.history.move(offset, main.scrollTop); if (entry) { render(); restorePlace(); } });
  let lastMouseNavigation: { direction: "back" | "forward"; at: number } | undefined;
  const mouseNavigate = (direction: "back" | "forward") => {
    const now = performance.now();
    if (lastMouseNavigation?.direction === direction && now - lastMouseNavigation.at < 150) return;
    lastMouseNavigation = { direction, at: now };
    element(direction === "back" ? "navigate-back" : "navigate-forward").click();
  };
  bridge.onNavigationCommand(mouseNavigate);
  const extendedMouseButton = (event: MouseEvent | PointerEvent) => {
    if (document.querySelector("dialog[open]") || (event.button !== 3 && event.button !== 4)) return;
    event.preventDefault();
    mouseNavigate(event.button === 3 ? "back" : "forward");
  };
  document.addEventListener("pointerup", extendedMouseButton, true);
  document.addEventListener("auxclick", extendedMouseButton, true);
  element("reload-evidence").addEventListener("click", actions.refresh);
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
