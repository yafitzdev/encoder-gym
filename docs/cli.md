# CLI reference guide

The terminal is the only new presentation surface for Slices 2–6. Use
`synth <COMMAND> --help` for the generated flag reference. Global
`--output human|json` behavior, pagination, exit status, and file-output rules
are documented in `operations.md`.

Transformer-specific command groups are:

- `synth encoder register|list|show|verify`
- `synth training run|list|status|cancel`
- `synth training checkpoints|checkpoint|predict|continue`
- `synth evaluation run`
- `synth provenance checkpoint`
- `synth doctor`

`training run --backend bert-cpu` requires either `--encoder-id` or
`training.base_model_id` in strict TOML. Explicit flags override TOML. Durable
progress is written to stderr; JSON results remain a single value on stdout.
There is no interactive TUI, graphical UI, or new HTTP endpoint for this work.

Error-analysis commands are:

- `synth analysis create <RUN_ID>` with support, ranking, confidence,
  intersection, evidence-limit, seed, and optional `--comparison-id` flags;
- `synth analysis list|show` for immutable reports;
- `synth analysis findings|finding` for normalized, paged findings;
- `synth analysis evidence` for resolved prediction evidence and JSONL/CSV
  export through `--format ... --file ...`;
- `synth analysis high-confidence-errors|weak-cells|comparison-group` for
  focused diagnostic views;
- `synth analysis review|reviews` for append-only human decisions; and
- `synth analysis export` for the immutable report summary.

Finding keys are canonical JSON strings. Quote them when invoking a shell.
See `error-analysis.md` for metric semantics and complete examples.

Optimization and campaign commands are:

- `synth optimize preview|propose|scenarios` for bounded decision artifacts;
- `synth optimize scenario-show|scenario-materialize` for policy alternatives;
- `synth optimize list|show|recommendations|explain` for stable inspection;
- `synth optimize review|apply|rebase` for the human approval boundary;
- `synth optimize training-space|training-candidates` for typed finite
  experiment candidates that never start training;
- `synth optimize export|export-summary|export-recommendations` for JSON,
  JSONL, and CSV artifacts; and
- `synth campaign create|list|show|link|assess` for a validated append-only
  experiment ledger.

Use a strict TOML/JSON protocol with decision-grade `preview` and `propose`.
`apply` requires an approval ID and only creates a plan. See
`optimization.md` for semantics and the complete operator workflow.

See `transformer-training.md` for the complete local BERT workflow and
`examples/transformer-project.toml` for non-secret settings.
