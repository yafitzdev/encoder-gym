# Encoder Gym demo

This is the complete offline first run used by the repository README. It uses
small deterministic source cohorts and the hashing-linear trainer, so it needs
no API key, model download, Python environment, or GPU.

From the repository root:

```powershell
$env:SYNTH_DATABASE_URL = "sqlite://encoder-gym-demo.db?mode=rwc"

cargo run -p synthetic-data-cli -- project bootstrap-preview demo/project-bootstrap.toml
cargo run -p synthetic-data-cli -- project bootstrap demo/project-bootstrap.toml
```

The manifest references `development.jsonl` and `sealed.csv` relative to this
directory. See [`../docs/QUICKSTART.md`](../docs/QUICKSTART.md) for the complete
workflow.
