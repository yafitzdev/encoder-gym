import type { CandidateRow } from "./catalog.js";

export type Page = "models" | "model" | "datasets" | "dataset" | "runs" | "benchmarks" | "activity" | "project" | "optimization" | "candidate" | "baseline" | "run" | "compare";
export interface Location { page: Page; id?: string; tab?: string; offset?: number; runId?: string; candidateIds?: string[]; metric?: string }
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
  readonly baselineBusy: boolean;
  promoteModel(runId: string, name: string): void;
  restoreBaseline(targetRevisionId: string, name: string): void;
}

export interface ProjectActions {
  create(): void;
  openManaged(): void;
  verify(): void;
  addFolder(): void;
  openExample(): void;
  select(id: string): void;
  rename(): void;
  relocate(): void;
  forget(): void;
}
