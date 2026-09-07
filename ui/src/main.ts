import { app, BrowserWindow, clipboard, ipcMain, shell } from "electron";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const directory = dirname(fileURLToPath(import.meta.url));
const smokeTest = process.argv.includes("--smoke-test");

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
    width: 1280,
    height: 820,
    minWidth: 860,
    minHeight: 560,
    frame: false,
    autoHideMenuBar: true,
    show: false,
    backgroundColor: "#ffffff",
    webPreferences: {
      preload: join(directory, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  });
  window.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("https:") || url.startsWith("http:")) void shell.openExternal(url);
    return { action: "deny" };
  });
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
      webPreferences: {
        preload: join(directory, "preload.cjs"),
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true,
      },
    });
    await window.loadFile(join(directory, "renderer", "index.html"));
    console.log("Encoder Gym renderer loaded in smoke mode.");
    window.destroy();
    app.quit();
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
