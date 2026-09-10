import { app, BrowserWindow, clipboard, dialog, ipcMain, safeStorage } from "electron";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { readProjectContent } from "./evidence/read-workspace.js";
import { example, readExample } from "./evidence/examples.js";
import { ProjectRegistry } from "./project-registry.js";
import type { OpenedProject } from "./projects.js";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { runSmokeChecks } from "./smoke-checks.js";
import { runManagedSmokeChecks } from "./managed-smoke-checks.js";
import { checkCurrentManaged } from "./current-managed-check.js";
import { ManagedBackend, datasetPurpose, managedSnapshot } from "./managed-backend.js";
import { CredentialStore } from "./credential-store.js";
import type { ManagedProviderStatus, ManagedReadiness, ProviderRole } from "./managed-control.js";
import type { NativeProgress } from "./managed-control.js";
import type { ProjectActivityReference } from "./project-activity.js";

const directory = dirname(fileURLToPath(import.meta.url));
const smokeTest = process.argv.includes("--smoke-test");
const verifyCurrent = process.argv.includes("--verify-managed-current");
const actualRegistryFile = join(app.getPath("userData"), "projects.json");
if (smokeTest) app.setPath("userData", process.env.ENCODER_GYM_SMOKE_PROFILE ?? mkdtempSync(join(tmpdir(), "encoder-gym-renderer-")));
if (verifyCurrent) app.setPath("userData", mkdtempSync(join(tmpdir(), "encoder-gym-current-check-")));
const registry = new ProjectRegistry(verifyCurrent ? actualRegistryFile : join(app.getPath("userData"), "projects.json"));
const credentials = new CredentialStore(join(app.getPath("userData"), "credentials.json"), {
  available: () => safeStorage.isEncryptionAvailable(),
  encrypt: value => safeStorage.encryptString(value),
  decrypt: value => safeStorage.decryptString(value),
});
const backend = new ManagedBackend(
  app.isPackaged ? join(process.resourcesPath, "synth" + (process.platform === "win32" ? ".exe" : "")) : join(directory, "..", "..", "target", "debug", "synth" + (process.platform === "win32" ? ".exe" : "")),
  registry,
  undefined,
  { resolveCredential: (id, environmentFallback) => credentials.resolve(id, environmentFallback) },
);
let smokeFolderChoice: string | undefined;
async function pickFolder(title: string, defaultPath?: string): Promise<string | undefined> {
  if (smokeTest) { const choice = smokeFolderChoice; smokeFolderChoice = undefined; return choice; }
  const selected = await dialog.showOpenDialog({ title, defaultPath, properties: ["openDirectory", "createDirectory"] });
  return selected.canceled ? undefined : selected.filePaths[0];
}
async function pickExecutable(title: string): Promise<string | undefined> {
  if (smokeTest) { const choice = smokeFolderChoice; smokeFolderChoice = undefined; return choice; }
  const selected = await dialog.showOpenDialog({
    title,
    properties: ["openFile"],
    filters: process.platform === "win32" ? [{ name: "Python executable", extensions: ["exe"] }] : undefined,
  });
  return selected.canceled ? undefined : selected.filePaths[0];
}
const projectId = (value: unknown): string => { if (typeof value !== "string" || !value) throw new Error("A project identity is required."); return value; };
const providerRole = (value: unknown): ProviderRole => {
  if (value !== "generation" && value !== "advisor" && value !== "evaluator") throw new Error("Choose a provider authority.");
  return value;
};
function desktopProviderStatus(status: ManagedProviderStatus): ManagedProviderStatus {
  return {
    ...status,
    credentialAvailability: status.catalog?.providers.map(provider => {
      if (provider.authentication === "none") return { role: provider.role, authentication: provider.authentication, availability: "available", source: "not_required" };
      const reference = provider.secret;
      if (!reference || reference.id !== `${status.projectId}:${provider.role}`) return { role: provider.role, authentication: provider.authentication, availability: "unavailable" };
      return { role: provider.role, authentication: provider.authentication, ...credentials.status(reference.id, reference.environmentFallback) };
    }) ?? [],
  };
}
function withDesktopCredentialAvailability(readiness: ManagedReadiness, providers: ManagedProviderStatus): ManagedReadiness {
  const checks = readiness.report.checks.map(check => {
    const role = check.key === "providers.generation" ? "generation" : check.key === "providers.advisor" ? "advisor" : check.key === "providers.evaluator" ? "evaluator" : undefined;
    if (!role) return check;
    const credential = providers.credentialAvailability.find(item => item.role === role);
    if (!credential) return check;
    const available = credential.availability === "available";
    const source = credential.source === "environment" ? "the configured environment fallback" : credential.source === "credential_store" ? "operating-system encrypted desktop storage" : credential.source === "not_required" ? "no credential is required" : "no usable credential source";
    return {
      ...check,
      state: available ? "ready" as const : credential.availability === "missing" ? "action_required" as const : "unavailable" as const,
      summary: available ? "Provider credential is available" : credential.availability === "missing" ? "Provider credential is missing" : "Provider credential availability cannot be verified",
      evidence: `The ${role} authority uses ${source}. No secret value crossed the IPC boundary or entered this report.`,
      ...(available ? { nextAction: undefined } : { nextAction: { key: "configure-provider-secret", label: "Configure provider credential" } }),
    };
  });
  return { ...readiness, report: { ...readiness.report, checks } };
}
async function desktopReadiness(id: string, manifestToken?: string): Promise<ManagedReadiness> {
  const readiness = await backend.readiness(id, manifestToken);
  return withDesktopCredentialAvailability(readiness, desktopProviderStatus(await backend.providerStatus(id)));
}
const activityReference = (kind: string, id: unknown): ProjectActivityReference[] =>
  typeof id === "string" && id.trim() ? [{ kind, id: id.trim().slice(0, 200) }] : [];

