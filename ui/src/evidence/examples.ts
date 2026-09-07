import recorded from "./nomos-snapshot.json";
import type { WorkspaceSnapshot } from "../workspace.js";

/** Explicitly opened archival example, never the app's default project. */
export const example = { key: "nomos-recorded", name: "Nomos (recorded example)" };
export function readExample(key: string): WorkspaceSnapshot {
  if (key !== example.key) throw new Error("This recorded example is not available in this version.");
  return structuredClone(recorded) as WorkspaceSnapshot;
}
