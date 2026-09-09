import { contextBridge, ipcRenderer } from "electron";
import type { ManagedOptimizationRequest, ManagedOptimizationResult, ManagedProviderStatus, ManagedReadiness, NativePathChoice, NomosBindingPreview, OptimizationManifestChoice, ProviderRole, ProviderSettingsRequest } from "./managed-control.js";
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
  upgradeManagedProject(id: string): Promise<OpenedProject>;
  managedReadiness(id: string, manifestToken?: string): Promise<ManagedReadiness>;
  chooseOptimizationManifest(id: string): Promise<OptimizationManifestChoice | null>;
  managedOptimize(id: string, request: ManagedOptimizationRequest): Promise<ManagedOptimizationResult>;
  managedProviders(id: string): Promise<ManagedProviderStatus>;
  configureManagedProviders(id: string, request: ProviderSettingsRequest): Promise<ManagedProviderStatus>;
  setProviderCredential(id: string, role: ProviderRole, secret: string): Promise<ManagedProviderStatus>;
  removeProviderCredential(id: string, role: ProviderRole): Promise<ManagedProviderStatus>;
  chooseNomosRuntime(id: string): Promise<NativePathChoice | null>;
  chooseNomosPython(id: string): Promise<NativePathChoice | null>;
  chooseNomosHistory(id: string): Promise<NativePathChoice | null>;
  previewNomosBinding(id: string, runtimeToken: string, pythonToken: string, historyToken?: string): Promise<NomosBindingPreview>;
  prepareNomosPython(id: string, previewToken: string): Promise<void>;
  bindNomos(id: string, previewToken: string): Promise<OpenedProject>;
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
  upgradeManagedProject: id => ipcRenderer.invoke("encoder-gym:upgrade-managed", id),
  managedReadiness: (id, manifestToken) => ipcRenderer.invoke("encoder-gym:managed-readiness", id, manifestToken),
  chooseOptimizationManifest: id => ipcRenderer.invoke("encoder-gym:choose-optimization-manifest", id),
  managedOptimize: (id, request) => ipcRenderer.invoke("encoder-gym:managed-optimize", id, request),
  managedProviders: id => ipcRenderer.invoke("encoder-gym:managed-providers", id),
  configureManagedProviders: (id, request) => ipcRenderer.invoke("encoder-gym:configure-managed-providers", id, request),
  setProviderCredential: (id, role, secret) => ipcRenderer.invoke("encoder-gym:set-provider-credential", id, role, secret),
  removeProviderCredential: (id, role) => ipcRenderer.invoke("encoder-gym:remove-provider-credential", id, role),
  chooseNomosRuntime: id => ipcRenderer.invoke("encoder-gym:choose-nomos-runtime", id),
  chooseNomosPython: id => ipcRenderer.invoke("encoder-gym:choose-nomos-python", id),
  chooseNomosHistory: id => ipcRenderer.invoke("encoder-gym:choose-nomos-history", id),
  previewNomosBinding: (id, runtimeToken, pythonToken, historyToken) => ipcRenderer.invoke("encoder-gym:preview-nomos-binding", id, runtimeToken, pythonToken, historyToken),
  prepareNomosPython: (id, previewToken) => ipcRenderer.invoke("encoder-gym:prepare-nomos-python", id, previewToken),
  bindNomos: (id, previewToken) => ipcRenderer.invoke("encoder-gym:bind-nomos", id, previewToken),
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