async function trackProjectAction<T>(
  id: string,
  operation: string,
  references: ProjectActivityReference[],
  work: (progress: (value: NativeProgress) => void) => Promise<T> | T,
  resultReferences: (result: T) => ProjectActivityReference[] = () => [],
): Promise<T> {
  const actionId = await backend.startProjectActivity(id, operation, references);
  let pendingProgress: Promise<unknown> = Promise.resolve();
  let progressMarker = "";
  const progress = (value: NativeProgress): void => {
    const bucket = value.completed !== undefined && value.total !== undefined
      ? Math.floor((value.completed / value.total) * 10)
      : undefined;
    const marker = `${value.phase}:${bucket ?? "stage"}`;
    if (marker === progressMarker) return;
    progressMarker = marker;
    pendingProgress = pendingProgress
      .then(() => backend.progressProjectActivity(id, actionId, operation, value.phase, value.completed, value.total))
      .catch(() => undefined);
  };
  let result: T;
  try {
    result = await work(progress);
  } catch (error) {
    await pendingProgress;
    try { await backend.failProjectActivity(id, actionId, operation, error); } catch { /* Preserve the operation's actual failure. */ }
    throw error;
  }
  await pendingProgress;
  await backend.succeedProjectActivity(id, actionId, operation, resultReferences(result));
  return result;
}

async function recordCompletedProjectAction(id: string, operation: string, references: ProjectActivityReference[] = []): Promise<void> {
  const actionId = await backend.startProjectActivity(id, operation);
  await backend.succeedProjectActivity(id, actionId, operation, references);
}
ipcMain.handle("encoder-gym:get-projects", () => registry.read());
ipcMain.handle("encoder-gym:add-project-folder", async () => {
  const selected = await pickFolder("Add an encoder project folder");
  return selected ? registry.addFolder(selected) : null;
});
ipcMain.handle("encoder-gym:open-example", () => registry.addExample(example.key, example.name));
ipcMain.handle("encoder-gym:select-project", async (_event, value: unknown): Promise<OpenedProject> => {
  const id = projectId(value);
  if (!verifyCurrent) registry.select(id);
  const project = registry.get(id);
  if (project.source.kind === "folder") {
    if (project.source.workspaceId) return { project, content: { state: "ready", workspace: managedSnapshot(await backend.openRegistered(id)) } };
    return { project, content: readProjectContent(project.source.path) };
  }
  try { return { project, content: { state: "ready", workspace: readExample(project.source.key) } }; }
  catch (error) { return { project, content: { state: "error", message: error instanceof Error ? error.message : "Example unavailable." } }; }
});
ipcMain.handle("encoder-gym:rename-project", async (_event, id: unknown, name: unknown) => {
  if (typeof name !== "string") throw new Error("A project name is required.");
  const selected = projectId(id), project = registry.get(selected);
  if (project.source.kind !== "folder" || !project.source.workspaceId) return registry.rename(selected, name);
  return trackProjectAction(selected, "project.rename", activityReference("previous_name", project.name), () => registry.rename(selected, name), () => activityReference("project_name", name));
});
ipcMain.handle("encoder-gym:relocate-project", async (_event, value: unknown) => {
  const id = projectId(value), project = registry.get(id);
  if (project.source.kind !== "folder") throw new Error("Recorded examples do not have a connected folder.");
  const selected = await pickFolder(`Reconnect ${project.name}`, project.source.path);
  const workspaceId = project.source.workspaceId;
  if (selected && workspaceId) {
    const workspace = await backend.open(selected, true);
    if (workspace.manifest.id !== workspaceId) throw new Error("Choose the same Gym project at its new location. This folder has a different project identity.");
    const result = registry.addManaged(workspace);
    await recordCompletedProjectAction(id, "project.relocate");
    return result;
  }
  return selected ? registry.relocate(id, selected) : null;
});
ipcMain.handle("encoder-gym:forget-project", async (_event, value: unknown) => {
  const id = projectId(value), project = registry.get(id);
  if (project.source.kind !== "folder" || !project.source.workspaceId) return registry.remove(id);
  const actionId = await backend.startProjectActivity(id, "project.forget");
  await backend.succeedProjectActivity(id, actionId, "project.forget");
  return registry.remove(id);
});

