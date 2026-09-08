import type { CandidateRow } from "./catalog.js";

export type Page = "models" | "datasets" | "runs" | "benchmarks" | "project" | "optimization" | "candidate" | "baseline" | "run" | "compare";
export interface Location { page: Page; id?: string; tab?: string; runId?: string; candidateIds?: string[] }
export interface Actions {
  navigate(location: Location): void;
  backTo(page: Page): void;
  render(): void;
  copy(text: string): void;
  help(metric?: string): void;
  notify(text: string): void;
  refresh(): void;
  connect(): void;
  compare(rows: CandidateRow[]): void;
  prepareOptimization(): void;
}

export interface ProjectActions {
  create(): void;
  openManaged(): void;
  importDataset(): void;
  verify(): void;
  addFolder(): void;
  openExample(): void;
  select(id: string): void;
  rename(): void;
  relocate(): void;
  forget(): void;
}
