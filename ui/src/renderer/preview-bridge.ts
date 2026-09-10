import type { EncoderGymBridge } from "../preload.js";
import type { ProjectCollection } from "../projects.js";
import { example, readExample } from "../evidence/examples.js";

/** Browser preview is explicitly limited to the recorded example, never local files. */
export function previewBridge(): EncoderGymBridge {
  let collection: ProjectCollection = { version: 1, selectedId: null, projects: [] };
  const unavailable = async (): Promise<never> => { throw new Error("Open the Encoder Gym desktop app to manage local project folders."); };
  return {
    openManagedProject: unavailable, chooseLocalModel: unavailable, chooseProjectParent: unavailable, createManagedProject: unavailable,
    chooseDataset: unavailable, importDataset: unavailable, verifyManagedProject: unavailable,
    upgradeManagedProject: unavailable, managedReadiness: unavailable, prepareOptimization: unavailable, chooseOptimizationManifest: unavailable, managedOptimize: unavailable, promoteAccepted: unavailable,
    managedProviders: unavailable, configureManagedProviders: unavailable, setProviderCredential: unavailable, removeProviderCredential: unavailable,
    chooseNomosRuntime: unavailable, chooseNomosPython: unavailable, chooseNomosHistory: unavailable, previewNomosBinding: unavailable, prepareNomosPython: unavailable, bindNomos: unavailable,
    getProjects: async () => structuredClone(collection),
    addProjectFolder: unavailable, relocateProject: unavailable, renameProject: unavailable,
    openExample: async () => {
      if (!collection.projects.length) collection = { version: 1, selectedId: "preview-example", projects: [{ id: "preview-example", name: example.name, createdAt: new Date().toISOString(), source: { kind: "example", key: example.key } }] };
      return structuredClone(collection);
    },
    selectProject: async id => {
      const project = collection.projects.find(p => p.id === id);
      if (!project) throw new Error("Example not found.");
      collection.selectedId = id;
      return { project, content: { state: "ready", workspace: readExample(example.key) } };
    },
    forgetProject: async () => { collection = { version: 1, selectedId: null, projects: [] }; return collection; },
    copyText: async value => navigator.clipboard.writeText(value),
    windowAction: async () => {},
    onNavigationCommand: () => {},
    versions: () => ({ electron: "browser preview", chrome: "browser", node: "none" }),
  };
}