ipcMain.handle("encoder-gym:open-managed", async () => {
  const path = await pickFolder("Open an Encoder Gym workspace");
  if (!path) return null;
  const collection = registry.addManaged(await backend.open(path, true));
  if (collection.selectedId) await recordCompletedProjectAction(collection.selectedId, "project.connected");
  return collection;
});
ipcMain.handle("encoder-gym:choose-model", async () => {
  const path = await pickFolder("Choose a local encoder checkpoint");
  return path ? backend.chooseModel(path) : null;
});
ipcMain.handle("encoder-gym:choose-parent", async () => {
  const path = await pickFolder("Choose where Encoder Gym will create the project", app.getPath("documents"));
  return path ? backend.chooseParent(path) : null;
});
ipcMain.handle("encoder-gym:create-managed", async (_event, value: unknown) => {
  const collection = await backend.create(value);
  if (collection.selectedId) await recordCompletedProjectAction(collection.selectedId, "project.created");
  return collection;
});
ipcMain.handle("encoder-gym:choose-dataset", async (_event, value: unknown, purpose: unknown) => {
  const id = projectId(value), role = datasetPurpose(purpose);
  await backend.openRegistered(id);
  let path: string | undefined;
  if (smokeTest) { path = smokeFolderChoice; smokeFolderChoice = undefined; }
  else {
    const result = await dialog.showOpenDialog({ title: "Import local JSONL data", properties: ["openFile"], filters: [{ name: "JSON Lines", extensions: ["jsonl", "ndjson"] }] });
    if (!result.canceled) path = result.filePaths[0];
  }
  return path ? backend.chooseDataset(id, path, role) : null;
});
ipcMain.handle("encoder-gym:query-datasets", (_event, value: unknown, request: unknown) => backend.datasetVersions.query(projectId(value), request));
ipcMain.handle("encoder-gym:mutate-dataset", (_event, value: unknown, request: unknown) => backend.datasetVersions.mutate(projectId(value), request));
ipcMain.handle("encoder-gym:import-dataset", async (_event, value: unknown, token: unknown, name: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "dataset.import", [], async () => {
    const workspace = await backend.importDataset(id, token, name);
    return { project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(workspace) } } satisfies OpenedProject;
  }, result => result.content.state === "ready" ? activityReference("dataset", result.content.workspace.managed?.datasets.at(-1)?.id) : []);
});
ipcMain.handle("encoder-gym:verify-managed", async (_event, value: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "project.verify", [], async () =>
    ({ project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(await backend.openRegistered(id, true)) } } satisfies OpenedProject));
});
ipcMain.handle("encoder-gym:upgrade-managed", async (_event, value: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "project.upgrade", [], async () =>
    ({ project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(await backend.upgradeRegistered(id)) } } satisfies OpenedProject));
});
ipcMain.handle("encoder-gym:managed-readiness", (_event, value: unknown, manifestToken: unknown) => {
  const id = projectId(value);
  if (manifestToken !== undefined && typeof manifestToken !== "string") throw new Error("Invalid optimization selection.");
  return desktopReadiness(id, manifestToken);
});
ipcMain.handle("encoder-gym:prepare-optimization", async (_event, value: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "optimization.prepare", [], () => backend.prepareOptimization(id));
});
ipcMain.handle("encoder-gym:choose-optimization-manifest", async (_event, value: unknown) => {
  const id = projectId(value);
  await backend.openRegistered(id);
  let path: string | undefined;
  if (smokeTest) { path = smokeFolderChoice; smokeFolderChoice = undefined; }
  else {
    const result = await dialog.showOpenDialog({ title: "Choose a reviewed optimization definition", properties: ["openFile"], filters: [{ name: "Optimization definition", extensions: ["toml"] }] });
    if (!result.canceled) path = result.filePaths[0];
  }
  if (!path) return null;
  const selected = await backend.chooseOptimizationManifest(id, path);
  return { ...selected, readiness: withDesktopCredentialAvailability(selected.readiness, desktopProviderStatus(await backend.providerStatus(id))) };
});
ipcMain.handle("encoder-gym:managed-optimize", (_event, value: unknown, request: unknown) => {
  const id = projectId(value), action = (request as { action?: unknown; runId?: unknown })?.action;
  const operations: Record<string, string> = {
    start: "optimization.reserve", resume: "optimization.resume", "authorize-external": "optimization.authorize_external",
    "authorize-sealed": "optimization.authorize_sealed", cancel: "optimization.cancel",
  };
  if (typeof action !== "string" || !operations[action]) return backend.optimize(id, request);
  const refs = activityReference("run", (request as { runId?: unknown }).runId);
  return trackProjectAction(id, operations[action], refs, progress => backend.optimize(id, request, progress), result =>
    activityReference("run", (result as { run_id?: unknown }).run_id));
});
ipcMain.handle("encoder-gym:promote-accepted", async (_event, value: unknown, request: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "model.promote", activityReference("run", (request as { runId?: unknown })?.runId), async () => {
    const workspace = await backend.promoteAccepted(id, request);
    return { project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(workspace) } } satisfies OpenedProject;
  }, result => result.content.state === "ready" ? activityReference("baseline_revision", result.content.workspace.managed?.modelCatalog?.activeBaselineRevisionId) : []);
});
ipcMain.handle("encoder-gym:managed-providers", async (_event, value: unknown) => desktopProviderStatus(await backend.providerStatus(projectId(value))));
ipcMain.handle("encoder-gym:configure-managed-providers", async (_event, value: unknown, settings: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "providers.configure", [], async () => desktopProviderStatus(await backend.configureProviders(id, settings)), status => activityReference("provider_catalog", status.catalog?.id));
});
ipcMain.handle("encoder-gym:set-provider-credential", async (_event, value: unknown, roleValue: unknown, secret: unknown) => {
  const id = projectId(value), role = providerRole(roleValue), status = await backend.providerStatus(id);
  const provider = status.catalog?.providers.find(candidate => candidate.role === role);
  if (!provider?.secret || provider.authentication !== "bearer" || provider.secret.id !== `${id}:${role}`) throw new Error("Configure bearer authentication for this provider before saving its credential.");
  const credentialId = provider.secret.id;
  return trackProjectAction(id, "credential.save", activityReference("provider_role", role), () => {
    credentials.set(credentialId, secret);
    return desktopProviderStatus(status);
  });
});
ipcMain.handle("encoder-gym:remove-provider-credential", async (_event, value: unknown, roleValue: unknown) => {
  const id = projectId(value), role = providerRole(roleValue), status = await backend.providerStatus(id);
  const provider = status.catalog?.providers.find(candidate => candidate.role === role);
  return trackProjectAction(id, "credential.remove", activityReference("provider_role", role), () => {
    if (provider?.secret?.id === `${id}:${role}`) credentials.remove(provider.secret.id);
    return desktopProviderStatus(status);
  });
});
ipcMain.handle("encoder-gym:choose-nomos-runtime", async (_event, value: unknown) => {
  const id = projectId(value), path = await pickFolder("Choose the isolated Nomos runtime");
  return path ? backend.chooseNomosRuntime(id, path) : null;
});
ipcMain.handle("encoder-gym:choose-nomos-python", async (_event, value: unknown) => {
  const id = projectId(value), path = await pickExecutable("Choose the Python executable for this runtime");
  return path ? backend.chooseNomosPython(id, path) : null;
});
ipcMain.handle("encoder-gym:choose-nomos-history", async (_event, value: unknown) => {
  const id = projectId(value);
  await backend.openRegistered(id);
  let path: string | undefined;
  if (smokeTest) { path = smokeFolderChoice; smokeFolderChoice = undefined; }
  else {
    const result = await dialog.showOpenDialog({ title: "Choose existing Encoder Gym scientific history", properties: ["openFile"], filters: [{ name: "Encoder Gym SQLite", extensions: ["sqlite", "db"] }] });
    if (!result.canceled) path = result.filePaths[0];
  }
  return path ? backend.chooseNomosHistory(id, path) : null;
});
ipcMain.handle("encoder-gym:preview-nomos-binding", (_event, value: unknown, runtimeToken: unknown, pythonToken: unknown, historyToken: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "runtime.verify", [], () => backend.previewNomosBinding(id, runtimeToken, pythonToken, historyToken));
});
ipcMain.handle("encoder-gym:prepare-nomos-python", (_event, value: unknown, previewToken: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "runtime.prepare_python", [], () => backend.prepareNomosPython(id, previewToken));
});
ipcMain.handle("encoder-gym:bind-nomos", async (_event, value: unknown, previewToken: unknown) => {
  const id = projectId(value);
  return trackProjectAction(id, "runtime.bind", [], async () => {
    const workspace = await backend.bindNomos(id, previewToken);
    return { project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(workspace) } } satisfies OpenedProject;
  }, result => result.content.state === "ready" ? activityReference("scientific_binding", result.content.workspace.managed?.scientificBinding?.id) : []);
});

