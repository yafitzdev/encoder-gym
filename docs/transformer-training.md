# Local BERT training

The `bert-cpu` adapter fine-tunes a local BERT encoder for text classification
without Python, a network connection, or provider credentials at runtime. It is
an explicit alternative to the fast `hashing-linear` development backend.

## Exact bundle format

Put these three files in one local directory:

```text
my-bert/
  config.json
  tokenizer.json
  model.safetensors
```

The currently supported family is the original BERT encoder with
`model_type = "bert"`, F32 safetensors, and canonical `bert.*` weight names.
`tokenizer.json` must be self-contained, its vocabulary size must agree with
`config.json`, and its padding token must agree with `pad_token_id`. The adapter
validates every required encoder tensor and shape. Sharded weights, PyTorch
`.bin` files, DistilBERT, RoBERTa, decoder models, and provider-specific model
objects are not supported.

Registration reads safetensors metadata without loading every weight body and
streams SHA-256 checksums for all three files. It stores paths, validated
metadata, sizes, and checksums in SQLite; model weights remain in the local
bundle directory.

## Register and verify

```text
synth encoder register --name support-bert C:\models\my-bert
synth encoder list
synth encoder show <ENCODER_ID>
synth encoder verify <ENCODER_ID>
```

`register` returns the encoder ID and immutable `sha256:` identity. Save the ID
in a project configuration only after registration. `verify`, new training,
continuation setup, and `doctor` detect changed, missing, or corrupt bundle
files. Prediction uses the self-contained checkpoint and does not require the
original bundle.

## Train on an immutable snapshot

Use the checked-in [transformer configuration](../examples/transformer-project.toml)
as a template, replace `base_model_id`, then run:

```text
synth config validate examples/transformer-project.toml
synth training run <SNAPSHOT_ID> --config examples/transformer-project.toml
```

Or supply the normalized settings explicitly:

```text
synth training run <SNAPSHOT_ID> --backend bert-cpu \
  --encoder-id <ENCODER_ID> --epochs 3 --learning-rate 0.00002 \
  --maximum-sequence-length 128 --batch-size 8 --weight-decay 0.01 \
  --warmup-ratio 0.1 --gradient-clip-norm 1.0 \
  --encoder-mode fine-tune --checkpoint-every 1 --seed 42
```

Training pages snapshot rows into a temporary local spool and reads only the
active raw-text batch. Tokenization, truncation, padding, attention masks, and
validation inference are also bounded by `batch_size`. The deterministic batch
order is derived from snapshot member IDs, the seed, and the epoch.

Progress on stderr includes epoch, batch, processed examples, loss, and
learning rate. Press Ctrl+C once to request cancellation; the active batch
finishes, durable progress is retained, and completed checkpoints remain
usable. Use `synth training status <RUN_ID>` from another terminal to inspect
persisted progress.

`fine-tune` updates the encoder and classifier head. `frozen` updates only the
classifier head and is useful for faster CPU experiments. CPU/F32 is the only
promised device and dtype in this phase.

## Checkpoints, prediction, continuation, and evaluation

```text
synth training checkpoints <RUN_ID>
synth training checkpoint <CHECKPOINT_ID>
synth training predict <CHECKPOINT_ID> --text "Why was I charged twice?"
synth training continue <CHECKPOINT_ID> --epochs 2
synth evaluation run <CHECKPOINT_ID> --split test
synth provenance checkpoint <CHECKPOINT_ID>
synth doctor --config examples/transformer-project.toml
```

Each checkpoint is one atomically written, immutable `bert-classifier-v1`
container. It includes BERT and classifier weights, tokenizer/configuration,
label order, source identities, normalized training settings, epoch/step, and
AdamW state. SQLite stores its size and SHA-256 checksum, which are verified
before loading.

`training continue` never mutates or silently resumes the parent run. It starts
a new run with a parent-checkpoint edge and restores compatible model and
optimizer state. A different architecture, tokenizer, base-model fingerprint,
label order, snapshot label set, or material transformer configuration is
rejected.

The resulting predictor implements the same project-owned port as the linear
backend, so evaluation, comparison, analysis, and optimization commands need
no transformer-specific business logic.

## Explicit public-model smoke test

Ordinary checks use a generated tiny fixture and never download anything. To
exercise a separately acquired public BERT bundle, place its three supported
files in one directory, then opt in:

```text
$env:SYNTH_PUBLIC_BERT_BUNDLE = "C:\models\public-bert"
cargo test -p training-transformer public_bert_bundle_smoke -- --ignored --nocapture
```

The ignored test performs local validation, one frozen CPU training batch, a
checkpoint round trip, and prediction. It does not download the model.

On the development machine, the full tiny-fixture CLI workflow (registration,
training, continuation, prediction, evaluation, provenance, and doctor) took
about 3.1 seconds after compilation and peaked near 37 MiB aggregate working
set. These numbers only verify the plumbing: a real BERT model needs
substantially more time and memory, primarily according to model size, sequence
length, batch size, and whether the encoder is frozen.
