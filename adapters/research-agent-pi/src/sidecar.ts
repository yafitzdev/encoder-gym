import { PiResearchAgent } from "./agent.js";
import {
  PROTOCOL_VERSION,
  type InputMessage,
  type OutputMessage,
  type ToolExecutionRequest,
  type ToolExecutionResult,
  type ToolExecutor,
} from "./protocol.js";

export const PI_PACKAGE_VERSION = "0.84.4";

type OutputSink = (message: OutputMessage) => Promise<void> | void;

interface PendingToolCall {
  resolve: (result: ToolExecutionResult) => void;
  reject: (error: Error) => void;
  removeAbortListener: () => void;
}

class HostToolBridge implements ToolExecutor {
  readonly #output: OutputSink;
  readonly #pending = new Map<string, PendingToolCall>();

  constructor(output: OutputSink) {
    this.#output = output;
  }

  async execute(request: ToolExecutionRequest, signal?: AbortSignal): Promise<ToolExecutionResult> {
    if (this.#pending.has(request.callId)) {
      throw new Error(`duplicate Pi tool call ${request.callId}`);
    }
    return new Promise<ToolExecutionResult>((resolve, reject) => {
      const abort = () => this.reject(request.callId, "tool call aborted");
      signal?.addEventListener("abort", abort, { once: true });
      this.#pending.set(request.callId, {
        resolve,
        reject,
        removeAbortListener: () => signal?.removeEventListener("abort", abort),
      });
      Promise.resolve(this.#output({ type: "tool_request", ...request })).catch((error) => {
        this.reject(request.callId, error instanceof Error ? error.message : String(error));
      });
    });
  }

  resolve(callId: string, result: ToolExecutionResult): void {
    const pending = this.#pending.get(callId);
    if (!pending) throw new Error(`unknown or completed tool call ${callId}`);
    this.#pending.delete(callId);
    pending.removeAbortListener();
    pending.resolve(result);
  }

  reject(callId: string, message: string): void {
    const pending = this.#pending.get(callId);
    if (!pending) throw new Error(`unknown or completed tool call ${callId}`);
    this.#pending.delete(callId);
    pending.removeAbortListener();
    pending.reject(new Error(message));
  }

  rejectAll(message: string): void {
    for (const callId of [...this.#pending.keys()]) this.reject(callId, message);
  }
}

export class ProtocolSession {
  readonly #output: OutputSink;
  readonly #bridge: HostToolBridge;
  readonly #agent: PiResearchAgent;
  #activeRunId: string | undefined;
  #active: Promise<void> | undefined;

  constructor(output: OutputSink) {
    this.#output = output;
    this.#bridge = new HostToolBridge(output);
    this.#agent = new PiResearchAgent(this.#bridge, (event) =>
      this.#output({ type: "event", event }),
    );
  }

  ready(): Promise<void> | void {
    return this.#output({
      type: "ready",
      protocolVersion: PROTOCOL_VERSION,
      piPackageVersion: PI_PACKAGE_VERSION,
    });
  }

  handle(message: InputMessage): void {
    switch (message.type) {
      case "start":
        this.start(message.request);
        break;
      case "tool_result":
        this.#bridge.resolve(message.callId, message.result);
        break;
      case "tool_error":
        this.#bridge.reject(message.callId, message.message);
        break;
      case "cancel":
        if (!this.#agent.cancel(message.runId)) {
          throw new Error(`research run ${message.runId} is not active`);
        }
        break;
    }
  }

  async waitForIdle(): Promise<void> {
    await this.#active;
  }

  abort(message: string): void {
    if (this.#activeRunId) this.#agent.cancel(this.#activeRunId);
    this.#bridge.rejectAll(message);
  }

  private start(request: Extract<InputMessage, { type: "start" }>["request"]): void {
    if (this.#active) throw new Error(`research run ${this.#activeRunId} is already active`);
    this.#activeRunId = request.runId;
    this.#active = this.#agent
      .run(request)
      .then(() => this.#output({ type: "completed", runId: request.runId }))
      .catch((error) =>
        this.#output({
          type: "failed",
          runId: request.runId,
          message: error instanceof Error ? error.message : String(error),
        }),
      )
      .then(() => undefined)
      .finally(() => {
        this.#active = undefined;
        this.#activeRunId = undefined;
      });
  }
}
