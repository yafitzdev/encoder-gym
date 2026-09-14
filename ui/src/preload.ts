import { contextBridge, ipcRenderer } from "electron";
import { validateNativeProgress } from "./native-progress.js";
import type { OptimizationSelection, OptimizationSetup, OptimizationSetupPreview, OptimizationSetupRequest, OptimizationSetupSaved } from "./optimization-setup.js";
import type { OptimizationLaunchAuthorization, OptimizationLaunchPreview, OptimizationLaunchRequest, OptimizationLaunchSaved } from "./optimization-launch.js";
import type { DatasetQuery, DatasetQueryResult, DatasetMutation, DatasetMutationResult } from "./dataset-workspace.js";
import type { BenchmarkAdoption, BenchmarkAdoptionResult, BenchmarkInitializationResult, BenchmarkPreview, BenchmarkQuery, BenchmarkQueryResult } from "./benchmark-workspace.js";
import type { ManagedBaselineRestorationRequest, ManagedOptimizationRequest, ManagedOptimizationResult, ManagedPromotionRequest, ManagedProviderStatus, ManagedReadiness, NativePathChoice, NativeProgress, NomosBindingPreview, OptimizationManifestChoice, PreparedOptimizationChoice, ProviderRole, ProviderSettingsRequest } from "./managed-control.js";
import type { OpenedProject, ProjectCollection } from "./projects.js";
import type { CreateProjectRequest, DatasetChoice, DatasetPurpose, FolderChoice, ModelChoice } from "./managed-workspace.js";
import type { ProjectActivityExport, ProjectActivityLog } from "./project-activity.js";
import type { InputOptimizationRun, InputOptimizationStarted } from "./input-optimization.js";
import type { ProjectProviderConnections, ProviderAssignmentRequest } from "./provider-connections.js";

