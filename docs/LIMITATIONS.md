# Limitations

Encoder Gym is under active development. The boundaries below describe the
current repository rather than a future product roadmap.

| Boundary | Current behavior |
|---|---|
| Deployment | Local, single-user operation; no hosted control plane, authentication, multi-user collaboration, or distributed workers |
| Standard task | The complete generic workflow targets text classification; other encoder tasks require compiled adapters |
| Desktop coverage | The desktop manages projects and evidence but does not expose every CLI workflow or production adapter |
| Trainer support | Deterministic hashing-linear training and the documented BERT-family CPU bundle contract |
| Model formats | Arbitrary model code and pickle-based weights are not accepted by the generic transformer path |
| Contamination checks | Exact source, exact text, normalized text, and declared group checks; no semantic or embedding deduplication |
| External provenance | The platform cannot prove that benchmark content was absent from a pretrained base model or from training performed elsewhere |
| Optimization | Runs are finite, persisted, and bounded; there is no open-ended autonomous optimization loop |
| Compatibility | The workspace is version `0.1.0`; no long-term public compatibility commitment has been declared |
| Licensing | No project license has been published, so the repository is not presently offered under an open-source license |

Additional task and authority boundaries are defined in
[`PLATFORM.md`](PLATFORM.md). Operational failures and recovery behavior are
documented in [`OPERATIONS.md`](OPERATIONS.md) and [`RECOVERY.md`](RECOVERY.md).
