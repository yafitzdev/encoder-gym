# Nomos managed-workspace onboarding

Status: baseline copy and dataset backfill verified; desktop connection and
New/Open/import presentation are still pending. This is not completion of the
managed-workspace goal.

## Identity

- Managed root: `C:\Users\yanfi\EncoderGym\Projects\Nomos`
- Project ID: `0dd64b24-47d2-4cfb-9523-6c0b65dc4a46`
- Created: `2026-09-08T00:04:25.780824600Z`
- Source repository: `C:\Users\yanfi\PycharmProjects\fitz-tool`
- Source revision: `14e0a1667431982ee00ee07108e7d82351fa28eb`
- Baseline source: `artifacts/nomos_bge_contrast_replay_ablation`
- Format: sentence-transformers / BERT; 11 files, 134,211,908 bytes, including
  the original pooling configuration, Normalize module and training manifest.
- Baseline inventory:
  `sha256:9885102b8b6e2fc530dcea5614dd7c3293eacb175f62774382a51f486691f630`
- Training manifest:
  `sha256:95e4ee636e9dcf8f97189893a064542ceaf70473cd9db498a8df730553bfcade`

This is the retained trainable local checkpoint, not a fresh upstream BGE
download and not an invented record of the earlier ONNX optimization runs.
There are no candidates, run records, or evaluation results in the new project.

## Imported final-stage inputs

| Original `data/generated/` file | Rows | Bytes | SHA-256 |
| --- | ---: | ---: | --- |
| `nomos_agentic_transitions_v3_3400.jsonl` | 3,400 | 45,511,441 | `3a90839b1f2be6288ad7ba9f73ecd06598e3df0ce33c237876608a0908a5ea2e` |
| `nomos_agentic_contrasts_v4_3400.jsonl` | 3,400 | 44,586,360 | `be983b9183e1d70f0c1eff067b98ef732c8b9ceb3407cb4b3832e83005a7dd7f` |

All 6,800 rows declare `evaluation_partition: train` and `accepted: true`.
They remain byte-for-byte native JSONL under content-addressed
`datasets/imports/<hash>/data.jsonl` paths. Each database record links the exact
baseline, training manifest, declared input name, row count and copied content.
These are imported source datasets, not newly approved training snapshots.

The historical manifest identifies the final training-stage inputs. It does
not contain hashes proving historical bytes, nor reconstruct all ancestor
pretraining. The backfill records the current available referenced files and
does not import sealed evidence or unrelated data directories.

## Recheck

The backend stage passed `cargo fmt-check`, `cargo check-all`, `cargo lint`,
and the full `cargo test-all` suite (`RUST_TEST_THREADS=1`). Existing UI checks
also passed (23 tests); this does not verify the still-pending onboarding UI.
Independent source hashing matched all 11 checkpoint files and both dataset
copies after import. The source repository retained the same seven pre-existing
modified/untracked paths; no source files were changed by onboarding.

```powershell
synth workspace verify C:\Users\yanfi\EncoderGym\Projects\Nomos
```

The workspace no longer needs the original checkpoint or input paths to open
or verify. Keep the whole managed folder together when moving it. The existing
app-library entry still points to `fitz-tool` until the desktop integration
stage replaces that connection through the registry; do not create duplicate
Nomos entries or delete the source repository.
