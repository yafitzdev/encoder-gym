import type { NativeProgress, RunActivity } from "./managed-control.js";
import { spawn } from "node:child_process";

const phases = new Set([
  "checking_model", "checking_dataset", "checking_evaluation", "checking_runtime",
  "loading_training_rows", "writing_training_rows", "checking_materialized_project",
  "loading_evaluation_protocol", "creating_candidate", "creating_experiment",
  "registering_candidate", "optimization_complete",
  "checking_files", "checking_training_data", "loading_model", "preparing_batches",
  "training", "saving_checkpoint", "evaluating_retrieval", "evaluating_agent",
]);

/** Treat subprocess output as untrusted; only this closed, numerical schema reaches the UI. */
export function parseProgress(line: string): NativeProgress | undefined {
  const prefix = "ENCODER_GYM_PROGRESS ";
  if (!line.startsWith(prefix) || line.length > 1024) return;
  try {
    const value = JSON.parse(line.slice(prefix.length));
    if (!value || !phases.has(value.phase) || Object.keys(value).some(key => !["phase", "completed", "total"].includes(key))) return;
    if (value.completed === undefined && value.total === undefined) return { phase: value.phase };
    if (!Number.isSafeInteger(value.completed) || !Number.isSafeInteger(value.total) || value.completed < 0 || value.total < 1 || value.total > 1_000_000_000 || value.completed > value.total) return;
    return { phase: value.phase, completed: value.completed, total: value.total };
  } catch { return; }
}

export class ProgressLines {
  private pending = "";
  private overflow = false;
  constructor(private receive: (progress: NativeProgress) => void, private diagnostic: (line: string) => void = () => {}) {}
  push(chunk: string): void {
    for (const piece of chunk.split(/(?<=\n)/)) {
      if (!this.overflow) this.pending += piece;
      if (this.pending.length > 1024) { this.pending = ""; this.overflow = true; }
      if (piece.endsWith("\n")) {
        const progress = this.overflow ? undefined : parseProgress(this.pending.trimEnd());
        if (progress) this.receive(progress);
        else this.diagnostic(this.overflow ? "[Long diagnostic omitted]" : this.pending);
        this.pending = ""; this.overflow = false;
      }
    }
  }
  finish(): void { if (this.pending || this.overflow) this.push("\n"); }
}

/** Drain progress continuously; a long run must not exhaust execFile's stderr buffer. */
export function executeObservedCommand(executable: string, args: string[], environment: Readonly<Record<string, string>> | undefined, receive: (progress: NativeProgress) => void, cancellationSignal?: AbortSignal): Promise<string> {
  return new Promise((resolve, reject) => {
    if (cancellationSignal?.aborted) { reject(new Error("Run cancelled.")); return; }
    const child = spawn(executable, args, { windowsHide: true, shell: false, stdio: ["ignore", "pipe", "pipe"], ...(environment ? { env: { ...process.env, ...environment } } : {}) });
    let stdout = "", diagnostic = "", tooLarge = false, cancelled = false;
    const stop = (): void => {
      cancelled = true;
      if (process.platform === "win32" && child.pid) {
        const killer = spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"], { windowsHide: true, shell: false, stdio: "ignore" });
        const fallback = setTimeout(() => { if (child.exitCode === null) child.kill(); }, 500);
        killer.once("close", () => { clearTimeout(fallback); if (child.exitCode === null) child.kill(); });
        killer.once("error", () => { clearTimeout(fallback); if (child.exitCode === null) child.kill(); });
      } else child.kill("SIGTERM");
    };
    cancellationSignal?.addEventListener("abort", stop, { once: true });
    const lines = new ProgressLines(receive, line => { diagnostic = (diagnostic + line).slice(-8000); });
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => lines.push(chunk));
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      if (tooLarge) return;
      stdout += chunk;
      if (stdout.length > 16 * 1024 * 1024) { tooLarge = true; stdout = ""; child.kill(); }
    });
    child.on("error", error => reject(error));
    child.on("close", (code, processSignal) => {
      cancellationSignal?.removeEventListener("abort", stop);
      lines.finish();
      if (cancelled) reject(new Error("Run cancelled."));
      else if (tooLarge) reject(new Error("The backend response exceeded its size limit."));
      else if (code !== 0) reject(new Error(diagnostic.trim() || `Run worker stopped (${processSignal ?? code}).`));
      else resolve(stdout);
    });
  });
}

export function recordProgress(activity: RunActivity, progress: NativeProgress, at = new Date().toISOString()): void {
  if (activity.phase !== progress.phase || progress.total !== activity.total || (progress.completed !== undefined && activity.completed !== undefined && progress.completed < activity.completed)) {
    activity.events.push({ phase: progress.phase, at });
    activity.events = activity.events.slice(-20);
  }
  Object.assign(activity, { ...progress, completed: progress.completed, total: progress.total, updatedAt: at });
}
