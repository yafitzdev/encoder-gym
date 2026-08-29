//! Tiny offline BERT bundle construction for integration tests and examples.

use std::path::Path;

use candle_core::{DType, Device};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::bert::{Config, HiddenAct, PositionEmbeddingType};
use tokenizers::{
    Tokenizer, models::wordpiece::WordPiece, pre_tokenizers::whitespace::Whitespace,
    processors::bert::BertProcessing,
};

use crate::BertClassifier;

pub fn write_tiny_bert_bundle(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let configuration = tiny_configuration();
    std::fs::write(
        directory.join("config.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "vocab_size": 16,
            "hidden_size": 8,
            "num_hidden_layers": 1,
            "num_attention_heads": 2,
            "intermediate_size": 16,
            "hidden_act": "gelu",
            "hidden_dropout_prob": 0.0,
            "max_position_embeddings": 16,
            "type_vocab_size": 2,
            "initializer_range": 0.02,
            "layer_norm_eps": 1e-12,
            "pad_token_id": 0,
            "position_embedding_type": "absolute",
            "use_cache": false,
            "classifier_dropout": null,
            "model_type": "bert"
        }))
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let vocabulary = [
        ("[PAD]".to_owned(), 0),
        ("[UNK]".to_owned(), 1),
        ("[CLS]".to_owned(), 2),
        ("[SEP]".to_owned(), 3),
        ("[MASK]".to_owned(), 4),
        ("billing".to_owned(), 5),
        ("account".to_owned(), 6),
        ("invoice".to_owned(), 7),
        ("login".to_owned(), 8),
        ("payment".to_owned(), 9),
        ("access".to_owned(), 10),
        ("charged".to_owned(), 11),
        ("password".to_owned(), 12),
        ("technical".to_owned(), 13),
        ("fraud".to_owned(), 14),
        ("help".to_owned(), 15),
    ];
    let wordpiece = WordPiece::builder()
        .vocab(vocabulary)
        .unk_token("[UNK]".into())
        .build()
        .map_err(|error| error.to_string())?;
    let mut tokenizer = Tokenizer::new(wordpiece);
    tokenizer.with_pre_tokenizer(Some(Whitespace));
    tokenizer.with_post_processor(Some(BertProcessing::new(
        ("[SEP]".into(), 3),
        ("[CLS]".into(), 2),
    )));
    tokenizer
        .save(directory.join("tokenizer.json"), false)
        .map_err(|error| error.to_string())?;

    let device = Device::Cpu;
    let variables = VarMap::new();
    let builder = VarBuilder::from_varmap(&variables, DType::F32, &device);
    BertClassifier::load(builder, &configuration, 2).map_err(|error| error.to_string())?;
    variables
        .save(directory.join("model.safetensors"))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn tiny_configuration() -> Config {
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
