use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use candle_transformers::models::bert::Config;
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokenizers::Tokenizer;
use training_core::domain::{
    EncoderArchitecture, EncoderArtifact, EncoderMetadata, RegisteredEncoder,
};
use uuid::Uuid;

const CONFIG_FILE: &str = "config.json";
const TOKENIZER_FILE: &str = "tokenizer.json";
const WEIGHTS_FILE: &str = "model.safetensors";
const MAX_SAFETENSORS_HEADER_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("encoder bundle path does not exist or is not a directory: {0}")]
    Directory(PathBuf),
    #[error("could not read encoder artifact {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid BERT configuration: {0}")]
    Configuration(String),
    #[error("invalid tokenizer: {0}")]
    Tokenizer(String),
    #[error("invalid safetensors file: {0}")]
    SafeTensors(String),
    #[error("required tensor {name} has shape {actual:?}; expected {expected:?}")]
    TensorShape {
        name: String,
        actual: Vec<usize>,
        expected: Vec<usize>,
    },
    #[error("required tensor is missing: {0}")]
    MissingTensor(String),
    #[error("required tensor {name} uses {actual}; only F32 is supported")]
    TensorDtype { name: String, actual: String },
    #[error("could not fingerprint encoder bundle: {0}")]
    Fingerprint(String),
}

#[derive(Debug, Clone)]
pub struct BertBundle {
    registration: RegisteredEncoder,
    configuration: Config,
}

impl BertBundle {
    pub fn inspect(
        name: impl Into<String>,
        directory: impl AsRef<Path>,
    ) -> Result<Self, BundleError> {
        let name = name.into();
        let directory = canonical_directory(directory.as_ref())?;
        let configuration_path = directory.join(CONFIG_FILE);
        let tokenizer_path = directory.join(TOKENIZER_FILE);
        let weights_path = directory.join(WEIGHTS_FILE);

        let configuration_bytes = read_file(&configuration_path)?;
        let configuration: Config = serde_json::from_slice(&configuration_bytes)
            .map_err(|error| BundleError::Configuration(error.to_string()))?;
        validate_configuration(&configuration)?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|error| BundleError::Tokenizer(error.to_string()))?;
        validate_tokenizer(&tokenizer, &configuration)?;

        let tensors = read_tensor_header(&weights_path)?;
        validate_tensor_shapes(&tensors, &configuration)?;

        let configuration_artifact = artifact(CONFIG_FILE, &configuration_path)?;
        let tokenizer_artifact = artifact(TOKENIZER_FILE, &tokenizer_path)?;
        let weights_artifact = artifact(WEIGHTS_FILE, &weights_path)?;
        let metadata = EncoderMetadata {
            vocab_size: configuration.vocab_size,
            hidden_size: configuration.hidden_size,
            layer_count: configuration.num_hidden_layers,
            attention_head_count: configuration.num_attention_heads,
            intermediate_size: configuration.intermediate_size,
            maximum_position_embeddings: configuration.max_position_embeddings,
            type_vocab_size: configuration.type_vocab_size,
            padding_token_id: configuration.pad_token_id,
            tensor_dtype: "F32".into(),
        };
        let fingerprint = artifact_core::fingerprint(&BundleIdentity {
            architecture: EncoderArchitecture::Bert,
            configuration_checksum: &configuration_artifact.checksum,
            tokenizer_checksum: &tokenizer_artifact.checksum,
            weights_checksum: &weights_artifact.checksum,
            metadata: &metadata,
        })
        .map_err(|error| BundleError::Fingerprint(error.to_string()))?;
        let registration = RegisteredEncoder {
            id: Uuid::new_v4(),
            name,
            architecture: EncoderArchitecture::Bert,
            source_path: path_text(&directory),
            fingerprint,
            configuration: configuration_artifact,
            tokenizer: tokenizer_artifact,
            weights: weights_artifact,
            metadata,
            created_at: Utc::now(),
        };
        registration
            .validate()
            .map_err(|error| BundleError::Configuration(error.to_string()))?;
        Ok(Self {
            registration,
            configuration,
        })
    }

    pub fn registration(&self) -> &RegisteredEncoder {
        &self.registration
    }

    pub fn into_registration(self) -> RegisteredEncoder {
        self.registration
    }

    pub fn configuration(&self) -> &Config {
        &self.configuration
    }

    pub fn verify(registration: &RegisteredEncoder) -> Result<Self, BundleError> {
        let inspected = Self::inspect(&registration.name, &registration.source_path)?;
        if inspected.registration.fingerprint != registration.fingerprint {
            return Err(BundleError::Fingerprint(format!(
                "registered fingerprint {} does not match current fingerprint {}",
                registration.fingerprint, inspected.registration.fingerprint
            )));
        }
        Ok(Self {
            registration: registration.clone(),
            configuration: inspected.configuration,
        })
    }
}

