import { createModels, createProvider, type Model } from "@earendil-works/pi-ai";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { openAIResponsesApi } from "@earendil-works/pi-ai/api/openai-responses.lazy";

import type { PiRunRequest } from "./protocol.js";

export function configureProjectPayload(request: PiRunRequest, payload: unknown): unknown {
  if (typeof payload !== "object" || payload === null || Array.isArray(payload)) {
    throw new Error("OpenAI-compatible request payload is invalid");
  }
  const configured: Record<string, unknown> = { ...payload };
  const endpoint = request.openaiCompatible?.baseUrl;
  const isResponsesPayload = "input" in configured;
  if (endpoint && new URL(endpoint).hostname.toLowerCase() === "api.deepseek.com") {
    if (isResponsesPayload) {
      // DeepSeek's Responses API uses the OpenAI reasoning object. The model
      // otherwise defaults to thinking, which is incompatible with forced tools.
      configured.reasoning = { effort: "none" };
    } else {
      // Retain the documented Chat Completions controls for payload-level
      // compatibility, even though official DeepSeek traffic uses Responses.
      configured.thinking = { type: "disabled" };
      configured.reasoning_effort = "none";
    }
  }
  if (request.capabilitySet === "encoder_optimization_proposal_v1") {
    configured.tool_choice = isResponsesPayload
      ? "required"
      : {
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
  const isDeepSeek = url.hostname.toLowerCase() === "api.deepseek.com";
  const baseModel = {
    id: request.model,
    name: request.model,
    provider: request.provider,
    baseUrl: url.toString().replace(/\/$/, ""),
    input: ["text"] as ("text" | "image")[],
    // The host reports unknown cost unless it has an independent pinned rate.
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 131072,
    maxTokens: configuration.maximumOutputTokens,
  };
  const models = createModels();
  const auth = {
    apiKey: {
      name: request.provider,
      resolve: async () => ({ auth: key ? { apiKey: key } : {} }),
    },
  };

  if (isDeepSeek) {
    // DeepSeek's current agent contract is its Responses API. Chat Completions
    // accepted but ignored a forced proposal tool choice for deepseek-flash.
    const model: Model<"openai-responses"> = {
      ...baseModel,
      api: "openai-responses",
      reasoning: true,
      thinkingLevelMap: { off: "none" },
      compat: {
        supportsDeveloperRole: false,
        supportsStrictMode: false,
      },
    };
    models.setProvider(
      createProvider({
        id: request.provider,
        name: request.provider,
        baseUrl: model.baseUrl,
        auth,
        models: [model],
        api: openAIResponsesApi(),
      }),
    );
    return { models, model };
  }

  const model: Model<"openai-completions"> = {
    ...baseModel,
    api: "openai-completions",
    reasoning: false,
    compat: {
      maxTokensField: "max_tokens",
      supportsStore: false,
    },
  };
  models.setProvider(
    createProvider({
      id: request.provider,
      name: request.provider,
      baseUrl: model.baseUrl,
      auth,
      models: [model],
      api: openAICompletionsApi(),
    }),
  );
  return { models, model };
}
