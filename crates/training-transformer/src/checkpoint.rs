use std::collections::BTreeMap;

use candle_core::{Device, Tensor};
use serde::{Deserialize, Serialize};
use training_core::domain::{TrainingConfiguration, TransformerTrainingConfiguration};
use uuid::Uuid;

const MAGIC: &[u8; 8] = b"STBERT01";
const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BertCheckpointManifest {
    pub format_version: u32,
    pub architecture: String,
    pub base_model_id: Uuid,
    pub base_model_fingerprint: String,
    pub tokenizer_fingerprint: String,
    pub snapshot_id: Uuid,
    pub labels: Vec<String>,
    pub bert_configuration_json: String,
    pub tokenizer_json: String,
    pub training_configuration: TrainingConfiguration,
    pub transformer_configuration: TransformerTrainingConfiguration,
    pub completed_epoch: u32,
    pub completed_batch: u32,
    pub optimizer_step: u64,
}

pub(crate) struct BertCheckpoint {
    pub manifest: BertCheckpointManifest,
    pub tensors: BTreeMap<String, Tensor>,
}

impl BertCheckpoint {
    pub fn serialize(&self) -> Result<Vec<u8>, String> {
        let manifest = serde_json::to_vec(&self.manifest).map_err(|error| error.to_string())?;
        if manifest.len() > MAX_MANIFEST_BYTES {
            return Err("checkpoint manifest is too large".into());
        }
        let tensor_bytes = safetensors::tensor::serialize(
            self.tensors
                .iter()
                .map(|(name, tensor)| (name.as_str(), tensor)),
            None,
        )
        .map_err(|error| error.to_string())?;
        let manifest_length = u64::try_from(manifest.len())
            .map_err(|_| "manifest length does not fit u64".to_owned())?;
        let tensor_length = u64::try_from(tensor_bytes.len())
            .map_err(|_| "tensor length does not fit u64".to_owned())?;
        let mut bytes = Vec::with_capacity(24 + manifest.len() + tensor_bytes.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&manifest_length.to_le_bytes());
        bytes.extend_from_slice(&tensor_length.to_le_bytes());
        bytes.extend_from_slice(&manifest);
        bytes.extend_from_slice(&tensor_bytes);
        Ok(bytes)
    }

    pub fn deserialize(bytes: &[u8], device: &Device) -> Result<Self, String> {
        if bytes.get(..8) != Some(MAGIC) {
            return Err("checkpoint magic or model format is invalid".into());
        }
        let manifest_length = read_length(bytes, 8)?;
        let tensor_length = read_length(bytes, 16)?;
        if manifest_length > MAX_MANIFEST_BYTES {
            return Err("checkpoint manifest is too large".into());
        }
        let manifest_start = 24_usize;
        let manifest_end = manifest_start
            .checked_add(manifest_length)
            .ok_or_else(|| "checkpoint manifest length overflows".to_owned())?;
        let tensor_end = manifest_end
            .checked_add(tensor_length)
            .ok_or_else(|| "checkpoint tensor length overflows".to_owned())?;
        if tensor_end != bytes.len() {
            return Err("checkpoint lengths do not cover the artifact exactly".into());
        }
        let manifest: BertCheckpointManifest = serde_json::from_slice(
            bytes
                .get(manifest_start..manifest_end)
                .ok_or_else(|| "checkpoint manifest is truncated".to_owned())?,
        )
        .map_err(|error| error.to_string())?;
        if manifest.format_version != 1 || manifest.architecture != "bert" {
            return Err("checkpoint manifest version or architecture is unsupported".into());
        }
        let tensors = candle_core::safetensors::load_buffer(
            bytes
                .get(manifest_end..tensor_end)
                .ok_or_else(|| "checkpoint tensors are truncated".to_owned())?,
            device,
        )
        .map_err(|error| error.to_string())?
        .into_iter()
        .collect();
        Ok(Self { manifest, tensors })
    }
}

fn read_length(bytes: &[u8], offset: usize) -> Result<usize, String> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "checkpoint header is truncated".to_owned())?
        .try_into()
        .map_err(|_| "checkpoint length is invalid".to_owned())?;
    usize::try_from(u64::from_le_bytes(raw))
        .map_err(|_| "checkpoint length does not fit usize".to_owned())
}