#[derive(Serialize)]
struct BundleIdentity<'a> {
    architecture: EncoderArchitecture,
    configuration_checksum: &'a str,
    tokenizer_checksum: &'a str,
    weights_checksum: &'a str,
    metadata: &'a EncoderMetadata,
}

#[derive(Debug)]
struct TensorHeader {
    dtype: String,
    shape: Vec<usize>,
    start_offset: u64,
    end_offset: u64,
}

fn canonical_directory(path: &Path) -> Result<PathBuf, BundleError> {
    if !path.is_dir() {
        return Err(BundleError::Directory(path.to_path_buf()));
    }
    fs::canonicalize(path).map_err(|source| BundleError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn read_file(path: &Path) -> Result<Vec<u8>, BundleError> {
    fs::read(path).map_err(|source| BundleError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn artifact(file_name: &str, path: &Path) -> Result<EncoderArtifact, BundleError> {
    let mut file = File::open(path).map_err(|source| BundleError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let size_bytes = file
        .metadata()
        .map_err(|source| BundleError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| BundleError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(EncoderArtifact {
        file_name: file_name.into(),
        path: path_text(path),
        size_bytes,
        checksum: format!("{:x}", digest.finalize()),
    })
}

fn validate_configuration(configuration: &Config) -> Result<(), BundleError> {
    if configuration.model_type.as_deref() != Some("bert") {
        return Err(BundleError::Configuration(
            "model_type must be exactly `bert`".into(),
        ));
    }
    if configuration.vocab_size == 0
        || configuration.hidden_size == 0
        || configuration.num_hidden_layers == 0
        || configuration.num_attention_heads == 0
        || configuration.intermediate_size == 0
        || configuration.max_position_embeddings < 2
        || configuration.type_vocab_size == 0
        || configuration.hidden_size % configuration.num_attention_heads != 0
        || configuration.pad_token_id >= configuration.vocab_size
    {
        return Err(BundleError::Configuration(
            "model dimensions, attention heads, positions, or padding token are invalid".into(),
        ));
    }
    Ok(())
}

fn validate_tokenizer(tokenizer: &Tokenizer, configuration: &Config) -> Result<(), BundleError> {
    let actual_vocab = tokenizer.get_vocab_size(true);
    if actual_vocab != configuration.vocab_size {
        return Err(BundleError::Tokenizer(format!(
            "tokenizer vocabulary has {actual_vocab} entries but config declares {}",
            configuration.vocab_size
        )));
    }
    if tokenizer
        .id_to_token(configuration.pad_token_id as u32)
        .as_deref()
        != Some("[PAD]")
    {
        return Err(BundleError::Tokenizer(format!(
            "pad_token_id {} does not identify [PAD]",
            configuration.pad_token_id
        )));
    }
    Ok(())
}

fn read_tensor_header(path: &Path) -> Result<BTreeMap<String, TensorHeader>, BundleError> {
    let mut file = File::open(path).map_err(|source| BundleError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let file_size = file
        .metadata()
        .map_err(|source| BundleError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let mut length_bytes = [0_u8; 8];
    file.read_exact(&mut length_bytes)
        .map_err(|error| BundleError::SafeTensors(error.to_string()))?;
    let header_length = u64::from_le_bytes(length_bytes);
    if header_length == 0 || header_length > MAX_SAFETENSORS_HEADER_BYTES {
        return Err(BundleError::SafeTensors(format!(
            "header length {header_length} is invalid"
        )));
    }
    let header_len = usize::try_from(header_length)
        .map_err(|_| BundleError::SafeTensors("header does not fit in memory".into()))?;
    let mut header = vec![0_u8; header_len];
    file.read_exact(&mut header)
        .map_err(|error| BundleError::SafeTensors(error.to_string()))?;
    let value: serde_json::Value = serde_json::from_slice(&header)
        .map_err(|error| BundleError::SafeTensors(error.to_string()))?;
    let fields = value
        .as_object()
        .ok_or_else(|| BundleError::SafeTensors("header must be a JSON object".into()))?;
    let mut tensors = BTreeMap::new();
    let mut maximum_end = 0_u64;
    for (name, value) in fields {
        if name == "__metadata__" {
            continue;
        }
        let dtype = value
            .get("dtype")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| BundleError::SafeTensors(format!("tensor {name} has no dtype")))?
            .to_owned();
        let shape = value
            .get("shape")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| BundleError::SafeTensors(format!("tensor {name} has no shape")))?
            .iter()
            .map(|dimension| {
                dimension
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| {
                        BundleError::SafeTensors(format!("tensor {name} has an invalid shape"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let offsets = value
            .get("data_offsets")
            .and_then(serde_json::Value::as_array)
            .filter(|values| values.len() == 2)
            .ok_or_else(|| {
                BundleError::SafeTensors(format!("tensor {name} has invalid offsets"))
            })?;
        let start_offset = offsets[0].as_u64().ok_or_else(|| {
            BundleError::SafeTensors(format!("tensor {name} has an invalid start offset"))
        })?;
        let end_offset = offsets[1].as_u64().ok_or_else(|| {
            BundleError::SafeTensors(format!("tensor {name} has an invalid end offset"))
        })?;
        if start_offset > end_offset {
            return Err(BundleError::SafeTensors(format!(
                "tensor {name} starts after it ends"
            )));
        }
        maximum_end = maximum_end.max(end_offset);
        tensors.insert(
            name.clone(),
            TensorHeader {
                dtype,
                shape,
                start_offset,
                end_offset,
            },
        );
    }
    let data_start = 8_u64
        .checked_add(header_length)
        .ok_or_else(|| BundleError::SafeTensors("header length overflow".into()))?;
    if data_start.checked_add(maximum_end) != Some(file_size) {
        return Err(BundleError::SafeTensors(
            "tensor offsets do not cover the file exactly".into(),
        ));
    }
    validate_tensor_layout(&tensors, maximum_end)?;
    file.seek(SeekFrom::Start(data_start))
        .map_err(|error| BundleError::SafeTensors(error.to_string()))?;
    if tensors.is_empty() || tensors.values().all(|tensor| tensor.end_offset == 0) {
        return Err(BundleError::SafeTensors(
            "weights contain no tensor data".into(),
        ));
    }
    Ok(tensors)
}

fn validate_tensor_layout(
    tensors: &BTreeMap<String, TensorHeader>,
    data_length: u64,
) -> Result<(), BundleError> {
    let mut ranges = tensors.iter().collect::<Vec<_>>();
    ranges.sort_by_key(|(_, tensor)| tensor.start_offset);
    let mut expected_start = 0_u64;
    for (name, tensor) in ranges {
        if tensor.dtype != "F32" {
            return Err(BundleError::TensorDtype {
                name: name.clone(),
                actual: tensor.dtype.clone(),
            });
        }
        if tensor.start_offset != expected_start {
            return Err(BundleError::SafeTensors(format!(
                "tensor {name} overlaps another tensor or leaves a data gap"
            )));
        }
        let element_count = tensor.shape.iter().try_fold(1_u64, |count, dimension| {
            count.checked_mul(u64::try_from(*dimension).ok()?)
        });
        let expected_bytes = element_count
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| {
                BundleError::SafeTensors(format!("tensor {name} byte size overflows"))
            })?;
        if tensor.end_offset - tensor.start_offset != expected_bytes {
            return Err(BundleError::SafeTensors(format!(
                "tensor {name} byte range does not match its F32 shape"
            )));
        }
        expected_start = tensor.end_offset;
    }
    if expected_start != data_length {
        return Err(BundleError::SafeTensors(
            "tensor ranges do not cover the data section exactly".into(),
        ));
    }
    Ok(())
}

fn validate_tensor_shapes(
    tensors: &BTreeMap<String, TensorHeader>,
    configuration: &Config,
) -> Result<(), BundleError> {
    for (name, expected) in required_tensors(configuration) {
        let tensor = tensors
            .get(&name)
            .ok_or_else(|| BundleError::MissingTensor(name.clone()))?;
        if tensor.dtype != "F32" {
            return Err(BundleError::TensorDtype {
                name,
                actual: tensor.dtype.clone(),
            });
        }
        if tensor.shape != expected {
            return Err(BundleError::TensorShape {
                name,
                actual: tensor.shape.clone(),
                expected,
            });
        }
    }
    Ok(())
}

fn required_tensors(configuration: &Config) -> Vec<(String, Vec<usize>)> {
    let hidden = configuration.hidden_size;
    let intermediate = configuration.intermediate_size;
    let mut tensors = vec![
        (
            "bert.embeddings.word_embeddings.weight".into(),
            vec![configuration.vocab_size, hidden],
        ),
        (
            "bert.embeddings.position_embeddings.weight".into(),
            vec![configuration.max_position_embeddings, hidden],
        ),
        (
            "bert.embeddings.token_type_embeddings.weight".into(),
            vec![configuration.type_vocab_size, hidden],
        ),
        ("bert.embeddings.LayerNorm.weight".into(), vec![hidden]),
        ("bert.embeddings.LayerNorm.bias".into(), vec![hidden]),
    ];
    for layer in 0..configuration.num_hidden_layers {
        let prefix = format!("bert.encoder.layer.{layer}");
        for projection in ["query", "key", "value"] {
            tensors.push((
                format!("{prefix}.attention.self.{projection}.weight"),
                vec![hidden, hidden],
            ));
            tensors.push((
                format!("{prefix}.attention.self.{projection}.bias"),
                vec![hidden],
            ));
        }
        tensors.extend([
            (
                format!("{prefix}.attention.output.dense.weight"),
                vec![hidden, hidden],
            ),
            (
                format!("{prefix}.attention.output.dense.bias"),
                vec![hidden],
            ),
            (
                format!("{prefix}.attention.output.LayerNorm.weight"),
                vec![hidden],
            ),
            (
                format!("{prefix}.attention.output.LayerNorm.bias"),
                vec![hidden],
            ),
            (
                format!("{prefix}.intermediate.dense.weight"),
                vec![intermediate, hidden],
            ),
            (
                format!("{prefix}.intermediate.dense.bias"),
                vec![intermediate],
            ),
            (
                format!("{prefix}.output.dense.weight"),
                vec![hidden, intermediate],
            ),
            (format!("{prefix}.output.dense.bias"), vec![hidden]),
            (format!("{prefix}.output.LayerNorm.weight"), vec![hidden]),
            (format!("{prefix}.output.LayerNorm.bias"), vec![hidden]),
        ]);
    }
    tensors
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use candle_core::{DType, Device};
    use candle_nn::{VarBuilder, VarMap};
    use candle_transformers::models::bert::{Config, HiddenAct, PositionEmbeddingType};
    use tokenizers::{
        Tokenizer, models::wordpiece::WordPiece, pre_tokenizers::whitespace::Whitespace,
        processors::bert::BertProcessing,
    };

    use crate::model::BertClassifier;

    use super::{BertBundle, BundleError};

    pub(crate) fn tiny_config() -> Config {
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

    pub(crate) fn write_tiny_bundle(directory: &std::path::Path) {
        let configuration = tiny_config();
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
            .expect("configuration JSON"),
        )
        .expect("configuration file");

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
            .expect("wordpiece");
        let mut tokenizer = Tokenizer::new(wordpiece);
        tokenizer.with_pre_tokenizer(Some(Whitespace));
        tokenizer.with_post_processor(Some(BertProcessing::new(
            ("[SEP]".into(), 3),
            ("[CLS]".into(), 2),
        )));
        tokenizer
            .save(directory.join("tokenizer.json"), false)
            .expect("tokenizer file");

        let device = Device::Cpu;
        let variables = VarMap::new();
        let builder = VarBuilder::from_varmap(&variables, DType::F32, &device);
        BertClassifier::load(builder, &configuration, 2).expect("model variables");
        variables
            .save(directory.join("model.safetensors"))
            .expect("weights file");
    }

    #[test]
    fn validates_and_fingerprints_a_tiny_local_bundle() {
        let directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(directory.path());

        let first = BertBundle::inspect("tiny", directory.path()).expect("valid bundle");
        let second = BertBundle::inspect("renamed", directory.path()).expect("same bundle");

        assert_eq!(
            first.registration().fingerprint,
            second.registration().fingerprint
        );
        assert_eq!(first.registration().metadata.hidden_size, 8);
        assert_eq!(first.registration().metadata.layer_count, 1);
        BertBundle::verify(first.registration()).expect("unchanged bundle verifies");
    }

    #[test]
    fn detects_changed_bundle_artifacts() {
        let directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(directory.path());
        let registered = BertBundle::inspect("tiny", directory.path())
            .expect("valid bundle")
            .into_registration();
        let tokenizer_path = directory.path().join("tokenizer.json");
        let mut tokenizer = std::fs::read_to_string(&tokenizer_path).expect("tokenizer source");
        tokenizer.push(' ');
        std::fs::write(tokenizer_path, tokenizer).expect("changed tokenizer");

        assert!(BertBundle::verify(&registered).is_err());
    }

    #[test]
    fn rejects_tokenizer_and_weight_shape_mismatches_before_training() {
        let tokenizer_directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(tokenizer_directory.path());
        let configuration_path = tokenizer_directory.path().join("config.json");
        let mut configuration: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&configuration_path).expect("configuration"))
                .expect("configuration JSON");
        configuration["vocab_size"] = 15.into();
        std::fs::write(
            &configuration_path,
            serde_json::to_vec_pretty(&configuration).expect("JSON"),
        )
        .expect("changed config");
        assert!(matches!(
            BertBundle::inspect("invalid-tokenizer", tokenizer_directory.path()),
            Err(BundleError::Tokenizer(_))
        ));

        let shape_directory = tempfile::tempdir().expect("temporary directory");
        write_tiny_bundle(shape_directory.path());
        let configuration_path = shape_directory.path().join("config.json");
        let mut configuration: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&configuration_path).expect("configuration"))
                .expect("configuration JSON");
        configuration["hidden_size"] = 10.into();
        std::fs::write(
            configuration_path,
            serde_json::to_vec_pretty(&configuration).expect("JSON"),
        )
        .expect("changed config");
        assert!(matches!(
            BertBundle::inspect("invalid-shapes", shape_directory.path()),
            Err(BundleError::TensorShape { .. })
        ));
    }
}
