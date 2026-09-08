import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Safe-format custody fixture, deliberately not a trained/executable encoder. */
export function writeLocalModel(folder, weight = 1) {
  mkdirSync(folder, { recursive: true });
  writeFileSync(join(folder, "config.json"), JSON.stringify({ model_type: "bert", is_decoder: false }));
  writeFileSync(join(folder, "tokenizer.json"), JSON.stringify({ model: { type: "WordPiece", unk_token: "[UNK]", vocab: { "[UNK]": 0 } } }));
  const header = Buffer.from(JSON.stringify({ "fixture.weight": { dtype: "F32", shape: [1], data_offsets: [0, 4] } }));
  const length = Buffer.alloc(8); length.writeBigUInt64LE(BigInt(header.length));
  const data = Buffer.alloc(4); data.writeFloatLE(weight);
  writeFileSync(join(folder, "model.safetensors"), Buffer.concat([length, header, data]));
}
