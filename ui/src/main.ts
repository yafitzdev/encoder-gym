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

const directory = dirname(fileURLToPath(import.meta.url));
const smokeTest = process.argv.includes("--smoke-test");
const verifyCurrent = process.argv.includes("--verify-managed-current");
const actualRegistryFile = join(app.getPath("userData"), "projects.json");
if (smokeTest) app.setPath("userData", process.env.ENCODER_GYM_SMOKE_PROFILE ?? mkdtempSync(join(tmpdir(), "encoder-gym-renderer-")));
if (verifyCurrent) app.setPath("userData", mkdtempSync(join(tmpdir(), "encoder-gym-current-check-")));
const registry = new ProjectRegistry(verifyCurrent ? actualRegistryFile : join(app.getPath("userData"), "projects.json"));
const backend = new ManagedBackend(app.isPackaged ? join(process.resourcesPath, "synth" + (process.platform === "win32" ? ".exe" : "")) : join(directory, "..", "..", "target", "debug", "synth" + (process.platform === "win32" ? ".exe" : "")), registry);
const credentials = new CredentialStore(join(app.getPath("userData"), "credentials.json"), {
  available: () => safeStorage.isEncryptionAvailable(),
  encrypt: value => safeStorage.encryptString(value),
  decrypt: value => safeStorage.decryptString(value),
});
let smokeFolderChoice: string | undefined;
async function pickFolder(title: string, defaultPath?: string): Promise<string | undefined> {
  if (smokeTest) { const choice = smokeFolderChoice; smokeFolderChoice = undefined; return choice; }
  const selected = await dialog.showOpenDialog({ title, defaultPath, properties: ["openDirectory", "createDirectory"] });
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
ipcMain.handle("encoder-gym:rename-project", (_event, id: unknown, name: unknown) => {
  if (typeof name !== "string") throw new Error("A project name is required.");
  return registry.rename(projectId(id), name);
});
ipcMain.handle("encoder-gym:relocate-project", async (_event, value: unknown) => {
  const id = projectId(value), project = registry.get(id);
  if (project.source.kind !== "folder") throw new Error("Recorded examples do not have a connected folder.");
  const selected = await pickFolder(`Reconnect ${project.name}`, project.source.path);
  if (selected && project.source.workspaceId) {
    const workspace = await backend.open(selected, true);
    if (workspace.manifest.id !== project.source.workspaceId) throw new Error("Choose the same Gym project at its new location. This folder has a different project identity.");
    return registry.addManaged(workspace);
  }
  return selected ? registry.relocate(id, selected) : null;
});
ipcMain.handle("encoder-gym:forget-project", (_event, value: unknown) => registry.remove(projectId(value)));

ipcMain.handle("encoder-gym:open-managed", async () => {
  const path = await pickFolder("Open an Encoder Gym workspace");
  return path ? registry.addManaged(await backend.open(path, true)) : null;
});
ipcMain.handle("encoder-gym:choose-model", async () => {
  const path = await pickFolder("Choose a local encoder checkpoint");
  return path ? backend.chooseModel(path) : null;
});
ipcMain.handle("encoder-gym:choose-parent", async () => {
  const path = await pickFolder("Choose where Encoder Gym will create the project", app.getPath("documents"));
  return path ? backend.chooseParent(path) : null;
});
ipcMain.handle("encoder-gym:create-managed", (_event, value: unknown) => backend.create(value));
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
ipcMain.handle("encoder-gym:import-dataset", async (_event, value: unknown, token: unknown, name: unknown) => {
  const id = projectId(value);
  return { project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(await backend.importDataset(id, token, name)) } } satisfies OpenedProject;
});
ipcMain.handle("encoder-gym:verify-managed", async (_event, value: unknown) => {
  const id = projectId(value);
  return { project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(await backend.openRegistered(id, true)) } } satisfies OpenedProject;
});
ipcMain.handle("encoder-gym:upgrade-managed", async (_event, value: unknown) => {
  const id = projectId(value);
  return { project: registry.get(id), content: { state: "ready", workspace: managedSnapshot(await backend.upgradeRegistered(id)) } } satisfies OpenedProject;
});
ipcMain.handle("encoder-gym:managed-readiness", (_event, value: unknown, manifestToken: unknown) => {
  const id = projectId(value);
  if (manifestToken !== undefined && typeof manifestToken !== "string") throw new Error("Invalid optimization selection.");
  return desktopReadiness(id, manifestToken);
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
ipcMain.handle("encoder-gym:managed-optimize", (_event, value: unknown, request: unknown) => backend.optimize(projectId(value), request));
ipcMain.handle("encoder-gym:managed-providers", async (_event, value: unknown) => desktopProviderStatus(await backend.providerStatus(projectId(value))));
ipcMain.handle("encoder-gym:configure-managed-providers", async (_event, value: unknown, settings: unknown) => desktopProviderStatus(await backend.configureProviders(projectId(value), settings)));
ipcMain.handle("encoder-gym:set-provider-credential", async (_event, value: unknown, roleValue: unknown, secret: unknown) => {
  const id = projectId(value), role = providerRole(roleValue), status = await backend.providerStatus(id);
  const provider = status.catalog?.providers.find(candidate => candidate.role === role);
  if (!provider?.secret || provider.authentication !== "bearer" || provider.secret.id !== `${id}:${role}`) throw new Error("Configure bearer authentication for this provider before saving its credential.");
  credentials.set(provider.secret.id, secret);
  return desktopProviderStatus(await backend.providerStatus(id));
});
ipcMain.handle("encoder-gym:remove-provider-credential", async (_event, value: unknown, roleValue: unknown) => {
  const id = projectId(value), role = providerRole(roleValue), status = await backend.providerStatus(id);
  const provider = status.catalog?.providers.find(candidate => candidate.role === role);
  if (provider?.secret?.id === `${id}:${role}`) credentials.remove(provider.secret.id);
  return desktopProviderStatus(await backend.providerStatus(id));
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
