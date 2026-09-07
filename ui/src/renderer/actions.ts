import type { CandidateRow } from "./catalog.js";

export type Page = "models" | "runs" | "benchmarks" | "project" | "candidate" | "baseline" | "run" | "compare";
export interface Location { page: Page; id?: string; tab?: string; runId?: string; candidateIds?: string[] }
export interface Actions {
  navigate(location: Location): void;
  render(): void;
  copy(text: string): void;
  help(metric?: string): void;
  notify(text: string): void;
  refresh(): void;
  connect(): void;
  compare(rows: CandidateRow[]): void;
}

export interface ProjectActions {
  addFolder(): void;
  openExample(): void;
  select(id: string): void;
  rename(): void;
  relocate(): void;
  forget(): void;
}