export interface EncoderGymBridge {
  optimizationSetups(id: string): Promise<OptimizationSetup[]>;
  previewOptimizationSetup(id: string, request: OptimizationSelection, progress?: (value: NativeProgress) => void, preparationId?: string): Promise<OptimizationSetupPreview>;
  saveOptimizationSetup(id: string, request: OptimizationSetupRequest, progress?: (value: NativeProgress) => void, preparationId?: string): Promise<OptimizationSetupSaved>;
  optimizationLaunches(id: string): Promise<OptimizationLaunchAuthorization[]>;
  previewOptimizationLaunch(id: string, setupId: string): Promise<OptimizationLaunchPreview>;
  authorizeOptimizationLaunch(id: string, request: OptimizationLaunchRequest): Promise<OptimizationLaunchSaved>;
  startInputOptimization(id: string, setupId: string, progress?: (value: NativeProgress) => void, preparationId?: string): Promise<InputOptimizationStarted>;
  stopInputPreparation(id: string, preparationId: string): Promise<void>;
  finishInputPreparation(id: string, preparationId: string): Promise<void>;
  driveInputOptimization(id: string, runId: string, progress?: (value: NativeProgress) => void): Promise<InputOptimizationRun>;
  cancelInputOptimization(id: string, runId: string): Promise<InputOptimizationRun>;
  stopInputOptimization(id: string, runId: string): Promise<void>;
  inputOptimizationRun(id: string, runId: string): Promise<InputOptimizationRun>;
  inputOptimizationRuns(id: string): Promise<InputOptimizationRun[]>;
  queryBenchmarks(id: string, request: BenchmarkQuery): Promise<BenchmarkQueryResult>;
  initializeBenchmark(id: string, progress?: (value: NativeProgress) => void, preparationId?: string): Promise<BenchmarkInitializationResult>;
  previewBenchmark(id: string, runId: string): Promise<BenchmarkPreview>;
  adoptBenchmark(id: string, request: BenchmarkAdoption): Promise<BenchmarkAdoptionResult>;
  queryDatasets(id: string, request: DatasetQuery): Promise<DatasetQueryResult>;
  mutateDataset(id: string, request: DatasetMutation): Promise<DatasetMutationResult>;
  openManagedProject(): Promise<ProjectCollection | null>;
  chooseLocalModel(): Promise<ModelChoice | null>;
  chooseProjectParent(): Promise<FolderChoice | null>;
  createManagedProject(request: CreateProjectRequest): Promise<ProjectCollection>;
  chooseDataset(id: string, purpose: DatasetPurpose): Promise<DatasetChoice | null>;
  importDataset(id: string, token: string, name: string): Promise<OpenedProject>;
  verifyManagedProject(id: string): Promise<OpenedProject>;
  upgradeManagedProject(id: string): Promise<OpenedProject>;
  managedReadiness(id: string, manifestToken?: string): Promise<ManagedReadiness>;
  prepareOptimization(id: string): Promise<PreparedOptimizationChoice>;
  chooseOptimizationManifest(id: string): Promise<OptimizationManifestChoice | null>;
  managedOptimize(id: string, request: ManagedOptimizationRequest): Promise<ManagedOptimizationResult>;
  promoteAccepted(id: string, request: ManagedPromotionRequest): Promise<OpenedProject>;
  restoreBaseline(id: string, request: ManagedBaselineRestorationRequest): Promise<OpenedProject>;
  managedProviders(id: string): Promise<ManagedProviderStatus>;
  configureManagedProviders(id: string, request: ProviderSettingsRequest): Promise<ManagedProviderStatus>;
  setProviderCredential(id: string, role: ProviderRole, secret: string): Promise<ManagedProviderStatus>;
  removeProviderCredential(id: string, role: ProviderRole): Promise<ManagedProviderStatus>;
  providerConnections(id: string): Promise<ProjectProviderConnections>;
  addProviderConnection(id: string, request: { endpoint: string; apiKey: string }): Promise<ProjectProviderConnections>;
  refreshProviderConnection(id: string, connectionId: string): Promise<ProjectProviderConnections>;
  removeProviderConnection(id: string, connectionId: string): Promise<ProjectProviderConnections>;
  assignProviderModels(id: string, request: ProviderAssignmentRequest): Promise<ManagedProviderStatus>;
  chooseNomosRuntime(id: string): Promise<NativePathChoice | null>;
  chooseNomosPython(id: string): Promise<NativePathChoice | null>;
  chooseNomosHistory(id: string): Promise<NativePathChoice | null>;
  previewNomosBinding(id: string, runtimeToken: string, pythonToken: string, historyToken?: string): Promise<NomosBindingPreview>;
  prepareNomosPython(id: string, previewToken: string): Promise<void>;
  bindNomos(id: string, previewToken: string): Promise<OpenedProject>;
  projectActivity(id: string, limit?: number, runId?: string): Promise<ProjectActivityLog>;
  exportProjectActivity(id: string): Promise<ProjectActivityExport | null>;
  getProjects(): Promise<ProjectCollection>;
  addProjectFolder(): Promise<ProjectCollection | null>;
  openExample(): Promise<ProjectCollection>;
  selectProject(id: string): Promise<OpenedProject>;
  renameProject(id: string, name: string): Promise<ProjectCollection>;
  relocateProject(id: string): Promise<ProjectCollection | null>;
  forgetProject(id: string): Promise<ProjectCollection>;
  windowAction(action: "minimize" | "maximize" | "close"): Promise<void>;
  onNavigationCommand(handler: (direction: "back" | "forward") => void): void;
  copyText(value: string): Promise<void>;
  versions(): { electron: string; chrome: string; node: string };
}

