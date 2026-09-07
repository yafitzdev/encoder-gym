import { app, BrowserWindow, clipboard, dialog, ipcMain } from "electron";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { readWorkspace } from "./evidence/read-workspace.js";
import recorded from "./evidence/nomos-snapshot.json";
import { runSmokeChecks } from "./smoke-checks.js";

const directory = dirname(fileURLToPath(import.meta.url));
const smokeTest = process.argv.includes("--smoke-test");
let workspaceFolder = recorded.folder;
ipcMain.handle("encoder-gym:load-workspace", () => readWorkspace(workspaceFolder));
ipcMain.handle("encoder-gym:choose-workspace", async () => {
  const selected = await dialog.showOpenDialog({ title: "Open an Encoder Gym experiment workspace", defaultPath: workspaceFolder, properties: ["openDirectory"] });
  if (selected.canceled || !selected.filePaths[0]) return null;
  const snapshot = readWorkspace(selected.filePaths[0]);
  workspaceFolder = selected.filePaths[0];
  return snapshot;
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

if (smokeTest) {
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
      await runSmokeChecks(window, join(directory, "..", "qa"));
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
    app.on("window-all-closed", () => {
      if (process.platform !== "darwin") app.quit();
    });
  }
}
