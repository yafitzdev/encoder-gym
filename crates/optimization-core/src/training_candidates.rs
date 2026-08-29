use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use training_core::domain::{
    EncoderTrainingMode, RegisteredEncoder, TrainingCheckpoint, TrainingConfiguration, TrainingRun,
    TrainingRunState, TransformerTrainingConfiguration,
};
use uuid::Uuid;

use crate::{
    evidence::OptimizationSourceIdentity,
    protocol::{OptimizationProtocol, RecommendationKind},
};

const MAXIMUM_SPACE_CHOICES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SupportedTrainingBackend {
    HashingLinear,
    BertCpu {
        registered_encoder_id: Uuid,
        registered_encoder_fingerprint: String,
        maximum_position_embeddings: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingBaselineIdentity {
    pub training_run_id: Uuid,
    pub training_run_fingerprint: String,
    pub checkpoint_id: Uuid,
    pub checkpoint_fingerprint: String,
    pub snapshot_id: Uuid,
    pub snapshot_fingerprint: String,
    pub backend_name: String,
    pub model_format: String,
    pub base_model_id: Option<Uuid>,
    pub backend_configuration_fingerprint: Option<String>,
    pub configuration: TrainingConfiguration,
    pub transformer_configuration: Option<TransformerTrainingConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingConfigurationChoice {
    pub configuration: TrainingConfiguration,
    pub transformer_configuration: Option<TransformerTrainingConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingConfigurationSpace {
    pub baseline: TrainingBaselineIdentity,
    pub backend: SupportedTrainingBackend,
    pub choices: Vec<TrainingConfigurationChoice>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingFieldDifference {
    pub field: String,
    pub baseline_value: String,
    pub candidate_value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingConfigurationCandidate {
    pub id: String,
    pub configuration_space_fingerprint: String,
    pub baseline: TrainingBaselineIdentity,
    pub configuration: TrainingConfiguration,
    pub transformer_configuration: Option<TransformerTrainingConfiguration>,
    pub differences: Vec<TrainingFieldDifference>,
    pub rationale: String,
    pub cautions: Vec<String>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingCandidateSet {
    pub proposal_id: Uuid,
    pub source_snapshot_id: Uuid,
    pub source_snapshot_fingerprint: String,
    pub configuration_space: TrainingConfigurationSpace,
    pub candidates: Vec<TrainingConfigurationCandidate>,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl TrainingCandidateSet {
    pub fn validate(
        &self,
        proposal_id: Uuid,
        source: &OptimizationSourceIdentity,
        protocol: &OptimizationProtocol,
    ) -> Result<(), TrainingCandidateError> {
        if self.proposal_id != proposal_id
            || self.source_snapshot_id != source.snapshot_id
            || self.source_snapshot_fingerprint != source.snapshot_fingerprint
            || source.training_configuration_space_fingerprint.as_deref()
                != Some(self.configuration_space.fingerprint.as_str())
        {
            return Err(TrainingCandidateError::SourceIdentity);
        }
        self.configuration_space.validate()?;
        let expected = create_candidate_values(
            proposal_id,
            source,
            protocol,
            &self.configuration_space,
            self.created_at,
        )?;
        if expected.fingerprint != self.fingerprint || expected.candidates != self.candidates {
            return Err(TrainingCandidateError::CandidateSetFingerprint);
        }
        Ok(())
    }
}

impl TrainingConfigurationSpace {
    pub fn new(
        baseline_run: &TrainingRun,
        baseline_checkpoint: &TrainingCheckpoint,
        snapshot_fingerprint: impl Into<String>,
        registered_encoder: Option<&RegisteredEncoder>,
        mut choices: Vec<TrainingConfigurationChoice>,
    ) -> Result<Self, TrainingCandidateError> {
        if baseline_run.state != TrainingRunState::Completed
            || baseline_checkpoint.run_id != baseline_run.id
            || !baseline_checkpoint.is_final
            || baseline_checkpoint.model_format != baseline_run.model_format
        {
            return Err(TrainingCandidateError::BaselineIdentity);
        }
        baseline_run.configuration.validate()?;
        if let Some(configuration) = &baseline_run.transformer_configuration {
            configuration.validate()?;
        }
        let snapshot_fingerprint = snapshot_fingerprint.into();
        if snapshot_fingerprint.trim().is_empty() {
            return Err(TrainingCandidateError::SnapshotFingerprint);
        }
        if choices.is_empty() || choices.len() > MAXIMUM_SPACE_CHOICES {
            return Err(TrainingCandidateError::SpaceSize);
        }
        let backend = supported_backend(baseline_run, registered_encoder)?;
        let baseline_choice = TrainingConfigurationChoice {
            configuration: baseline_run.configuration.clone(),
            transformer_configuration: baseline_run.transformer_configuration.clone(),
        };
        for choice in &choices {
            validate_choice(choice, &backend)?;
            if choice == &baseline_choice {
                return Err(TrainingCandidateError::UnchangedChoice);
            }
        }
        choices.sort_by_cached_key(|choice| artifact_core::fingerprint(choice).unwrap_or_default());
        if choices.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(TrainingCandidateError::DuplicateChoice);
        }
        let baseline = TrainingBaselineIdentity {
            training_run_id: baseline_run.id,
            training_run_fingerprint: artifact_core::fingerprint(baseline_run)?,
            checkpoint_id: baseline_checkpoint.id,
            checkpoint_fingerprint: artifact_core::fingerprint(baseline_checkpoint)?,
            snapshot_id: baseline_run.snapshot_id,
            snapshot_fingerprint,
            backend_name: baseline_run.backend_name.clone(),
            model_format: baseline_run.model_format.clone(),
            base_model_id: baseline_run.base_model_id,
            backend_configuration_fingerprint: baseline_run
                .backend_configuration_fingerprint
                .clone(),
            configuration: baseline_run.configuration.clone(),
            transformer_configuration: baseline_run.transformer_configuration.clone(),
        };
        let mut space = Self {
            baseline,
            backend,
            choices,
            fingerprint: String::new(),
        };
        space.fingerprint = space.reproduce_fingerprint()?;
        Ok(space)
    }

    pub fn validate(&self) -> Result<(), TrainingCandidateError> {
        if self.choices.is_empty() || self.choices.len() > MAXIMUM_SPACE_CHOICES {
            return Err(TrainingCandidateError::SpaceSize);
        }
        let baseline_choice = TrainingConfigurationChoice {
            configuration: self.baseline.configuration.clone(),
            transformer_configuration: self.baseline.transformer_configuration.clone(),
        };
        let mut previous = None;
        for choice in &self.choices {
            validate_choice(choice, &self.backend)?;
            if choice == &baseline_choice {
                return Err(TrainingCandidateError::UnchangedChoice);
            }
            let fingerprint = artifact_core::fingerprint(choice)?;
            if previous.as_ref().is_some_and(|value| value >= &fingerprint) {
                return Err(TrainingCandidateError::ChoiceOrder);
            }
            previous = Some(fingerprint);
        }
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(TrainingCandidateError::SpaceFingerprint);
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, artifact_core::FingerprintError> {
        let mut input = self.clone();
        input.fingerprint.clear();
        artifact_core::fingerprint(&input)
    }
}

pub fn create_training_candidate_set(
    proposal_id: Uuid,
    source: &OptimizationSourceIdentity,
    protocol: &OptimizationProtocol,
    space: &TrainingConfigurationSpace,
) -> Result<TrainingCandidateSet, TrainingCandidateError> {
    create_candidate_values(proposal_id, source, protocol, space, Utc::now())
}

fn create_candidate_values(
    proposal_id: Uuid,
    source: &OptimizationSourceIdentity,
    protocol: &OptimizationProtocol,
    space: &TrainingConfigurationSpace,
    created_at: DateTime<Utc>,
) -> Result<TrainingCandidateSet, TrainingCandidateError> {
    protocol.validate()?;
    space.validate()?;
    let request = protocol
        .training_candidates
        .as_ref()
        .ok_or(TrainingCandidateError::NotRequested)?;
    if !protocol
        .recommendation_kinds
        .contains(&RecommendationKind::TrainingConfiguration)
    {
        return Err(TrainingCandidateError::NotRequested);
    }
    if source.snapshot_id != space.baseline.snapshot_id
        || source.snapshot_fingerprint != space.baseline.snapshot_fingerprint
        || source.training_configuration_space_fingerprint.as_deref()
            != Some(space.fingerprint.as_str())
    {
        return Err(TrainingCandidateError::SourceIdentity);
    }
    let mut ranked = space
        .choices
        .iter()
        .map(|choice| {
            let differences = differences(&space.baseline, choice);
            let choice_fingerprint = artifact_core::fingerprint(choice)?;
            Ok((differences.len(), choice_fingerprint, choice, differences))
        })
        .collect::<Result<Vec<_>, artifact_core::FingerprintError>>()?;
    ranked.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let candidates = ranked
        .into_iter()
        .take(usize::from(request.maximum_candidates))
        .map(|(_, _, choice, differences)| {
            let changed_fields = differences
                .iter()
                .map(|difference| difference.field.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let rationale = format!(
                "bounded experiment changing {changed_fields} from the persisted baseline; no optimality is inferred"
            );
            let cautions = vec![
                "candidate performance is unknown until a separately started run is evaluated".into(),
                "compare on the same immutable cohort and evaluation protocol".into(),
            ];
            let fingerprint = artifact_core::fingerprint(&CandidateFingerprintInput {
                configuration_space_fingerprint: &space.fingerprint,
                baseline: &space.baseline,
                configuration: &choice.configuration,
                transformer_configuration: choice.transformer_configuration.as_ref(),
                differences: &differences,
                rationale: &rationale,
                cautions: &cautions,
            })?;
            Ok(TrainingConfigurationCandidate {
                id: fingerprint.clone(),
                configuration_space_fingerprint: space.fingerprint.clone(),
                baseline: space.baseline.clone(),
                configuration: choice.configuration.clone(),
                transformer_configuration: choice.transformer_configuration.clone(),
                differences,
                rationale,
                cautions,
                fingerprint,
            })
        })
        .collect::<Result<Vec<_>, artifact_core::FingerprintError>>()?;
    let fingerprint = artifact_core::fingerprint(&CandidateSetFingerprintInput {
        proposal_id,
        source_snapshot_id: source.snapshot_id,
        source_snapshot_fingerprint: &source.snapshot_fingerprint,
        configuration_space_fingerprint: &space.fingerprint,
        candidate_fingerprints: candidates
            .iter()
            .map(|candidate| candidate.fingerprint.as_str())
            .collect::<Vec<_>>(),
        created_at,
    })?;
    Ok(TrainingCandidateSet {
        proposal_id,
        source_snapshot_id: source.snapshot_id,
        source_snapshot_fingerprint: source.snapshot_fingerprint.clone(),
        configuration_space: space.clone(),
        candidates,
        fingerprint,
        created_at,
    })
}

fn supported_backend(
    run: &TrainingRun,
    encoder: Option<&RegisteredEncoder>,
) -> Result<SupportedTrainingBackend, TrainingCandidateError> {
    match (run.backend_name.as_str(), run.model_format.as_str()) {
        ("hashing-linear", "hashing-linear-v1")
            if run.base_model_id.is_none()
                && run.transformer_configuration.is_none()
                && run.backend_configuration_fingerprint.is_none()
                && encoder.is_none() =>
        {
            Ok(SupportedTrainingBackend::HashingLinear)
        }
        ("bert-cpu", "bert-classifier-v1") => {
            let encoder = encoder.ok_or(TrainingCandidateError::BackendIdentity)?;
            encoder.validate()?;
            let transformer = run
                .transformer_configuration
                .as_ref()
                .ok_or(TrainingCandidateError::BackendIdentity)?;
            if run.base_model_id != Some(encoder.id)
                || run.backend_configuration_fingerprint.as_deref()
                    != Some(artifact_core::fingerprint(transformer)?.as_str())
            {
                return Err(TrainingCandidateError::BackendIdentity);
            }
            Ok(SupportedTrainingBackend::BertCpu {
                registered_encoder_id: encoder.id,
                registered_encoder_fingerprint: encoder.fingerprint.clone(),
                maximum_position_embeddings: encoder.metadata.maximum_position_embeddings,
            })
        }
        _ => Err(TrainingCandidateError::UnsupportedBackend),
    }
}

fn validate_choice(
    choice: &TrainingConfigurationChoice,
    backend: &SupportedTrainingBackend,
) -> Result<(), TrainingCandidateError> {
    choice.configuration.validate()?;
    match (backend, &choice.transformer_configuration) {
        (SupportedTrainingBackend::HashingLinear, None) => Ok(()),
        (
            SupportedTrainingBackend::BertCpu {
                maximum_position_embeddings,
                ..
            },
            Some(configuration),
        ) => {
            configuration.validate()?;
            if configuration.maximum_sequence_length > *maximum_position_embeddings {
                return Err(TrainingCandidateError::SequenceLengthLimit);
            }
            Ok(())
        }
        _ => Err(TrainingCandidateError::BackendConfiguration),
    }
}

fn differences(
    baseline: &TrainingBaselineIdentity,
    candidate: &TrainingConfigurationChoice,
) -> Vec<TrainingFieldDifference> {
    let mut differences = Vec::new();
    diff(
        &mut differences,
        "feature_dimension",
        baseline.configuration.feature_dimension,
        candidate.configuration.feature_dimension,
    );
    diff(
        &mut differences,
        "epochs",
        baseline.configuration.epochs,
        candidate.configuration.epochs,
    );
    diff(
        &mut differences,
        "learning_rate",
        baseline.configuration.learning_rate,
        candidate.configuration.learning_rate,
    );
    diff(
        &mut differences,
        "l2",
        baseline.configuration.l2,
        candidate.configuration.l2,
    );
    diff(
        &mut differences,
        "checkpoint_every",
        baseline.configuration.checkpoint_every,
        candidate.configuration.checkpoint_every,
    );
    diff(
        &mut differences,
        "seed",
        baseline.configuration.seed,
        candidate.configuration.seed,
    );
    if let (Some(baseline), Some(candidate)) = (
        baseline.transformer_configuration.as_ref(),
        candidate.transformer_configuration.as_ref(),
    ) {
        diff(
            &mut differences,
            "maximum_sequence_length",
            baseline.maximum_sequence_length,
            candidate.maximum_sequence_length,
        );
        diff(
            &mut differences,
            "batch_size",
            baseline.batch_size,
            candidate.batch_size,
        );
        diff(
            &mut differences,
            "weight_decay",
            baseline.weight_decay,
            candidate.weight_decay,
        );
        diff(
            &mut differences,
            "warmup_ratio",
            baseline.warmup_ratio,
            candidate.warmup_ratio,
        );
        diff(
            &mut differences,
            "gradient_clip_norm",
            baseline.gradient_clip_norm,
            candidate.gradient_clip_norm,
        );
        if baseline.mode != candidate.mode {
            differences.push(TrainingFieldDifference {
                field: "mode".into(),
                baseline_value: mode_name(baseline.mode).into(),
                candidate_value: mode_name(candidate.mode).into(),
            });
        }
    }
    differences
}

fn diff<T: PartialEq + ToString>(
    differences: &mut Vec<TrainingFieldDifference>,
    field: &str,
    baseline: T,
    candidate: T,
) {
    if baseline != candidate {
        differences.push(TrainingFieldDifference {
            field: field.into(),
            baseline_value: baseline.to_string(),
            candidate_value: candidate.to_string(),
        });
    }
}

const fn mode_name(mode: EncoderTrainingMode) -> &'static str {
    match mode {
        EncoderTrainingMode::FineTune => "fine_tune",
        EncoderTrainingMode::Frozen => "frozen",
    }
}

#[derive(Serialize)]
struct CandidateFingerprintInput<'a> {
    configuration_space_fingerprint: &'a str,
    baseline: &'a TrainingBaselineIdentity,
    configuration: &'a TrainingConfiguration,
    transformer_configuration: Option<&'a TransformerTrainingConfiguration>,
    differences: &'a [TrainingFieldDifference],
    rationale: &'a str,
    cautions: &'a [String],
}

#[derive(Serialize)]
struct CandidateSetFingerprintInput<'a> {
    proposal_id: Uuid,
    source_snapshot_id: Uuid,
    source_snapshot_fingerprint: &'a str,
    configuration_space_fingerprint: &'a str,
    candidate_fingerprints: Vec<&'a str>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum TrainingCandidateError {
    #[error(transparent)]
    Training(#[from] training_core::domain::TrainingDomainError),
    #[error(transparent)]
    Protocol(#[from] crate::protocol::OptimizationProtocolError),
    #[error(transparent)]
    Fingerprint(#[from] artifact_core::FingerprintError),
    #[error("baseline must be a completed run with its final compatible checkpoint")]
    BaselineIdentity,
    #[error("baseline snapshot fingerprint must not be empty")]
    SnapshotFingerprint,
    #[error("configuration space must contain between 1 and 256 explicit choices")]
    SpaceSize,
    #[error("configuration space repeats an explicit choice")]
    DuplicateChoice,
    #[error("configuration choices must be in canonical fingerprint order")]
    ChoiceOrder,
    #[error("a candidate choice must differ from the baseline")]
    UnchangedChoice,
    #[error("baseline backend identity is incomplete or mismatched")]
    BackendIdentity,
    #[error("baseline training backend is not supported for candidate generation")]
    UnsupportedBackend,
    #[error("candidate configuration is not supported by the selected backend")]
    BackendConfiguration,
    #[error("candidate sequence length exceeds the registered encoder limit")]
    SequenceLengthLimit,
    #[error("training candidates were not requested by the optimization protocol")]
    NotRequested,
    #[error("training configuration space does not match optimization source identity")]
    SourceIdentity,
    #[error("training configuration-space fingerprint does not reproduce")]
    SpaceFingerprint,
    #[error("training candidate-set fingerprint or contents do not reproduce")]
    CandidateSetFingerprint,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use training_core::domain::{
        TrainingCheckpoint, TrainingConfiguration, TrainingRun, TrainingRunState,
    };
    use uuid::Uuid;

    use crate::{
        evidence::OptimizationSourceIdentity,
        protocol::{OptimizationProtocol, RecommendationKind, TrainingCandidateRequest},
    };

    use super::{
        TrainingConfigurationChoice, TrainingConfigurationSpace, create_training_candidate_set,
    };

    #[test]
    fn creates_bounded_typed_candidates_from_a_completed_baseline() {
        let snapshot_id = Uuid::new_v4();
        let mut run = TrainingRun::queued(
            snapshot_id,
            "hashing-linear",
            "hashing-linear-v1",
            TrainingConfiguration::default(),
        )
        .expect("baseline run");
        run.transition(TrainingRunState::Running).expect("running");
        run.transition(TrainingRunState::Completed)
            .expect("completed");
        let checkpoint = TrainingCheckpoint {
            id: Uuid::new_v4(),
            run_id: run.id,
            epoch: run.configuration.epochs,
            artifact_path: "artifacts/model.bin".into(),
            artifact_checksum: "sha256:checkpoint".into(),
            artifact_size_bytes: 42,
            model_format: run.model_format.clone(),
            training_loss: 0.2,
            validation_loss: Some(0.3),
            is_final: true,
            created_at: Utc::now(),
        };
        let mut lower_rate = run.configuration.clone();
        lower_rate.learning_rate = 0.05;
        let mut more_epochs = run.configuration.clone();
        more_epochs.epochs += 5;
        more_epochs.l2 = 0.001;
        let space = TrainingConfigurationSpace::new(
            &run,
            &checkpoint,
            "sha256:snapshot",
            None,
            vec![
                TrainingConfigurationChoice {
                    configuration: more_epochs,
                    transformer_configuration: None,
                },
                TrainingConfigurationChoice {
                    configuration: lower_rate,
                    transformer_configuration: None,
                },
            ],
        )
        .expect("finite supported space");
        let mut protocol = OptimizationProtocol::legacy(10, 1);
        protocol.recommendation_kinds = vec![
            RecommendationKind::DataGeneration,
            RecommendationKind::TrainingConfiguration,
        ];
        protocol.training_candidates = Some(TrainingCandidateRequest {
            maximum_candidates: 1,
        });
        let protocol = protocol.normalize().expect("protocol");
        let source = OptimizationSourceIdentity {
            analysis_report_id: Uuid::new_v4(),
            analysis_fingerprint: "sha256:analysis".into(),
            analysis_protocol_fingerprint: "sha256:analysis-protocol".into(),
            diagnostic_contract_fingerprint: "sha256:diagnostics".into(),
            evaluation_run_id: Uuid::new_v4(),
            evaluation_input_fingerprint: "sha256:evaluation".into(),
            evaluation_protocol_fingerprint: "sha256:evaluation-protocol".into(),
            cohort_fingerprint: "sha256:cohort".into(),
            dataset_id: Uuid::new_v4(),
            dataset_fingerprint: "sha256:dataset".into(),
            snapshot_id,
            snapshot_fingerprint: "sha256:snapshot".into(),
            coverage_fingerprint: "sha256:coverage".into(),
            comparison_id: None,
            comparison_fingerprint: None,
            optimization_protocol_fingerprint: protocol.fingerprint().expect("fingerprint"),
            training_configuration_space_fingerprint: Some(space.fingerprint.clone()),
        };
        let proposal_id = Uuid::new_v4();
        let set = create_training_candidate_set(proposal_id, &source, &protocol, &space)
            .expect("candidate set");
        assert_eq!(set.candidates.len(), 1);
        assert_eq!(set.candidates[0].differences.len(), 1);
        assert_eq!(set.candidates[0].differences[0].field, "learning_rate");
        set.validate(proposal_id, &source, &protocol)
            .expect("candidate set verifies");
    }
}
