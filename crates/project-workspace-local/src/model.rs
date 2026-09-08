use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

use anyhow::{Result, ensure};
use project_workspace_core::{FileIdentity, LocalModel, inventory_fingerprint, validate_relative};
use serde_json::Value;

use crate::files::{canonical_plain, contained, hash, json, plain};

pub fn inspect_model(source: &Path) -> Result<LocalModel> {
    let root = canonical_plain(source)?;
    ensure!(
        plain(&root)?.is_dir(),
        "Choose a local checkpoint directory, not a file."
    );
    let config: Value = json(&contained(&root, "config.json")?)?;
    let architecture = config
        .get("model_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    ensure!(
        [
            "bert",
            "roberta",
            "xlm-roberta",
            "distilbert",
            "mpnet",
            "deberta",
            "deberta-v2",
            "electra",
            "modernbert"
        ]
        .contains(&architecture)
            && config.get("is_decoder").and_then(Value::as_bool) != Some(true),
        "This local model is not a supported encoder bundle. Use a BERT-family safetensors checkpoint; no custom code is executed."
    );
    ensure!(
        config.get("auto_map").is_none(),
        "Models requiring custom code are not supported."
    );
    let tokenizer: Value = json(&contained(&root, "tokenizer.json")?)?;
    ensure!(
        tokenizer.get("model").is_some_and(Value::is_object),
        "tokenizer.json must contain a self-contained tokenizer model."
    );
    validate_weights(&contained(&root, "model.safetensors")?)?;
    let sentence_transformer = root.join("modules.json").exists();
    if sentence_transformer {
        validate_modules(&root)?;
    }
    let mut files = vec![];
    inventory(&root, &root, &mut files, 0)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let result = LocalModel {
        source: root.to_string_lossy().into_owned(),
        format: if sentence_transformer {
            "sentence-transformers"
        } else {
            "safetensors-encoder"
        }
        .into(),
        architecture: architecture.into(),
        bytes: files.iter().map(|file| file.bytes).sum(),
        fingerprint: inventory_fingerprint(&files),
        files,
        execution: "not-configured".into(),
    };
    result.validate()?;
    Ok(result)
}

fn validate_weights(path: &Path) -> Result<()> {
    let mut file = File::open(path)?;
    let mut length = [0; 8];
    file.read_exact(&mut length)?;
    let length = u64::from_le_bytes(length);
    ensure!(
        (1..=16 * 1_048_576).contains(&length),
        "Invalid safetensors header size."
    );
    let mut header = vec![0; length as usize];
    file.read_exact(&mut header)?;
    // Deserializing Metadata validates tensor shapes, contiguous offsets and dtypes.
    let metadata: safetensors::tensor::Metadata = serde_json::from_slice(&header)?;
    ensure!(
        metadata.data_len() > 0
            && length
                .checked_add(8)
                .and_then(|n| n.checked_add(metadata.data_len() as u64))
                == Some(file.metadata()?.len()),
        "Safetensors contents do not match the validated header."
    );
    Ok(())
}

fn validate_modules(root: &Path) -> Result<()> {
    let modules: Vec<Value> = json(&contained(root, "modules.json")?)?;
    ensure!(
        !modules.is_empty(),
        "Sentence-transformer modules are empty."
    );
    for (index, module) in modules.iter().enumerate() {
        ensure!(
            module.get("idx").and_then(Value::as_u64) == Some(index as u64),
            "Module order is invalid."
        );
        let kind = module.get("type").and_then(Value::as_str).unwrap_or("");
        let path = module.get("path").and_then(Value::as_str).unwrap_or("");
        if index == 0 {
            ensure!(
                kind == "sentence_transformers.models.Transformer" && path.is_empty(),
                "The first module must use the local root transformer."
            );
        } else {
            ensure!(
                [
                    "sentence_transformers.models.Pooling",
                    "sentence_transformers.models.Normalize"
                ]
                .contains(&kind),
                "Unsupported sentence-transformer module: {kind}"
            );
            validate_relative(path)?;
            ensure!(
                plain(&contained(root, path)?)?.is_dir(),
                "Module directory is missing."
            );
            if kind.ends_with(".Pooling") {
                let _: Value = json(&contained(root, &format!("{path}/config.json"))?)?;
            }
        }
    }
    Ok(())
}

fn inventory(
    root: &Path,
    directory: &Path,
    result: &mut Vec<FileIdentity>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 8, "Model bundle directories are too deeply nested.");
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let metadata = plain(&path)?;
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Model filenames must be UTF-8."))?
            .replace('\\', "/");
        validate_relative(&relative)?;
        if metadata.is_dir() {
            inventory(root, &path, result, depth + 1)?;
        } else {
            let extension = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            ensure!(
                ["json", "safetensors", "txt", "md", "model"].contains(&extension.as_str())
                    && !relative.split('/').any(|s| s.starts_with('.')),
                "Unsupported file in checkpoint: {relative}. Choose a clean model bundle, not a source repository."
            );
            ensure!(
                metadata.len() <= 16 * 1024 * 1_048_576_u64 && result.len() < 512,
                "Checkpoint exceeds local import limits."
            );
            result.push(hash(&path, &relative)?);
        }
    }
    Ok(())
}
