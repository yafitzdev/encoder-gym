//! Strict TOML configuration and domain-validated resolution for local projects.

use std::{collections::BTreeMap, future::Future, path::PathBuf, pin::Pin};

use chrono::Utc;
use dataset_core::domain::{SnapshotSplit, SplitConfiguration, SplitRatios};
use generation_core::{
    construction::{FieldDefinition, RowConstructionPlan},
    domain::{
        BackendConfiguration, DatasetDefinition, DimensionDefinition, GenerationParameters,
        GenerationPlan,
    },
    planning::equal_target_plan,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use training_core::domain::{TrainingConfiguration, TransformerTrainingConfiguration};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub version: u32,
    pub dataset: DatasetSection,
    #[serde(default)]
    pub generation: GenerationSection,
    #[serde(default)]
    pub snapshot: SnapshotSection,
    #[serde(default)]
    pub training: TrainingSection,
    #[serde(default)]
    pub evaluation: EvaluationSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetSection {
    pub name: String,
    pub task: String,
    pub labels: Vec<String>,
    #[serde(default)]
    pub dimensions: Vec<DimensionSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionSection {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GenerationSection {
    pub target_per_cell: u32,
    pub batch_size: u32,
    pub max_retries: u32,
    pub max_attempt_multiplier: u32,
    pub backend: GenerationBackendKind,
    pub base_url: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub seed: Option<u64>,
    pub extra: BTreeMap<String, Value>,
    pub api_key_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub construction: Option<ConstructionSection>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConstructionSection {
    pub seed: u64,
    pub fields: Vec<FieldDefinition>,
}

impl Default for GenerationSection {
    fn default() -> Self {
        Self {
            target_per_cell: 10,
            batch_size: 20,
            max_retries: 3,
            max_attempt_multiplier: 3,
            backend: GenerationBackendKind::Fake,
            base_url: None,
            model: "deterministic-v1".into(),
            temperature: None,
            max_tokens: None,
            seed: None,
            extra: BTreeMap::new(),
            api_key_env: "SYNTH_OPENAI_API_KEY".into(),
            construction: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenerationBackendKind {
    Fake,
    OpenaiCompatible,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SnapshotSection {
    pub name: String,
    pub description: Option<String>,
    pub train_ratio: f64,
    pub validation_ratio: f64,
    pub test_ratio: f64,
    pub seed: u64,
}

impl Default for SnapshotSection {
    fn default() -> Self {
        Self {
            name: "baseline".into(),
            description: None,
            train_ratio: 0.8,
            validation_ratio: 0.1,
            test_ratio: 0.1,
            seed: 42,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrainingSection {
    pub backend: TrainingBackendKind,
    pub base_model_id: Option<Uuid>,
    pub feature_dimension: usize,
    pub epochs: u32,
    pub learning_rate: f32,
    pub l2: f32,
    pub checkpoint_every: u32,
    pub seed: u64,
    pub artifact_root: PathBuf,
    pub transformer: TransformerTrainingConfiguration,
}

impl Default for TrainingSection {
    fn default() -> Self {
        let configuration = TrainingConfiguration::default();
        Self {
            backend: TrainingBackendKind::HashingLinear,
            base_model_id: None,
            feature_dimension: configuration.feature_dimension,
            epochs: configuration.epochs,
            learning_rate: configuration.learning_rate,
            l2: configuration.l2,
            checkpoint_every: configuration.checkpoint_every,
            seed: configuration.seed,
            artifact_root: PathBuf::from("artifacts/training"),
            transformer: TransformerTrainingConfiguration::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrainingBackendKind {
    HashingLinear,
    BertCpu,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluationSection {
    pub split: SnapshotSplit,
    pub batch_size: usize,
    pub top_k: Vec<usize>,
    pub calibration_bins: usize,
    pub minimum_slice_support: u64,
    pub bootstrap_samples: u32,
    pub statistical_seed: u64,
    pub confidence_level: f64,
    pub dimension_intersections: Vec<Vec<String>>,
}

impl Default for EvaluationSection {
    fn default() -> Self {
        Self {
            split: SnapshotSplit::Test,
            batch_size: 32,
            top_k: vec![1],
            calibration_bins: 10,
            minimum_slice_support: 1,
            bootstrap_samples: 1_000,
            statistical_seed: 42,
            confidence_level: 0.95,
            dimension_intersections: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectOverrides {
    pub dataset_name: Option<String>,
    pub target_per_cell: Option<u32>,
    pub snapshot_seed: Option<u64>,
    pub training_epochs: Option<u32>,
    pub training_learning_rate: Option<f32>,
    pub evaluation_split: Option<SnapshotSplit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedProjectConfig {
    pub version: u32,
    pub dataset: DatasetSection,
    pub generation: GenerationSection,
    pub snapshot: SnapshotSection,
    pub training: TrainingSection,
    pub evaluation: EvaluationSection,
}

impl ProjectConfig {
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        toml::from_str(source).map_err(|error| ConfigError::Parse(error.to_string()))
    }

    pub fn resolve(
        self,
        overrides: ProjectOverrides,
    ) -> Result<ResolvedProjectConfig, ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::Version(self.version));
        }
        let mut resolved = ResolvedProjectConfig {
            version: self.version,
            dataset: self.dataset,
            generation: self.generation,
            snapshot: self.snapshot,
            training: self.training,
            evaluation: self.evaluation,
        };
        if let Some(value) = overrides.dataset_name {
            resolved.dataset.name = value;
        }
        if let Some(value) = overrides.target_per_cell {
            resolved.generation.target_per_cell = value;
        }
        if let Some(value) = overrides.snapshot_seed {
            resolved.snapshot.seed = value;
        }
        if let Some(value) = overrides.training_epochs {
            resolved.training.epochs = value;
        }
        if let Some(value) = overrides.training_learning_rate {
            resolved.training.learning_rate = value;
        }
        if let Some(value) = overrides.evaluation_split {
            resolved.evaluation.split = value;
        }
        resolved.validate()?;
        Ok(resolved)
    }
}

impl ResolvedProjectConfig {
    pub fn fingerprint(&self) -> Result<String, ConfigError> {
        artifact_core::fingerprint(self)
            .map_err(|error| ConfigError::Fingerprint(error.to_string()))
    }

    pub fn persisted(
        &self,
        dataset_id: Uuid,
        generation_plan_id: Uuid,
    ) -> Result<PersistedProjectConfiguration, ConfigError> {
        Ok(PersistedProjectConfiguration {
            id: Uuid::new_v4(),
            fingerprint: self.fingerprint()?,
            dataset_id,
            generation_plan_id,
            resolved: self.clone(),
            created_at: Utc::now(),
        })
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let dataset = self.dataset_definition()?;
        self.split_configuration()?;
        self.training_configuration()?;
        self.training.transformer.validate()?;
        if self.generation.target_per_cell == 0 {
            return Err(ConfigError::Invalid(
                "generation.target_per_cell must be greater than zero".into(),
            ));
        }
        if self.generation.batch_size == 0 {
            return Err(ConfigError::Invalid(
                "generation.batch_size must be greater than zero".into(),
            ));
        }
        if self.generation.max_attempt_multiplier == 0 {
            return Err(ConfigError::Invalid(
                "generation.max_attempt_multiplier must be greater than zero".into(),
            ));
        }
        if self.generation.api_key_env.trim().is_empty() {
            return Err(ConfigError::Invalid(
                "generation.api_key_env must not be empty".into(),
            ));
        }
        self.row_construction_plan()?
            .validate_for_dataset(&dataset)
            .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        if self.training.artifact_root.as_os_str().is_empty() {
            return Err(ConfigError::Invalid(
                "training.artifact_root must not be empty".into(),
            ));
        }
        if self.evaluation.batch_size == 0
            || self.evaluation.top_k.is_empty()
            || self.evaluation.top_k.contains(&0)
            || self
                .evaluation
                .top_k
                .windows(2)
                .any(|values| values[0] >= values[1])
            || self.evaluation.calibration_bins < 2
            || self.evaluation.minimum_slice_support == 0
            || self.evaluation.bootstrap_samples == 0
            || !self.evaluation.confidence_level.is_finite()
            || !(0.0..1.0).contains(&self.evaluation.confidence_level)
            || self.evaluation.dimension_intersections.iter().any(|names| {
                names.is_empty()
                    || names.iter().any(|name| name.trim().is_empty())
                    || names.windows(2).any(|pair| pair[0] >= pair[1])
            })
            || self
                .evaluation
                .dimension_intersections
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(ConfigError::Invalid(
                "evaluation protocol values are invalid".into(),
            ));
        }
        if self.training.backend == TrainingBackendKind::BertCpu
            && self.training.base_model_id.is_none()
        {
            return Err(ConfigError::Invalid(
                "training.base_model_id is required for bert-cpu".into(),
            ));
        }
        if self.generation.backend == GenerationBackendKind::OpenaiCompatible {
            if self
                .generation
                .base_url
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(ConfigError::Invalid(
                    "generation.base_url is required for openai-compatible".into(),
                ));
            }
            if self.generation.model.trim().is_empty() {
                return Err(ConfigError::Invalid(
                    "generation.model is required for openai-compatible".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn dataset_definition(&self) -> Result<DatasetDefinition, ConfigError> {
        let dimensions = self
            .dataset
            .dimensions
            .iter()
            .map(|dimension| DimensionDefinition::new(&dimension.name, dimension.values.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        DatasetDefinition::new(
            &self.dataset.name,
            &self.dataset.task,
            self.dataset.labels.clone(),
            dimensions,
        )
        .map_err(Into::into)
    }

    pub fn generation_plan(
        &self,
        dataset: &DatasetDefinition,
    ) -> Result<GenerationPlan, ConfigError> {
        equal_target_plan(dataset, self.generation.target_per_cell).map_err(Into::into)
    }

    pub fn generation_parameters(&self) -> GenerationParameters {
        GenerationParameters {
            temperature: self.generation.temperature,
            max_tokens: self.generation.max_tokens,
            seed: self.generation.seed,
            extra: self.generation.extra.clone(),
        }
    }

    pub fn row_construction_plan(&self) -> Result<RowConstructionPlan, ConfigError> {
        self.generation
            .construction
            .as_ref()
            .filter(|construction| !construction.fields.is_empty())
            .map_or_else(RowConstructionPlan::llm_text_default, |construction| {
                RowConstructionPlan::new(construction.seed, construction.fields.clone())
            })
            .map_err(|error| ConfigError::Invalid(error.to_string()))
    }

    pub fn backend_configuration(&self) -> Option<BackendConfiguration> {
        (self.generation.backend == GenerationBackendKind::OpenaiCompatible).then(|| {
            BackendConfiguration {
                name: "openai-compatible".into(),
                base_url: self.generation.base_url.clone(),
                model: self.generation.model.clone(),
                parameters: self.generation_parameters(),
                updated_at: Utc::now(),
            }
        })
    }

    pub fn split_configuration(&self) -> Result<SplitConfiguration, ConfigError> {
        Ok(SplitConfiguration::new(
            SplitRatios::new(
                self.snapshot.train_ratio,
                self.snapshot.validation_ratio,
                self.snapshot.test_ratio,
            )?,
            self.snapshot.seed,
        ))
    }

    pub fn training_configuration(&self) -> Result<TrainingConfiguration, ConfigError> {
        let configuration = TrainingConfiguration {
            feature_dimension: self.training.feature_dimension,
            epochs: self.training.epochs,
            learning_rate: self.training.learning_rate,
            l2: self.training.l2,
            checkpoint_every: self.training.checkpoint_every,
            seed: self.training.seed,
        };
        configuration.validate()?;
        Ok(configuration)
    }

    pub fn transformer_configuration_fingerprint(&self) -> Result<String, ConfigError> {
        artifact_core::fingerprint(&self.training.transformer)
            .map_err(|error| ConfigError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid project TOML: {0}")]
    Parse(String),
    #[error("unsupported project configuration version: {0}")]
    Version(u32),
    #[error("invalid project configuration: {0}")]
    Invalid(String),
    #[error("could not fingerprint project configuration: {0}")]
    Fingerprint(String),
    #[error(transparent)]
    GenerationDomain(#[from] generation_core::domain::DomainError),
    #[error(transparent)]
    DatasetDomain(#[from] dataset_core::domain::DatasetError),
    #[error(transparent)]
    TrainingDomain(#[from] training_core::domain::TrainingDomainError),
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("project initialization failed: {0}")]
pub struct ProjectStoreError(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedProjectConfiguration {
    pub id: Uuid,
    pub fingerprint: String,
    pub dataset_id: Uuid,
    pub generation_plan_id: Uuid,
    pub resolved: ResolvedProjectConfig,
    pub created_at: chrono::DateTime<Utc>,
}

pub trait ProjectInitializer: Send + Sync {
    fn initialize_project(
        &self,
        dataset: &DatasetDefinition,
        plan: &GenerationPlan,
        backend: Option<&BackendConfiguration>,
        configuration: &PersistedProjectConfiguration,
    ) -> BoxFuture<'_, Result<(), ProjectStoreError>>;
}

pub trait ProjectConfigurationStore: Send + Sync {
    fn get_project_configuration(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PersistedProjectConfiguration>, ProjectStoreError>>;

    fn list_project_configurations(
        &self,
        dataset_id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<PersistedProjectConfiguration>, ProjectStoreError>>;
}

#[cfg(test)]
mod tests {
    use dataset_core::domain::SnapshotSplit;
    use generation_core::construction::FieldRecipe;
    use training_core::domain::EncoderTrainingMode;
    use uuid::Uuid;

    use super::{ProjectConfig, ProjectOverrides};

    const CONFIG: &str = r#"
version = 1

[dataset]
name = "support"
task = "Classify support requests"
labels = ["billing", "fraud"]

[[dataset.dimensions]]
name = "style"
values = ["clean", "messy"]

[generation]
target_per_cell = 25

[generation.extra.thinking]
type = "disabled"

[snapshot]
name = "baseline"
train_ratio = 0.7
validation_ratio = 0.1
test_ratio = 0.2

[training]
epochs = 12

[evaluation]
split = "test"
"#;

    #[test]
    fn parses_defaults_validates_domains_and_applies_overrides() {
        let resolved = ProjectConfig::parse(CONFIG)
            .expect("parse")
            .resolve(ProjectOverrides {
                target_per_cell: Some(40),
                training_epochs: Some(4),
                evaluation_split: Some(SnapshotSplit::Validation),
                ..ProjectOverrides::default()
            })
            .expect("resolve");
        assert_eq!(resolved.generation.target_per_cell, 40);
        assert_eq!(resolved.training.epochs, 4);
        assert_eq!(resolved.evaluation.split, SnapshotSplit::Validation);
        assert_eq!(resolved.generation.batch_size, 20);
        assert_eq!(
            resolved.generation_parameters().extra["thinking"]["type"],
            "disabled"
        );
        assert_eq!(
            resolved.dataset_definition().expect("dataset").labels.len(),
            2
        );
        let repeated = ProjectConfig::parse(CONFIG)
            .expect("parse again")
            .resolve(ProjectOverrides {
                target_per_cell: Some(40),
                training_epochs: Some(4),
                evaluation_split: Some(SnapshotSplit::Validation),
                ..ProjectOverrides::default()
            })
            .expect("resolve again");
        assert_eq!(
            resolved.fingerprint().expect("fingerprint"),
            repeated.fingerprint().expect("repeated fingerprint")
        );
    }

    #[test]
    fn rejects_unknown_secret_like_fields() {
        let source = CONFIG.replace(
            "target_per_cell = 25",
            "target_per_cell = 25\napi_key = \"must-not-be-stored\"",
        );
        assert!(ProjectConfig::parse(&source).is_err());
    }

    #[test]
    fn resolves_strict_hybrid_row_construction() {
        let source = CONFIG.replace(
            "[snapshot]",
            r#"[generation.construction]
seed = 42

[[generation.construction.fields]]
name = "ticket_id"
value_type = "string"

[generation.construction.fields.recipe]
type = "sequence"
prefix = "T-"
start = 100
step = 1
width = 5

[[generation.construction.fields]]
name = "style_copy"
value_type = "string"

[generation.construction.fields.recipe]
type = "cell_dimension"
name = "style"

[[generation.construction.fields]]
name = "text"
value_type = "string"

[generation.construction.fields.recipe]
type = "llm"
instruction = "Generate the support request for ${style_copy}."

[snapshot]"#,
        );
        let resolved = ProjectConfig::parse(&source)
            .expect("parse hybrid config")
            .resolve(ProjectOverrides::default())
            .expect("resolve hybrid config");
        let plan = resolved.row_construction_plan().expect("construction plan");

        assert_eq!(plan.seed, 42);
        assert_eq!(plan.fields.len(), 3);
        assert!(matches!(
            plan.field("text").expect("text").recipe,
            FieldRecipe::Llm { .. }
        ));
        assert!(resolved.generation.construction.is_some());
    }

    #[test]
    fn rejects_construction_cycles_and_unknown_dimensions() {
        let cycle = CONFIG.replace(
            "[snapshot]",
            r#"[generation.construction]
[[generation.construction.fields]]
name = "text"
value_type = "string"
[generation.construction.fields.recipe]
type = "template"
template = "${other}"

[[generation.construction.fields]]
name = "other"
value_type = "string"
[generation.construction.fields.recipe]
type = "template"
template = "${text}"

[snapshot]"#,
        );
        assert!(
            ProjectConfig::parse(&cycle)
                .expect("TOML parses")
                .resolve(ProjectOverrides::default())
                .is_err()
        );

        let unknown_dimension = CONFIG.replace(
            "[snapshot]",
            r#"[generation.construction]
[[generation.construction.fields]]
name = "text"
value_type = "string"
[generation.construction.fields.recipe]
type = "cell_dimension"
name = "missing"

[snapshot]"#,
        );
        assert!(
            ProjectConfig::parse(&unknown_dimension)
                .expect("TOML parses")
                .resolve(ProjectOverrides::default())
                .is_err()
        );
    }

    #[test]
    fn default_construction_stays_absent_from_serialized_legacy_configuration() {
        let resolved = ProjectConfig::parse(CONFIG)
            .expect("parse")
            .resolve(ProjectOverrides::default())
            .expect("resolve");
        assert!(resolved.generation.construction.is_none());
        assert!(
            serde_json::to_value(&resolved).expect("serialize")["generation"]
                .get("construction")
                .is_none()
        );
    }

    #[test]
    fn resolves_strict_bert_cpu_configuration() {
        let encoder_id = Uuid::from_u128(42);
        let source = CONFIG.replace(
            "[training]\nepochs = 12",
            &format!(
                "[training]\nbackend = \"bert-cpu\"\nbase_model_id = \"{encoder_id}\"\n\
                 epochs = 3\nlearning_rate = 0.00002\n\n[training.transformer]\n\
                 maximum_sequence_length = 64\nbatch_size = 4\nweight_decay = 0.02\n\
                 warmup_ratio = 0.2\ngradient_clip_norm = 0.5\nmode = \"frozen\""
            ),
        );
        let resolved = ProjectConfig::parse(&source)
            .expect("parse transformer config")
            .resolve(ProjectOverrides::default())
            .expect("resolve transformer config");

        assert_eq!(resolved.training.base_model_id, Some(encoder_id));
        assert_eq!(resolved.training.transformer.batch_size, 4);
        assert_eq!(
            resolved.training.transformer.mode,
            EncoderTrainingMode::Frozen
        );
        assert!(
            resolved
                .transformer_configuration_fingerprint()
                .expect("fingerprint")
                .starts_with("sha256:")
        );

        let unknown = source.replace("batch_size = 4", "batch_size = 4\napi_token = \"no\"");
        assert!(ProjectConfig::parse(&unknown).is_err());
    }
}
