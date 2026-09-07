import { contextBridge, ipcRenderer } from "electron";
import type { OpenedProject, ProjectCollection } from "./projects.js";

export interface EncoderGymBridge {
  getProjects(): Promise<ProjectCollection>;
  addProjectFolder(): Promise<ProjectCollection | null>;
  openExample(): Promise<ProjectCollection>;
  selectProject(id: string): Promise<OpenedProject>;
  renameProject(id: string, name: string): Promise<ProjectCollection>;
  relocateProject(id: string): Promise<ProjectCollection | null>;
  forgetProject(id: string): Promise<ProjectCollection>;
  windowAction(action: "minimize" | "maximize" | "close"): Promise<void>;
  copyText(value: string): Promise<void>;
  versions(): { electron: string; chrome: string; node: string };
}

const bridge: EncoderGymBridge = {
  getProjects: () => ipcRenderer.invoke("encoder-gym:get-projects"),
  addProjectFolder: () => ipcRenderer.invoke("encoder-gym:add-project-folder"),
  openExample: () => ipcRenderer.invoke("encoder-gym:open-example"),
  selectProject: id => ipcRenderer.invoke("encoder-gym:select-project", id),
  renameProject: (id, name) => ipcRenderer.invoke("encoder-gym:rename-project", id, name),
  relocateProject: id => ipcRenderer.invoke("encoder-gym:relocate-project", id),
  forgetProject: id => ipcRenderer.invoke("encoder-gym:forget-project", id),
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
