export type RecipeStatus = "queued" | "running" | "completed" | "failed";

/** D) Evaluation suite belongs to the project, not to individual recipes. */
export interface EvaluationSuite {
  id: string;
  name: string;
  description: string;
}

export interface DatasetSnapshot {
  id: string;
  name: string;
  note: string;
}

/** C) A model/checkpoint snapshot that a recipe produced or depends on. */
export interface ModelSnapshot {
  id: string;
  name: string;
  checksum: string;
}

/** B) An instruction or configuration entry owned by a recipe. */
export interface Instruction {
  label: string;
  value: string;
}

export interface Recipe {
  id: string;
  projectId: string;
  name: string;
  description: string;
  status: RecipeStatus;
  /** A) dataset snapshots */
  datasetSnapshots: DatasetSnapshot[];
  /** B) instructions / configs */
  instructions: Instruction[];
  commands: string;
  /** C) model snapshots */
  modelSnapshots: ModelSnapshot[];
}

export interface Project {
  id: string;
  name: string;
  task: string;
  folder: string;
  description: string;
  createdAt: string;
  /** D) evaluation suite — shared across the project's recipes. */
  evaluationSuite: EvaluationSuite;
}

/** Only the active Nomos project is displayed in this skeleton. */
export const projects: Project[] = [
  {
    id: "nomos-repair",
    name: "Nomos repair",
    task: "Tool registry ranking",
    folder: "C:\\workspaces\\nomos-encoder-gym-experiment",
    description: "Recover bounded-change preflight routing on the retired cohort without regressing generic retrieval.",
    createdAt: "2026-09-02",
    evaluationSuite: {
      id: "nomos-suites",
      name: "Nomos development + retired suites",
      description: "generic_holdout and retired_post_scaling; sealed evidence stays separate.",
    },
  },
];

const nomosSnapshots: DatasetSnapshot[] = [
  { id: "ds-nomos-combined", name: "nomos-combined-v1", note: "Combined repair training snapshot" },
];

export const recipes: Recipe[] = [
  {
    id: "recipe-nomos-baseline",
    projectId: "nomos-repair",
    name: "nomos-baseline",
    description: "Frozen baseline that reproduces the development holdout before any repair candidate is trained.",
    status: "completed",
    datasetSnapshots: nomosSnapshots,
    instructions: [
      { label: "Adapter", value: "encoder-experiment-nomos" },
      { label: "Epochs", value: "20" },
      { label: "Split", value: "train_and_validation_v1" },
    ],
    commands: "cargo run -p synthetic-data-cli -- training run <SNAPSHOT_ID> --epochs 20\ncargo run -p synthetic-data-cli -- evaluation run <CHECKPOINT_ID> --split test",
    modelSnapshots: [
      { id: "ms-nomos-baseline", name: "nomos-baseline-checkpoint", checksum: "sha256:95943e3e…ff856c" },
    ],
  },
  {
    id: "recipe-nomos-interp",
    projectId: "nomos-repair",
    name: "nomos-interp-2p5",
    description: "Conservative 2.5% interpolation control that improves retired-suite routing while preserving generic behavior.",
    status: "completed",
    datasetSnapshots: nomosSnapshots,
    instructions: [
      { label: "Adapter", value: "encoder-experiment-nomos" },
      { label: "Weight", value: "2.5% interpolation control" },
      { label: "Split", value: "train_and_validation_v1" },
    ],
    commands: "cargo run -p synthetic-data-cli -- encoder optimize preview --manifest docs/examples/nomos-proven-optimize.toml",
    modelSnapshots: [
      { id: "ms-nomos-interp", name: "nomos-interp-2p5-checkpoint", checksum: "sha256:interp-2p5…a1c9d2" },
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