ipcMain.handle("encoder-gym:project-activity", (_event, value: unknown, limit: unknown) => {
  const parsed = limit === undefined ? 100 : limit;
  if (typeof parsed !== "number") throw new Error("Invalid activity limit.");
  return backend.projectActivity(projectId(value), parsed);
});
ipcMain.handle("encoder-gym:export-project-activity", async (_event, value: unknown) => {
  const id = projectId(value), project = registry.get(id);
  if (smokeTest) return null;
  const selected = await dialog.showSaveDialog({ title: "Export project activity", defaultPath: `${project.name}-activity.jsonl`, filters: [{ name: "JSON Lines", extensions: ["jsonl"] }] });
  return selected.canceled || !selected.filePath ? null : backend.exportProjectActivity(id, selected.filePath);
});

ipcMain.handle("encoder-gym:window-action", (event, action: unknown) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!window) return;
  if (action === "minimize") window.minimize();
  else if (action === "maximize") window.isMaximized() ? window.unmaximize() : window.maximize();
  else if (action === "close") window.close();
});

ipcMain.handle("encoder-gym:copy-text", (_event, value: unknown) => {
  if (typeof value !== "string") throw new Error("Clipboard text must be a string");
  clipboard.writeText(value);
});

function bindNavigationCommands(window: BrowserWindow): void {
  window.on("app-command", (_event, command) => {
    if (command === "browser-backward") window.webContents.send("encoder-gym:navigation-command", "back");
    else if (command === "browser-forward") window.webContents.send("encoder-gym:navigation-command", "forward");
  });
}

