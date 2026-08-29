use candle_core::{IndexOp, Tensor};
use candle_nn::{Linear, Module, VarBuilder, linear};
use candle_transformers::models::bert::{BertModel, Config};

/// BERT encoder plus a project-owned sequence-classification head.
///
/// The adapter intentionally supports the canonical Hugging Face tensor prefix
/// `bert.*` and a `classifier.*` head. Framework types remain private to this
/// crate once the training and predictor ports are implemented.
pub struct BertClassifier {
    encoder: BertModel,
    classifier: Linear,
}

impl BertClassifier {
    pub fn load(
        vb: VarBuilder<'_>,
        config: &Config,
        label_count: usize,
    ) -> candle_core::Result<Self> {
        let encoder = BertModel::load(vb.pp("bert"), config)?;
        let classifier = linear(config.hidden_size, label_count, vb.pp("classifier"))?;
        Ok(Self {
            encoder,
            classifier,
        })
    }

    pub fn forward(
        &self,
        input_ids: &Tensor,
        token_type_ids: &Tensor,
        attention_mask: &Tensor,
    ) -> candle_core::Result<Tensor> {
        let encoded = self
            .encoder
            .forward(input_ids, token_type_ids, Some(attention_mask))?;
        let cls = encoded.i((.., 0, ..))?;
        self.classifier.forward(&cls)
    }
}

#[cfg(test)]
mod tests {
    use candle_core::{DType, Device, Tensor};
    use candle_nn::{VarBuilder, VarMap};
    use candle_transformers::models::bert::{Config, HiddenAct, PositionEmbeddingType};

    use super::BertClassifier;

    fn tiny_config() -> Config {
        Config {
            vocab_size: 16,
            hidden_size: 8,
            num_hidden_layers: 1,
            num_attention_heads: 2,
            intermediate_size: 16,
            hidden_act: HiddenAct::Gelu,
            hidden_dropout_prob: 0.0,
            max_position_embeddings: 16,
            type_vocab_size: 2,
            initializer_range: 0.02,
            layer_norm_eps: 1e-12,
            pad_token_id: 0,
            position_embedding_type: PositionEmbeddingType::Absolute,
            use_cache: false,
            classifier_dropout: None,
            model_type: Some("bert".into()),
        }
    }

    #[test]
    fn tiny_bert_forward_pass_has_classification_shape() {
        let device = Device::Cpu;
        let variables = VarMap::new();
        let builder = VarBuilder::from_varmap(&variables, DType::F32, &device);
        let model = BertClassifier::load(builder, &tiny_config(), 3).expect("tiny BERT");
        let input_ids = Tensor::new(&[[2_u32, 5, 6, 3], [2, 7, 8, 3]], &device).expect("input ids");
        let token_types = Tensor::zeros((2, 4), DType::U32, &device).expect("token types");
        let attention = Tensor::ones((2, 4), DType::U32, &device).expect("attention");

        let logits = model
            .forward(&input_ids, &token_types, &attention)
            .expect("forward pass");

        assert_eq!(logits.dims(), &[2, 3]);
        assert!(variables.all_vars().len() > 10);
    }
}
