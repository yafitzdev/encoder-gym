import type { AgentTool } from "@earendil-works/pi-agent-core";
import type { Tool } from "@earendil-works/pi-ai";
import { Type } from "typebox";

import type { EncoderOptimizationToolName, ToolExecutor } from "./protocol.js";

const identity = Type.String({ minLength: 1, maxLength: 128 });
const explanation = Type.String({ minLength: 1, maxLength: 2000 });

interface ProposalRequirements {
  maximumRowChanges: number;
  trainingRowIds: string[];
  trainingRowIdsComplete: boolean;
  developmentEvidenceIds: string[];
  developmentEvidenceIdsComplete: boolean;
}

function proposalRequirements(initialPrompt?: string): ProposalRequirements | undefined {
  if (!initialPrompt) return undefined;
  let parsed: unknown;
  try {
    parsed = JSON.parse(initialPrompt);
  } catch {
    return undefined; // Historical callers used a plain-text prompt.
  }
  if (!parsed || typeof parsed !== "object" || !("proposalRequirements" in parsed))
    return undefined;
  const value = parsed.proposalRequirements;
  if (!value || typeof value !== "object") throw new Error("Invalid host proposal requirements");
  const fields = value as Record<string, unknown>;
  const maximum = fields.maximumRowChanges;
  if (
    typeof maximum !== "number" ||
    !Number.isSafeInteger(maximum) ||
    maximum < 0 ||
    maximum > 5000 ||
    fields.maximumSummaryCharacters !== 400 ||
    fields.maximumEvidenceIdsPerEdit !== 20
  ) {
    throw new Error("Invalid host proposal limits");
  }
  function references(name: string): { ids: string[]; complete: boolean } {
    const ids = fields[name];
    const complete = fields[`${name}Complete`];
    if (
      !Array.isArray(ids) ||
      ids.length > 20 ||
      ids.some((id) => typeof id !== "string" || id.length === 0 || Buffer.byteLength(id) > 128) ||
      new Set(ids).size !== ids.length ||
      typeof complete !== "boolean"
    ) {
      throw new Error("Invalid host proposal references");
    }
    return { ids, complete };
  }
  const rows = references("trainingRowIds");
  const evidence = references("developmentEvidenceIds");
  return {
    maximumRowChanges: maximum,
    trainingRowIds: rows.ids,
    trainingRowIdsComplete: rows.complete,
    developmentEvidenceIds: evidence.ids,
    developmentEvidenceIdsComplete: evidence.complete,
  };
}

function proposalSchema(requirements?: ProposalRequirements) {
  const maximum = requirements?.maximumRowChanges ?? 5000;
  const reference = (ids: string[] | undefined, complete: boolean | undefined) =>
    ids?.length && complete ? Type.String({ minLength: 1, maxLength: 128, enum: ids }) : identity;
  const row = reference(requirements?.trainingRowIds, requirements?.trainingRowIdsComplete);
  const evidence = Type.Array(
    reference(requirements?.developmentEvidenceIds, requirements?.developmentEvidenceIdsComplete),
    {
      minItems: 1,
      maxItems: 20,
      uniqueItems: true,
      description:
        "Only exact outer evidence item.id values returned by the active inspection protocol. Never nested report IDs, source-row IDs, training-row IDs or fingerprints.",
    },
  );
  return Type.Object(
    {
      summary: Type.String({ minLength: 1, maxLength: 400 }),
      stop: Type.Boolean(),
      removals: Type.Array(
        Type.Object({ rowId: row, reason: explanation, evidenceIds: evidence }),
        { maxItems: maximum },
      ),
      additions: Type.Array(
        Type.Object({
          templateRowId: row,
          instruction: explanation,
          count: Type.Integer({ minimum: 1, maximum: Math.max(1, maximum) }),
          evidenceIds: evidence,
        }),
        { maxItems: maximum },
      ),
    },
    {
      description: `Total row changes = removals.length + sum(additions[*].count), at most ${maximum}. Stop requires both edit arrays empty.`,
    },
  );
}

