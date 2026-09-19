//! Closed projection of an already admitted training sample. No registry,
//! hidden state, provider output or benchmark rows enter the desktop payload.
use encoder_optimization_core::generation::AdmittedGenerationRow;
use serde::Serialize;

use crate::{
    EncoderTaskAdapterError, adapter_error, managed_training::validate_native_training_row,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosGenerationPreviewRow {
    pub index: u32,
    pub fingerprint: String,
    pub question: String,
    pub task_kind: Option<String>,
}

pub fn project_generation_preview(
    row: &AdmittedGenerationRow,
) -> Result<NomosGenerationPreviewRow, EncoderTaskAdapterError> {
    validate_native_training_row(&row.content)?;
    if row.index >= 8
        || artifact_core::fingerprint(&row.content).map_err(adapter_error)? != row.fingerprint
    {
        return Err(adapter_error(
            "Generation preview has an invalid saved row identity",
        ));
    }
    let question = row.content["question"]
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= 8192)
        .ok_or_else(|| adapter_error("Generation preview question is invalid"))?;
    let task_kind = row.content["task_kind"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(|text| {
            if text.chars().count() > 200 {
                format!("{}…", text.chars().take(199).collect::<String>())
            } else {
                text.to_owned()
            }
        });
    Ok(NomosGenerationPreviewRow {
        index: row.index,
        fingerprint: row.fingerprint.clone(),
        question: question.into(),
        task_kind,
    })
}
