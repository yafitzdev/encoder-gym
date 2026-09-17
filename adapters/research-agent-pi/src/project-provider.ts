import { createModels, createProvider, type Model } from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";

import type { PiRunRequest } from "./protocol.js";

export function configureProjectPayload(request: PiRunRequest, payload: unknown): unknown {
  if (typeof payload !== "object" || payload === null || Array.isArray(payload)) {
    throw new Error("OpenAI-compatible request payload is invalid");
  }
  const configured: Record<string, unknown> = { ...payload };
  const endpoint = request.openaiCompatible?.baseUrl;
  if (endpoint && new URL(endpoint).hostname.toLowerCase() === "api.deepseek.com") {
    // DeepSeek defaults to thinking mode. Send both documented controls because
    // reasoning-only completions cannot satisfy a required proposal tool call.
    configured.thinking = { type: "disabled" };
    configured.reasoning_effort = "none";
  }
  if (request.capabilitySet === "encoder_optimization_proposal_v1") {
    configured.tool_choice = {
      type: "function",
      function: { name: "propose_dataset_edits" },
    };
  }
  return configured;
}

/** Use the selected project model verbatim, never a built-in catalog substitute. */
export function projectProvider(request: PiRunRequest) {
  const configuration = request.openaiCompatible;
  if (!configuration) throw new Error("Project provider configuration is missing");
  const url = new URL(configuration.baseUrl);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    throw new Error("Project provider URL must not contain credentials, a query or a fragment");
  }
  if (
    !Number.isSafeInteger(configuration.maximumOutputTokens) ||
    configuration.maximumOutputTokens < 1 ||
    configuration.maximumOutputTokens > 65536
  ) {
    throw new Error("Project provider output-token ceiling must be between 1 and 65536");
  }
  const key = request.apiKeyEnv ? process.env[request.apiKeyEnv] : undefined;
  if (request.apiKeyEnv && !key?.trim()) {
    throw new Error("The selected project connection has no available API key");
  }
  // DeepSeek enables thinking by default at the HTTP boundary. Pi's
  // thinkingLevel="off" needs this compatibility declaration to serialize
  // `thinking: { type: "disabled" }`; otherwise a forced tool choice is
  // rejected with HTTP 400 even though the Agent requested non-thinking mode.
  const isDeepSeek = url.hostname.toLowerCase() === "api.deepseek.com";
  const model: Model<"openai-completions"> = {
    id: request.model,
    name: request.model,
    api: "openai-completions",
    provider: request.provider,
    baseUrl: url.toString().replace(/\/$/, ""),
    reasoning: isDeepSeek,
    input: ["text"],
    // The host reports unknown cost unless it has an independent pinned rate.
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 131072,
    maxTokens: configuration.maximumOutputTokens,
    compat: {
      maxTokensField: "max_tokens",
      supportsStore: false,
      ...(isDeepSeek ? { thinkingFormat: "deepseek" as const } : {}),
    },
  };
  const models = createModels();
  models.setProvider(
    createProvider({
      id: request.provider,
      name: request.provider,
      baseUrl: model.baseUrl,
      auth: {
        apiKey: {
          name: request.provider,
          resolve: async () => ({ auth: key ? { apiKey: key } : {} }),
        },
      },
      models: [model],
      api: openAICompletionsApi(),
    }),
  );
  return { models, model };
}