const schemas = {
  inspect_development_failures: Type.Object({
    offset: Type.Integer({ minimum: 0 }),
    limit: Type.Integer({ minimum: 1, maximum: 20 }),
  }),
  inspect_training_rows: Type.Object({
    offset: Type.Integer({ minimum: 0 }),
    limit: Type.Integer({ minimum: 1, maximum: 20 }),
    query: Type.Optional(Type.String({ maxLength: 200 })),
  }),
  inspect_dataset_landscape: Type.Object({
    offset: Type.Integer({ minimum: 0 }),
    limit: Type.Integer({ minimum: 1, maximum: 20 }),
  }),
  inspect_dataset_clusters: Type.Object({
    clusterIds: Type.Array(identity, { minItems: 1, maxItems: 4, uniqueItems: true }),
    examplesPerCluster: Type.Integer({ minimum: 1, maximum: 4 }),
  }),
  propose_dataset_edits: proposalSchema(),
};

const descriptions: Record<EncoderOptimizationToolName, string> = {
  inspect_development_failures:
    "Inspect persisted development failures for the pinned benchmark. Pages are byte-bounded and may return fewer items than limit; nextOffset identifies the next complete item. No sealed evidence is accessible.",
  inspect_training_rows:
    "Inspect a bounded page of the exact starting training dataset; optionally filter its task-visible text. Pages may return fewer items than limit to bound context; only returned items have been inspected, and nextOffset identifies the next complete item.",
  inspect_dataset_landscape:
    "Inspect ranked aggregate clusters built by scanning the complete pinned training dataset and joining development metrics. Compare evaluation support, errors and regression with training coverage; coverage is descriptive, not causal. No sealed evidence is accessible.",
  inspect_dataset_clusters:
    "Inspect deterministic representative training rows for 1–4 exact cluster IDs already returned by inspect_dataset_landscape. This reveals qualitative row content without reading the full dataset.",
  propose_dataset_edits:
    "Submit evidence-linked removals and targeted generation instructions, or stop without changes. Keep summary within 400 characters and state the intended bounded coverage shift. Use only exact outer IDs returned by the active inspection protocol. Total removals plus requested additions must fit the remaining row-change budget. The host validates every proposal; this tool cannot train, approve or change a benchmark.",
};

export function encoderOptimizationModelTools(tools: Tool[], initialPrompt?: string): Tool[] {
  const proposal = proposalSchema(proposalRequirements(initialPrompt));
  return tools.map((tool) => {
    if (!Object.hasOwn(schemas, tool.name)) {
      throw new Error("Unexpected encoder optimization tool");
    }
    return {
      ...tool,
      parameters:
        tool.name === "propose_dataset_edits"
          ? proposal
          : schemas[tool.name as EncoderOptimizationToolName],
    };
  });
}

export function createEncoderOptimizationTools(
  runId: string,
  executor: ToolExecutor,
  proposalOnly = false,
  analysisProtocol: 1 | 2 = 1,
): AgentTool[] {
  const names: EncoderOptimizationToolName[] = proposalOnly
    ? ["propose_dataset_edits"]
    : analysisProtocol === 2
      ? ["inspect_dataset_landscape", "inspect_dataset_clusters"]
      : ["inspect_development_failures", "inspect_training_rows", "propose_dataset_edits"];
  return names.map((name) => ({
    name,
    label: name,
    description: descriptions[name],
    // Pi validates/coerces before calling execute. Keep its transport schema
    // permissive so every attempt reaches the authoritative Rust validator and
    // append-only trace unchanged. The model receives the strict schema above.
    parameters: Type.Unknown(),
    executionMode: "sequential",
    async execute(callId, parameters, signal) {
      const result = await executor.execute({ runId, callId, name, arguments: parameters }, signal);
      return {
        content: [{ type: "text", text: JSON.stringify(result.content) }],
        details: result.details ?? {},
        terminate: result.terminate ?? false,
      };
    },
  }));
}
