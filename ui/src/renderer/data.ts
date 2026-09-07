export type RecipeStatus = "queued" | "running" | "completed" | "failed";
export type DecisionOutcome = "retain-baseline" | "promote-candidate" | "pending";
export type StageState = "completed" | "failed" | "active" | "protected" | "pending";
export type GateState = "passed" | "failed" | "not-run";

export interface EvaluationSuite {
  id: string;
  name: string;
  description: string;
  developmentSuites: string[];
  sealedSuite: string;
}

export interface DatasetSnapshot {
  id: string;
  name: string;
  note: string;
}

export interface ModelSnapshot {
  id: string;
  name: string;
  checksum: string;
  role: "baseline" | "candidate";
}

export interface Instruction {
  label: string;
  value: string;
}

export interface RunStage {
  label: string;
  detail: string;
  state: StageState;
}

export interface EvaluationResult {
  suite: string;
  role: "development" | "sealed";
  metric: string;
  baseline: string;
  candidate: string;
  delta: string;
  gate: GateState;
}

export interface BudgetUsage {
  label: string;
  used: number;
  limit: number;
  unit: string;
}

export interface ProvenanceFact {
  label: string;
  value: string;
}

export interface Recipe {
  id: string;
  projectId: string;
  name: string;
  runId: string;
  description: string;
  hypothesis: string;
  status: RecipeStatus;
  outcome: DecisionOutcome;
  outcomeLabel: string;
  outcomeSummary: string;
  nextAction: string;
  sealedState: "unused" | "used" | "locked";
  datasetSnapshots: DatasetSnapshot[];
  instructions: Instruction[];
  commands: string;
  modelSnapshots: ModelSnapshot[];
  stages: RunStage[];
  evaluations: EvaluationResult[];
  budgets: BudgetUsage[];
  provenance: ProvenanceFact[];
}

export interface Project {
  id: string;
  name: string;
  task: string;
  folder: string;
  description: string;
  createdAt: string;
  revision: string;
  baselineModel: string;
  latestRecipeId: string;
  benchmarkGeneration: string;
  evaluationSuite: EvaluationSuite;
}

export const projects: Project[] = [
  {
    id: "nomos-repair",
    name: "Nomos repair",
    task: "Tool registry ranking",
    folder: "C:\\Users\\yanfi\\PycharmProjects\\nomos-encoder-gym-experiment",
    description: "Recover bounded-change preflight routing without regressing generic retrieval.",
    createdAt: "2026-09-02",
    revision: "4450ab3f1de8a1fc64bcbe5d77c67d0fb0f99af9",
    baselineModel: "Nomos FP32 ONNX baseline",
    latestRecipeId: "recipe-nomos-repair-training",
    benchmarkGeneration: "10cba5de-e501-4169-a603-27f75c2abd37",
    evaluationSuite: {
      id: "nomos-suites",
      name: "Nomos governed benchmark bundle",
      description: "Two development suites and one independently protected promotion suite.",
      developmentSuites: ["generic_holdout", "retired_post_scaling"],
      sealedSuite: "promotion agent suite",
    },
  },
];

const repairSnapshot: DatasetSnapshot[] = [
  {
    id: "2602ef88-db56-4591-8a4c-2581c5613726",
    name: "Repair training snapshot",
    note: "6,800 immutable base rows + 192 reviewed repair rows",
  },
];