const bridge: EncoderGymBridge = {
  optimizationSetups: id => ipcRenderer.invoke("encoder-gym:optimization-setups", id),
  previewOptimizationSetup: (id, request, progress, preparationId) => invokeWithProgress("encoder-gym:preview-optimization-setup", id, request, progress, preparationId),
  saveOptimizationSetup: (id, request, progress, preparationId) => invokeWithProgress("encoder-gym:save-optimization-setup", id, request, progress, preparationId),
  optimizationLaunches: id => ipcRenderer.invoke("encoder-gym:optimization-launches", id),
  previewOptimizationLaunch: (id, setupId) => ipcRenderer.invoke("encoder-gym:preview-optimization-launch", id, setupId),
  authorizeOptimizationLaunch: (id, request) => ipcRenderer.invoke("encoder-gym:authorize-optimization-launch", id, request),
  startInputOptimization: (id, setupId, progress, preparationId) => invokeWithProgress("encoder-gym:start-input-optimization", id, setupId, progress, preparationId),
  stopInputPreparation: (id, token) => ipcRenderer.invoke("encoder-gym:stop-input-preparation", id, token),
  finishInputPreparation: (id, token) => ipcRenderer.invoke("encoder-gym:finish-input-preparation", id, token),
  driveInputOptimization: (id, runId, progress) => invokeWithProgress("encoder-gym:drive-input-optimization", id, runId, progress),
  cancelInputOptimization: (id, runId) => ipcRenderer.invoke("encoder-gym:cancel-input-optimization", id, runId),
  stopInputOptimization: (id, runId) => ipcRenderer.invoke("encoder-gym:stop-input-optimization", id, runId),
  inputOptimizationRun: (id, runId) => ipcRenderer.invoke("encoder-gym:input-optimization-run", id, runId),
  inputOptimizationRuns: id => ipcRenderer.invoke("encoder-gym:input-optimization-runs", id),
  queryBenchmarks: (id, request) => ipcRenderer.invoke("encoder-gym:query-benchmarks", id, request),
  initializeBenchmark: (id, receive, preparationId) => {
    const requestId = globalThis.crypto.randomUUID();
    const listener = (_event: Electron.IpcRendererEvent, value: unknown, progress: unknown): void => {
      if (value !== requestId || !receive) return;
      const parsed = nativeProgress(progress);
      if (parsed) receive(parsed);
    };
    ipcRenderer.on("encoder-gym:benchmark-progress", listener);
    return ipcRenderer.invoke("encoder-gym:initialize-benchmark", id, requestId, preparationId)
      .finally(() => ipcRenderer.removeListener("encoder-gym:benchmark-progress", listener));
  },
  previewBenchmark: (id, runId) => ipcRenderer.invoke("encoder-gym:preview-benchmark", id, runId),
  adoptBenchmark: (id, request) => ipcRenderer.invoke("encoder-gym:adopt-benchmark", id, request),
  queryDatasets: (id, request) => ipcRenderer.invoke("encoder-gym:query-datasets", id, request),
  mutateDataset: (id, request) => ipcRenderer.invoke("encoder-gym:mutate-dataset", id, request),
  openManagedProject: () => ipcRenderer.invoke("encoder-gym:open-managed"),
  chooseLocalModel: () => ipcRenderer.invoke("encoder-gym:choose-model"),
  chooseProjectParent: () => ipcRenderer.invoke("encoder-gym:choose-parent"),
  createManagedProject: request => ipcRenderer.invoke("encoder-gym:create-managed", request),
  chooseDataset: (id, purpose) => ipcRenderer.invoke("encoder-gym:choose-dataset", id, purpose),
  importDataset: (id, token, name) => ipcRenderer.invoke("encoder-gym:import-dataset", id, token, name),
  verifyManagedProject: id => ipcRenderer.invoke("encoder-gym:verify-managed", id),
  upgradeManagedProject: id => ipcRenderer.invoke("encoder-gym:upgrade-managed", id),
  managedReadiness: (id, manifestToken) => ipcRenderer.invoke("encoder-gym:managed-readiness", id, manifestToken),
  prepareOptimization: id => ipcRenderer.invoke("encoder-gym:prepare-optimization", id),
  chooseOptimizationManifest: id => ipcRenderer.invoke("encoder-gym:choose-optimization-manifest", id),
  managedOptimize: (id, request) => ipcRenderer.invoke("encoder-gym:managed-optimize", id, request),
  promoteAccepted: (id, request) => ipcRenderer.invoke("encoder-gym:promote-accepted", id, request),
  restoreBaseline: (id, request) => ipcRenderer.invoke("encoder-gym:restore-baseline", id, request),
  managedProviders: id => ipcRenderer.invoke("encoder-gym:managed-providers", id),
  configureManagedProviders: (id, request) => ipcRenderer.invoke("encoder-gym:configure-managed-providers", id, request),
  setProviderCredential: (id, role, secret) => ipcRenderer.invoke("encoder-gym:set-provider-credential", id, role, secret),
  removeProviderCredential: (id, role) => ipcRenderer.invoke("encoder-gym:remove-provider-credential", id, role),
  providerConnections: id => ipcRenderer.invoke("encoder-gym:provider-connections", id),
  addProviderConnection: (id, request) => ipcRenderer.invoke("encoder-gym:add-provider-connection", id, request),
  refreshProviderConnection: (id, connectionId) => ipcRenderer.invoke("encoder-gym:refresh-provider-connection", id, connectionId),
  removeProviderConnection: (id, connectionId) => ipcRenderer.invoke("encoder-gym:remove-provider-connection", id, connectionId),
  assignProviderModels: (id, request) => ipcRenderer.invoke("encoder-gym:assign-provider-models", id, request),
  chooseNomosRuntime: id => ipcRenderer.invoke("encoder-gym:choose-nomos-runtime", id),
  chooseNomosPython: id => ipcRenderer.invoke("encoder-gym:choose-nomos-python", id),
  chooseNomosHistory: id => ipcRenderer.invoke("encoder-gym:choose-nomos-history", id),
  previewNomosBinding: (id, runtimeToken, pythonToken, historyToken) => ipcRenderer.invoke("encoder-gym:preview-nomos-binding", id, runtimeToken, pythonToken, historyToken),
  prepareNomosPython: (id, previewToken) => ipcRenderer.invoke("encoder-gym:prepare-nomos-python", id, previewToken),
  bindNomos: (id, previewToken) => ipcRenderer.invoke("encoder-gym:bind-nomos", id, previewToken),
  projectActivity: (id, limit, runId) => ipcRenderer.invoke("encoder-gym:project-activity", id, limit, runId),
  exportProjectActivity: id => ipcRenderer.invoke("encoder-gym:export-project-activity", id),
  getProjects: () => ipcRenderer.invoke("encoder-gym:get-projects"),
  addProjectFolder: () => ipcRenderer.invoke("encoder-gym:add-project-folder"),
  openExample: () => ipcRenderer.invoke("encoder-gym:open-example"),
  selectProject: id => ipcRenderer.invoke("encoder-gym:select-project", id),
  renameProject: (id, name) => ipcRenderer.invoke("encoder-gym:rename-project", id, name),
  relocateProject: id => ipcRenderer.invoke("encoder-gym:relocate-project", id),
  forgetProject: id => ipcRenderer.invoke("encoder-gym:forget-project", id),
  windowAction: (action) => ipcRenderer.invoke("encoder-gym:window-action", action),
  onNavigationCommand: handler => {
    ipcRenderer.on("encoder-gym:navigation-command", (_event, direction: unknown) => {
      if (direction === "back" || direction === "forward") handler(direction);
    });
  },
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

const nativeProgress = validateNativeProgress;
function invokeWithProgress<T>(channel: string, id: string, request: unknown, receive?: (value: NativeProgress) => void, preparationId?: string): Promise<T> {
  const token = globalThis.crypto.randomUUID();
  const listener = (_event: Electron.IpcRendererEvent, receivedToken: unknown, value: unknown): void => {
    if (receivedToken !== token) return;
    const progress = nativeProgress(value);
    if (progress) receive?.(progress);
  };
  ipcRenderer.on("encoder-gym:optimization-progress", listener);
  return ipcRenderer.invoke(channel, id, request, token, preparationId).finally(() => ipcRenderer.removeListener("encoder-gym:optimization-progress", listener));
}