function createWindow(): void {
  const window = new BrowserWindow({
    width: 1440,
    height: 960,
    minWidth: 760,
    minHeight: 560,
    frame: false,
    autoHideMenuBar: true,
    show: false,
    backgroundColor: "#131719",
    webPreferences: {
      preload: join(directory, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  });
  bindNavigationCommands(window);
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", (event, url) => {
    if (url !== window.webContents.getURL()) event.preventDefault();
  });
  window.once("ready-to-show", () => window.show());
  void window.loadFile(join(directory, "renderer", "index.html"));
}

if (smokeTest || verifyCurrent) {
  app.whenReady().then(async () => {
    const window = new BrowserWindow({
      show: false,
      width: 1440,
      height: 960,
      webPreferences: {
        preload: join(directory, "preload.cjs"),
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true,
        backgroundThrottling: false,
      },
    });
    bindNavigationCommands(window);
    try {
      await window.loadFile(join(directory, "renderer", "index.html"));
      if (verifyCurrent) await checkCurrentManaged(window, join(directory, "..", "qa"), registry, backend);
      else await (process.argv.includes("--smoke-managed") ? runManagedSmokeChecks : runSmokeChecks)(window, join(directory, "..", "qa"), { registry, backend, chooseFolder: path => { smokeFolderChoice = path; }, restart: process.argv.includes("--smoke-restart") });
      window.destroy(); app.quit();
    } catch (error) { console.error(error); window.destroy(); app.exit(1); }
  });
  app.on("window-all-closed", () => app.quit());
} else {
  const primaryInstance = app.requestSingleInstanceLock();
  if (!primaryInstance) {
    app.quit();
  } else {
    app.whenReady().then(() => {
      createWindow();
      app.on("activate", () => {
        if (BrowserWindow.getAllWindows().length === 0) createWindow();
      });
    });
    app.on("second-instance", () => {
      const window = BrowserWindow.getAllWindows()[0];
      if (window) { if (window.isMinimized()) window.restore(); window.show(); window.focus(); }
    });
    app.on("window-all-closed", () => {
      if (process.platform !== "darwin") app.quit();
    });
  }
}
