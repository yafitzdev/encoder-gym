import { contextBridge, ipcRenderer } from "electron";
import type { OpenedProject, ProjectCollection } from "./projects.js";
import type { CreateProjectRequest, DatasetChoice, DatasetPurpose, FolderChoice, ModelChoice } from "./managed-workspace.js";

export interface EncoderGymBridge {
  openManagedProject(): Promise<ProjectCollection | null>;
  chooseLocalModel(): Promise<ModelChoice | null>;
  chooseProjectParent(): Promise<FolderChoice | null>;
  createManagedProject(request: CreateProjectRequest): Promise<ProjectCollection>;
  chooseDataset(id: string, purpose: DatasetPurpose): Promise<DatasetChoice | null>;
  importDataset(id: string, token: string, name: string): Promise<OpenedProject>;
  verifyManagedProject(id: string): Promise<OpenedProject>;
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
  openManagedProject: () => ipcRenderer.invoke("encoder-gym:open-managed"),
  chooseLocalModel: () => ipcRenderer.invoke("encoder-gym:choose-model"),
  chooseProjectParent: () => ipcRenderer.invoke("encoder-gym:choose-parent"),
  createManagedProject: request => ipcRenderer.invoke("encoder-gym:create-managed", request),
  chooseDataset: (id, purpose) => ipcRenderer.invoke("encoder-gym:choose-dataset", id, purpose),
  importDataset: (id, token, name) => ipcRenderer.invoke("encoder-gym:import-dataset", id, token, name),
  verifyManagedProject: id => ipcRenderer.invoke("encoder-gym:verify-managed", id),
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
