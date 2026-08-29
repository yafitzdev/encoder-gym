use candle_core::{Device, Tensor};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};
use training_core::domain::TrainingExample;
use training_core::domain::TransformerTrainingConfiguration;

pub(crate) struct TokenizedBatch {
    pub input_ids: Tensor,
    pub token_type_ids: Tensor,
    pub attention_mask: Tensor,
    pub targets: Tensor,
    pub size: usize,
}

#[derive(Clone)]
pub(crate) struct BertTokenizer {
    tokenizer: Tokenizer,
}

impl BertTokenizer {
    pub fn from_bytes(
        bytes: &[u8],
        padding_token_id: usize,
        configuration: &TransformerTrainingConfiguration,
    ) -> Result<Self, String> {
        let mut tokenizer = Tokenizer::from_bytes(bytes).map_err(|error| error.to_string())?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: configuration.maximum_sequence_length,
                ..TruncationParams::default()
            }))
            .map_err(|error| error.to_string())?;
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_id: u32::try_from(padding_token_id)
                .map_err(|_| "padding token ID does not fit u32".to_owned())?,
            ..PaddingParams::default()
        }));
        Ok(Self { tokenizer })
    }

    pub fn encode(
        &self,
        examples: &[&TrainingExample],
        labels: &[String],
        device: &Device,
    ) -> Result<TokenizedBatch, String> {
        if examples.is_empty() {
            return Err("cannot tokenize an empty batch".into());
        }
        let encodings = self
            .tokenizer
            .encode_batch(
                examples
                    .iter()
                    .map(|example| example.text.as_str())
                    .collect::<Vec<_>>(),
                true,
            )
            .map_err(|error| error.to_string())?;
        let sequence_length = encodings
            .first()
            .map(|encoding| encoding.get_ids().len())
            .ok_or_else(|| "tokenizer returned no encodings".to_owned())?;
        if sequence_length == 0
            || encodings
                .iter()
                .any(|encoding| encoding.get_ids().len() != sequence_length)
        {
            return Err("tokenizer returned empty or uneven padded encodings".into());
        }
        let flatten = |select: fn(&tokenizers::Encoding) -> &[u32]| {
            encodings
                .iter()
                .flat_map(select)
                .copied()
                .collect::<Vec<_>>()
        };
        let shape = (examples.len(), sequence_length);
        let input_ids = Tensor::from_vec(flatten(tokenizers::Encoding::get_ids), shape, device)
            .map_err(|error| error.to_string())?;
        let token_type_ids =
            Tensor::from_vec(flatten(tokenizers::Encoding::get_type_ids), shape, device)
                .map_err(|error| error.to_string())?;
        let attention_mask = Tensor::from_vec(
            flatten(tokenizers::Encoding::get_attention_mask),
            shape,
            device,
        )
        .map_err(|error| error.to_string())?;
        let targets = examples
            .iter()
            .map(|example| {
                labels
                    .iter()
                    .position(|label| label == &example.label)
                    .and_then(|index| u32::try_from(index).ok())
                    .ok_or_else(|| format!("unknown or oversized label: {}", example.label))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let targets =
            Tensor::from_vec(targets, examples.len(), device).map_err(|error| error.to_string())?;
        Ok(TokenizedBatch {
            input_ids,
            token_type_ids,
            attention_mask,
            targets,
            size: examples.len(),
        })
    }
}
