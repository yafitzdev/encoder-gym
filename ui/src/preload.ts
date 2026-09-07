import { contextBridge, ipcRenderer } from "electron";
import type { WorkspaceSnapshot } from "./workspace.js";

export interface EncoderGymBridge {
  loadWorkspace(): Promise<WorkspaceSnapshot>;
  chooseWorkspace(): Promise<WorkspaceSnapshot | null>;
  windowAction(action: "minimize" | "maximize" | "close"): Promise<void>;
  copyText(value: string): Promise<void>;
  versions(): { electron: string; chrome: string; node: string };
}

const bridge: EncoderGymBridge = {
  loadWorkspace: () => ipcRenderer.invoke("encoder-gym:load-workspace"),
  chooseWorkspace: () => ipcRenderer.invoke("encoder-gym:choose-workspace"),
  windowAction: (action) => ipcRenderer.invoke("encoder-gym:window-action", action),
  copyText: (value) => ipcRenderer.invoke("encoder-gym:copy-text", value),
  versions: () => {
    const versions = (process as { versions?: Record<string, string | undefined> }).versions ?? {};
    return {
      electron: versions.electron ?? "unknown",
      chrome: versions.chrome ?? "unknown",
      node: versions.node ?? "unknown",
    };
  },
};

contextBridge.exposeInMainWorld("encoderGym", Object.freeze(bridge));
