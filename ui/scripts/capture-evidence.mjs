import { writeFileSync } from "node:fs";
import { readWorkspace } from "../dist/evidence/read-workspace.js";

const [workspace, output] = process.argv.slice(2);
if (!workspace || !output) throw new Error("Usage: node scripts/capture-evidence.mjs <workspace> <snapshot.json>");
const snapshot = readWorkspace(workspace);
snapshot.source = "recorded";
writeFileSync(output, JSON.stringify(snapshot, null, 2) + "\n");
console.log(`Captured ${snapshot.runs.length} runs, with development comparisons only.`);
