# ADR 001 — Rust-native BERT training backend

## Status

Accepted for Slice 3.1.

## Context

The platform has project-owned `TrainingBackend`, `TrainingSession`,
`Predictor`, and checkpoint contracts. The next backend must fine-tune a real
pretrained encoder on CPU without Python or provider-specific types crossing
those contracts. It must load a local, inspectable model bundle and keep
ordinary tests offline.

## Decision

Use Hugging Face Candle 0.11 for tensor operations, automatic differentiation,
and the BERT encoder implementation. Use Hugging Face Tokenizers 0.23 to read a
local `tokenizer.json`. Implement the sequence-classification head, batching,
optimizer state, checkpoint container, bundle validation, and port adapters in
the dedicated `training-transformer` crate.

The first supported bundle format is deliberately exact:

- architecture: BERT encoder (`model_type = "bert"`)
- weights: one `model.safetensors` file with canonical `bert.*` tensor names
- tokenizer: one self-contained `tokenizer.json`
- configuration: one `config.json`
- numeric execution: F32 on CPU
- classifier: a platform-owned linear head over the first token representation

The adapter may ignore extra pretrained tensors such as a pooler or language-
modeling head, but every tensor required by the encoder must exist with the
expected shape. It does not describe untested BERT variants as compatible.

Registered bundles are immutable by identity, not by trusting a path. Their
identity includes canonical metadata plus SHA-256 checksums of configuration,
tokenizer, and weight files. Each use verifies those facts again.

## Contract impact

Core contracts will gain only backend-neutral concepts needed by more than one
implementation: bounded training progress, registered base-model identity,
opaque normalized backend-configuration identity, checkpoint size, and an
optional parent checkpoint for explicit continuation. A backend-neutral
example-source port provides member IDs and bounded training/validation reads;
the CLI implements it with a paged SQLite-to-temporary-file spool. Candle tensors,
tokenizer encodings, optimizer objects, and BERT configuration types remain in
the adapter crate.

Transformer checkpoints use one immutable platform container holding a JSON
manifest, tokenizer/configuration data, trainable safetensors, and optimizer
state. A single container preserves the existing atomic `CheckpointSink`
boundary while remaining independently checksum-verifiable.

## Consequences and limitations

- CPU execution is always available and is the only execution mode promised by
  this slice.
- Initial training can be slower and more memory-intensive than the linear
  baseline; batches and tokenization are bounded.
- The adapter supports full fine-tuning and an explicit frozen-encoder mode.
- Model download, GPU execution, mixed precision, ONNX, quantization, serving,
  and additional transformer families remain outside this decision.
- The hashing/linear backend remains the default deterministic development and
  smoke-test backend.