export const recipes: Recipe[] = [
  {
    id: "recipe-nomos-repair-training",
    projectId: "nomos-repair",
    name: "Repair training · run 01",
    runId: "2317e08b-5848-4773-9a9e-42499ee09815",
    description: "A genuine CPU triplet fine-tune over the approved repair population.",
    hypothesis: "Introduce the reviewed repair signal through a bounded full-model fine-tune while preserving both development suites.",
    status: "completed",
    outcome: "retain-baseline",
    outcomeLabel: "Baseline retained",
    outcomeSummary: "The candidate failed unchanged gates in both development suites. No sealed evidence was exposed.",
    nextAction: "Create new immutable authority for one conservative, predeclared training hypothesis.",
    sealedState: "unused",
    datasetSnapshots: repairSnapshot,
    instructions: [
      { label: "Trainer", value: "continued triplet fine-tune" },
      { label: "Epochs", value: "1" },
      { label: "Batch size", value: "16" },
      { label: "Learning rate", value: "5e-6" },
      { label: "Margin", value: "0.2" },
    ],
    commands: "synth encoder optimize status 2317e08b-5848-4773-9a9e-42499ee09815 --workspace C:\\Users\\yanfi\\PycharmProjects\\nomos-encoder-gym-experiment\nsynth encoder optimize report 2317e08b-5848-4773-9a9e-42499ee09815 --workspace C:\\Users\\yanfi\\PycharmProjects\\nomos-encoder-gym-experiment\nsynth encoder optimize doctor 2317e08b-5848-4773-9a9e-42499ee09815 --workspace C:\\Users\\yanfi\\PycharmProjects\\nomos-encoder-gym-experiment",
    modelSnapshots: [
      { id: "44b98240-6b63-49ca-a0c3-21bddba1d151", name: "CPU triplet candidate", checksum: "sha256:36289478c89d50b3…2799171", role: "candidate" },
    ],
    stages: [
      { label: "Authority", detail: "Snapshot verified", state: "completed" },
      { label: "Train", detail: "1 candidate", state: "completed" },
      { label: "Development", detail: "0 eligible", state: "failed" },
      { label: "Sealed", detail: "Unused", state: "protected" },
      { label: "Decision", detail: "Retain baseline", state: "completed" },
    ],
    evaluations: [
      { suite: "generic_holdout", role: "development", metric: "MRR", baseline: "0.895654", candidate: "0.895627", delta: "−0.000026", gate: "failed" },
      { suite: "generic_holdout", role: "development", metric: "Recall@2", baseline: "0.906", candidate: "0.901", delta: "−0.005", gate: "failed" },
      { suite: "retired_post_scaling", role: "development", metric: "MRR", baseline: "0.966499", candidate: "0.885465", delta: "−0.081034", gate: "failed" },
    ],
    budgets: [
      { label: "Candidates", used: 1, limit: 1, unit: "trained" },
      { label: "Development", used: 2, limit: 2, unit: "suites" },
      { label: "Sealed exposure", used: 0, limit: 1, unit: "reports" },
      { label: "External calls", used: 0, limit: 0, unit: "calls" },
    ],
    provenance: [
      { label: "Optimization run", value: "2317e08b-5848-4773-9a9e-42499ee09815" },
      { label: "Candidate", value: "44b98240-6b63-49ca-a0c3-21bddba1d151" },
      { label: "Journal head", value: "sha256:37d6e0a16516da7…e6ed691" },
      { label: "Evidence bundle", value: "sha256:19d30e96ebe62da7…50eb4" },
    ],
  },
  {
    id: "recipe-nomos-interpolation",
    projectId: "nomos-repair",
    name: "Interpolation · qualification",
    runId: "c4e1b908-ed55-434e-928d-ffe321b7a799",
    description: "A bounded interpolation experiment that passed development but failed sealed acceptance.",
    hypothesis: "A small interpolation toward the repair checkpoint can improve development retrieval without changing agent outcomes.",
    status: "completed",
    outcome: "retain-baseline",
    outcomeLabel: "Baseline retained",
    outcomeSummary: "The selected candidate passed development, then failed strict sealed MRR and Recall@1 gates.",
    nextAction: "Do not tune against the revealed sealed cohort; use a successor acceptance generation.",
    sealedState: "used",
    datasetSnapshots: [
      { id: "historical-protocol-inputs", name: "Frozen protocol inputs", note: "Historical immutable interpolation authority" },
    ],
    instructions: [
      { label: "Transformation", value: "checkpoint interpolation" },
      { label: "Selected weight", value: "5%" },
      { label: "Inference policy", value: "top-one + recovery" },
    ],
    commands: "synth encoder-experiment inspect c4e1b908-ed55-434e-928d-ffe321b7a799 --workspace C:\\Users\\yanfi\\PycharmProjects\\nomos-encoder-gym-experiment",
    modelSnapshots: [
      { id: "historical-selected-candidate", name: "5% interpolation candidate", checksum: "sha256:95943e3eeeb1224c…ff856c", role: "candidate" },
    ],
    stages: [
      { label: "Authority", detail: "Protocol frozen", state: "completed" },
      { label: "Transform", detail: "Finite set", state: "completed" },
      { label: "Development", detail: "Candidate selected", state: "completed" },
      { label: "Sealed", detail: "Gate failed", state: "failed" },
      { label: "Decision", detail: "Retain baseline", state: "completed" },
    ],
    evaluations: [
      { suite: "generic_holdout", role: "development", metric: "MRR", baseline: "0.895654", candidate: "0.898004", delta: "+0.002350", gate: "passed" },
      { suite: "promotion suite", role: "sealed", metric: "MRR", baseline: "0.966499", candidate: "0.965600", delta: "−0.000899", gate: "failed" },
      { suite: "promotion suite", role: "sealed", metric: "Recall@1", baseline: "0.959722", candidate: "0.958333", delta: "−0.001389", gate: "failed" },
    ],
    budgets: [
      { label: "Candidates", used: 4, limit: 4, unit: "created" },
      { label: "Development", used: 4, limit: 4, unit: "reports" },
      { label: "Sealed exposure", used: 1, limit: 1, unit: "reports" },
    ],
    provenance: [
      { label: "Experiment run", value: "c4e1b908-ed55-434e-928d-ffe321b7a799" },
      { label: "Selected model", value: "sha256:95943e3eeeb1224c…ff856c" },
      { label: "Decision", value: "retain_baseline" },
    ],
  },
];

export const activeProjectId = "nomos-repair";

export function projectById(id: string): Project | undefined {
  return projects.find((item) => item.id === id);
}

export function recipeById(id: string): Recipe | undefined {
  return recipes.find((item) => item.id === id);
}

export function recipesForProject(projectId: string): Recipe[] {
  return recipes.filter((item) => item.projectId === projectId);
}
