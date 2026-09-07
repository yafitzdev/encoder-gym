import type { WorkspaceSnapshot } from "./workspace.js";

/** App-owned organization, deliberately separate from immutable experiment IDs. */
export interface ProjectEntry {
  id: string;
  name: string;
  createdAt: string;
  source: { kind: "folder"; path: string } | { kind: "example"; key: string };
}
export interface ProjectCollection {
  version: 1;
  selectedId: string | null;
  projects: ProjectEntry[];
}
export type ProjectContent =
  | { state: "ready"; workspace: WorkspaceSnapshot }
  | { state: "empty"; databases: string[] }
  | { state: "error"; message: string };
export interface OpenedProject { project: ProjectEntry; content: ProjectContent }

/** A slow response from a previous folder must never replace the selected project. */
export class ProjectSelection {
  private revision = 0;
  selectedId: string | null = null;
  invalidate(id: string | null): number { this.selectedId = id; return ++this.revision; }
  async open(id: string, read: (id: string) => Promise<OpenedProject>): Promise<OpenedProject | undefined> {
    const revision = this.invalidate(id);
    try {
      const result = await read(id);
      return this.revision === revision && result.project.id === id ? result : undefined;
    } catch (error) {
      if (this.revision === revision) throw error;
      return undefined;
    }
  }
}
