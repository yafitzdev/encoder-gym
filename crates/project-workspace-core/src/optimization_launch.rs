//! Finite authority for one input-first Optimize action.
//!
//! This record pins user-selected inputs and non-secret provider settings. It
//! neither contains credentials nor performs agent, generation, training, or
//! evaluation work.
use crate::{
    BoundIdentity, Invalid, OptimizationAgentSettings, OptimizationSetup, ProjectBenchmarkVersion,
    ProviderCatalog, ProviderLimits, ProviderRole, require, validate_name,
};
use chrono::{DateTime, Utc};
use encoder_experiment_core::domain::EvidenceRole;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_MAXIMUM_ITERATIONS: u32 = 3;
pub const DEFAULT_MAXIMUM_MODELS: u32 = 3;
pub const DEFAULT_MAXIMUM_DATASET_ROW_CHANGES: u64 = 5_000;
pub const DEFAULT_MAXIMUM_TRAINING_SECONDS: u64 = 21_600;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationExecutionLimits {
    pub maximum_iterations: u32,
    pub maximum_models: u32,
    pub maximum_dataset_row_changes: u64,
    pub maximum_training_seconds: u64,
    pub maximum_development_evaluations: u32,
    pub maximum_final_evaluations: u32,
}

impl OptimizationExecutionLimits {
    fn defaults(development_suites: u32) -> Result<Self, Invalid> {
        let value = Self {
            maximum_iterations: DEFAULT_MAXIMUM_ITERATIONS,
            maximum_models: DEFAULT_MAXIMUM_MODELS,
            maximum_dataset_row_changes: DEFAULT_MAXIMUM_DATASET_ROW_CHANGES,
            maximum_training_seconds: DEFAULT_MAXIMUM_TRAINING_SECONDS,
            maximum_development_evaluations: DEFAULT_MAXIMUM_MODELS
                .checked_mul(development_suites)
                .ok_or_else(|| Invalid("Evaluation limit overflow.".into()))?,
            maximum_final_evaluations: 1,
        };
        value.validate(development_suites)?;
        Ok(value)
    }

    fn validate(&self, development_suites: u32) -> Result<(), Invalid> {
        require(
            (1..=10).contains(&self.maximum_iterations)
                && self.maximum_models >= self.maximum_iterations
                && self.maximum_models <= 20
                && (1..=1_000_000).contains(&self.maximum_dataset_row_changes)
                && (1..=604_800).contains(&self.maximum_training_seconds)
                && self.maximum_development_evaluations
                    == self
                        .maximum_models
                        .checked_mul(development_suites)
                        .unwrap_or(0)
                && self.maximum_final_evaluations <= 1,
            "Optimization limits must match the finite default execution envelope.",
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalEvaluationAuthorization {
    SelectedCandidateOnce,
    DevelopmentOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationLaunchScope {
    pub project_id: Uuid,
    pub setup: BoundIdentity,
    pub provider_catalog: BoundIdentity,
    pub limits: OptimizationExecutionLimits,
    pub generation: ProviderLimits,
    pub advisor: ProviderLimits,
    pub final_evaluation: FinalEvaluationAuthorization,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agentic: Option<OptimizationAgentSettings>,
    pub fingerprint: String,
}

impl OptimizationLaunchScope {
    pub fn bind(
        setup: &OptimizationSetup,
        providers: &ProviderCatalog,
        benchmark: &ProjectBenchmarkVersion,
    ) -> Result<Self, Invalid> {
        providers.validate()?;
        verify_setup_identity(setup)?;
        verify_benchmark_identity(benchmark)?;
        require(
            setup.inputs.project_id == providers.project_id
                && setup.inputs.project_id == benchmark.project_id
                && setup.inputs.benchmark.id == benchmark.id.to_string()
                && setup.inputs.benchmark.fingerprint == benchmark.fingerprint,
            "Optimization setup, providers, and benchmark must belong to the same project.",
        )?;
        let development_suites = u32::try_from(
            benchmark
                .definition
                .suites
                .iter()
                .filter(|suite| suite.role == EvidenceRole::Development)
                .count(),
        )
        .map_err(|_| Invalid("Too many development evaluation suites.".into()))?;
        let final_suites = benchmark
            .definition
            .suites
            .iter()
            .filter(|suite| suite.role == EvidenceRole::SealedAcceptance)
            .count();
        require(
            development_suites > 0 && final_suites == 1,
            "Automatic optimization requires development tests and one final holdout.",
        )?;
        let generation = providers
            .provider(ProviderRole::Generation)
            .ok_or_else(|| Invalid("Configure a data-generation provider.".into()))?
            .limits
            .clone();
        let advisor = providers
            .provider(ProviderRole::Advisor)
            .ok_or_else(|| Invalid("Configure an agentic-work provider.".into()))?
            .limits
            .clone();
        let mut value = Self {
            project_id: setup.inputs.project_id,
            setup: BoundIdentity {
                id: setup.id.to_string(),
                fingerprint: setup.fingerprint.clone(),
            },
            provider_catalog: BoundIdentity {
                id: providers.id.to_string(),
                fingerprint: providers.fingerprint.clone(),
            },
            limits: OptimizationExecutionLimits::defaults(development_suites)?,
            generation,
            advisor,
            final_evaluation: FinalEvaluationAuthorization::SelectedCandidateOnce,
            agentic: None,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate(setup, providers, benchmark)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        let mut value = serde_json::json!({
            "projectId":self.project_id,"setup":self.setup,
            "providerCatalog":self.provider_catalog,"limits":self.limits,
            "generation":self.generation,"advisor":self.advisor,
            "finalEvaluation":self.final_evaluation,
        });
        // Omitted for old authorities: historical fingerprints must not change.
        if let Some(settings) = &self.agentic {
            value["agentic"] =
                serde_json::to_value(settings).map_err(|error| Invalid(error.to_string()))?;
        }
        artifact_core::fingerprint(&value).map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate_identity(&self) -> Result<(), Invalid> {
        self.setup.validate("Optimization setup")?;
        self.provider_catalog.validate("Provider catalog")?;
        self.generation.validate()?;
        self.advisor.validate()?;
        require(
            !self.project_id.is_nil()
                && self.limits.maximum_models > 0
                && self.limits.maximum_development_evaluations % self.limits.maximum_models == 0,
            "Optimization launch scope is invalid.",
        )?;
        let development_suites =
            self.limits.maximum_development_evaluations / self.limits.maximum_models;
        require(
            development_suites > 0 && self.reproduce()? == self.fingerprint,
            "Optimization launch scope changed or is invalid.",
        )?;
        self.limits.validate(development_suites)?;
        match &self.agentic {
            Some(settings) => {
                settings.validate()?;
                if let Some(limits) = &settings.provider_limits {
                    require(
                        self.advisor == limits.advisor && self.generation == limits.generation,
                        "Provider ceilings do not match the pinned Agent settings.",
                    )?;
                }
                require(
                    self.limits == settings.execution_limits(development_suites)?
                        && self.final_evaluation == settings.final_authorization(),
                    "Execution limits do not match the pinned agent settings.",
                )
            }
            None => require(
                self.final_evaluation == FinalEvaluationAuthorization::SelectedCandidateOnce
                    && self.limits.maximum_final_evaluations == 1,
                "Legacy launches require their original final-evaluation policy.",
            ),
        }
    }

    pub fn validate(
        &self,
        setup: &OptimizationSetup,
        providers: &ProviderCatalog,
        benchmark: &ProjectBenchmarkVersion,
    ) -> Result<(), Invalid> {
        verify_setup_identity(setup)?;
        providers.validate()?;
        verify_benchmark_identity(benchmark)?;
        self.validate_identity()?;
        let development_suites = u32::try_from(
            benchmark
                .definition
                .suites
                .iter()
                .filter(|suite| suite.role == EvidenceRole::Development)
                .count(),
        )
        .map_err(|_| Invalid("Too many development evaluation suites.".into()))?;
        self.limits.validate(development_suites)?;
        let generation = &providers
            .provider(ProviderRole::Generation)
            .ok_or_else(|| Invalid("Generation provider is missing.".into()))?
            .limits;
        let advisor = &providers
            .provider(ProviderRole::Advisor)
            .ok_or_else(|| Invalid("Advisor provider is missing.".into()))?
            .limits;
        if let Some(limits) = self
            .agentic
            .as_ref()
            .and_then(|settings| settings.provider_limits.as_ref())
        {
            limits.validate_within(advisor, generation)?;
        } else {
            require(
                self.generation == *generation && self.advisor == *advisor,
                "Provider ceilings changed from the pinned project limits.",
            )?;
        }
        require(
            self.project_id == setup.inputs.project_id
                && self.project_id == providers.project_id
                && self.project_id == benchmark.project_id
                && self.setup.id == setup.id.to_string()
                && self.setup.fingerprint == setup.fingerprint
                && self.provider_catalog.id == providers.id.to_string()
                && self.provider_catalog.fingerprint == providers.fingerprint
                && setup.inputs.benchmark.id == benchmark.id.to_string()
                && setup.inputs.benchmark.fingerprint == benchmark.fingerprint
                && benchmark
                    .definition
                    .suites
                    .iter()
                    .filter(|suite| suite.role == EvidenceRole::SealedAcceptance)
                    .count()
                    == 1
                && self.reproduce()? == self.fingerprint,
            "Optimization launch scope changed or does not match its exact inputs.",
        )
    }

    /// Bind settings into a new immutable authority without changing old runs.
    pub fn with_agentic_settings(
        mut self,
        settings: OptimizationAgentSettings,
    ) -> Result<Self, Invalid> {
        self.validate_identity()?;
        settings.validate()?;
        if let Some(limits) = &settings.provider_limits {
            limits.validate_within(&self.advisor, &self.generation)?;
            self.advisor = limits.advisor.clone();
            self.generation = limits.generation.clone();
        }
        let development_suites =
            self.limits.maximum_development_evaluations / self.limits.maximum_models;
        self.limits = settings.execution_limits(development_suites)?;
        self.final_evaluation = settings.final_authorization();
        self.agentic = Some(settings);
        self.fingerprint = self.reproduce()?;
        self.validate_identity()?;
        Ok(self)
    }
}

impl OptimizationAgentSettings {
    fn execution_limits(
        &self,
        development_suites: u32,
    ) -> Result<OptimizationExecutionLimits, Invalid> {
        Ok(OptimizationExecutionLimits {
            maximum_iterations: self.maximum_iterations,
            maximum_models: self.maximum_iterations,
            maximum_dataset_row_changes: u64::from(self.maximum_row_changes),
            maximum_training_seconds: u64::from(self.training.maximum_seconds_per_iteration)
                * u64::from(self.maximum_iterations),
            maximum_development_evaluations: self
                .maximum_iterations
                .checked_mul(development_suites)
                .ok_or_else(|| Invalid("Evaluation limit overflow.".into()))?,
            maximum_final_evaluations: u32::from(self.permits_final_evaluation()),
        })
    }

    fn final_authorization(&self) -> FinalEvaluationAuthorization {
        if self.permits_final_evaluation() {
            FinalEvaluationAuthorization::SelectedCandidateOnce
        } else {
            FinalEvaluationAuthorization::DevelopmentOnly
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationLaunchAuthorization {
    pub id: Uuid,
    pub scope: OptimizationLaunchScope,
    pub authorized_by: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl OptimizationLaunchAuthorization {
    pub fn create(
        id: Uuid,
        scope: OptimizationLaunchScope,
        authorized_by: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            scope,
            authorized_by: authorized_by.into(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.scope.validate_identity()?;
        value.validate_identity()?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "id":self.id,"scope":self.scope,"authorizedBy":self.authorized_by,
            "createdAt":self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate_identity(&self) -> Result<(), Invalid> {
        validate_name(&self.authorized_by)?;
        require(
            !self.id.is_nil()
                && self.authorized_by.trim() == self.authorized_by
                && self.reproduce()? == self.fingerprint,
            "Optimization launch authorization changed or is invalid.",
        )
    }

    pub fn validate(
        &self,
        setup: &OptimizationSetup,
        providers: &ProviderCatalog,
        benchmark: &ProjectBenchmarkVersion,
    ) -> Result<(), Invalid> {
        self.validate_identity()?;
        self.scope.validate(setup, providers, benchmark)
    }
}

fn verify_setup_identity(setup: &OptimizationSetup) -> Result<(), Invalid> {
    setup.inputs.validate()?;
    require(
        !setup.id.is_nil() && setup.number > 0 && setup.reproduce()? == setup.fingerprint,
        "Optimization setup identity changed.",
    )
}

fn verify_benchmark_identity(benchmark: &ProjectBenchmarkVersion) -> Result<(), Invalid> {
    benchmark
        .definition
        .validate_integrity()
        .map_err(|error| Invalid(error.to_string()))?;
    benchmark.source.validate()?;
    require(
        !benchmark.id.is_nil()
            && benchmark.number > 0
            && benchmark.reproduce()? == benchmark.fingerprint,
        "Benchmark version identity changed.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BenchmarkSource, OptimizationInputs, ProviderAuthentication, ProviderConfiguration,
        ProviderKind, SecretReference,
    };
    use dataset_core::versions::DatasetVersionRef;
    use encoder_experiment_core::{
        benchmark::{BenchmarkDefinition, BenchmarkSuite},
        domain::{BackendIdentity, EncoderTaskKind},
        metrics::{
            MetricContract, MetricDefinition, MetricDirection, MetricGate, MetricGateCondition,
        },
    };

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn fixture() -> (OptimizationSetup, ProviderCatalog, ProjectBenchmarkVersion) {
        let project_id = Uuid::new_v4();
        let metric = MetricContract::create(
            vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
            "mrr",
            vec![
                MetricGate::new(
                    "mrr",
                    EvidenceRole::Development,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "mrr",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let mut definition = BenchmarkDefinition {
            schema_version: 1,
            task: EncoderTaskKind::RetrievalRanking,
            backend: BackendIdentity {
                name: "fixture".into(),
                protocol_version: "v1".into(),
                configuration_fingerprint: digest('1'),
            },
            source_revision: "revision-1".into(),
            evaluation_configuration_fingerprint: digest('2'),
            metric_contract: metric,
            suites: vec![
                BenchmarkSuite {
                    key: "development-a".into(),
                    role: EvidenceRole::Development,
                    fingerprint: digest('3'),
                    support: 10,
                },
                BenchmarkSuite {
                    key: "development-b".into(),
                    role: EvidenceRole::Development,
                    fingerprint: digest('4'),
                    support: 10,
                },
                BenchmarkSuite {
                    key: "final".into(),
                    role: EvidenceRole::SealedAcceptance,
                    fingerprint: digest('5'),
                    support: 10,
                },
            ],
            fingerprint: String::new(),
        };
        definition.fingerprint = definition.reproduce_fingerprint().unwrap();
        let reference = |character| BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: digest(character),
        };
        let benchmark = ProjectBenchmarkVersion::create(
            Uuid::new_v4(),
            project_id,
            None,
            definition,
            BenchmarkSource {
                scientific_binding: reference('6'),
                project_snapshot: reference('7'),
                protocol: reference('8'),
            },
            Utc::now(),
        )
        .unwrap();
        let setup = OptimizationSetup::create(
            Uuid::new_v4(),
            None,
            OptimizationInputs {
                project_id,
                baseline_revision: reference('9'),
                model: reference('a'),
                dataset: DatasetVersionRef {
                    id: Uuid::new_v4(),
                    dataset_id: Uuid::new_v4(),
                    project_id,
                    number: 1,
                    fingerprint: digest('b'),
                },
                benchmark: BoundIdentity {
                    id: benchmark.id.to_string(),
                    fingerprint: benchmark.fingerprint.clone(),
                },
            },
            Utc::now(),
        )
        .unwrap();
        let limits = ProviderLimits {
            maximum_requests: 5,
            maximum_input_tokens: 10_000,
            maximum_output_tokens: 2_000,
            maximum_cost_microusd: 0,
        };
        let provider = |role| ProviderConfiguration {
            role,
            kind: ProviderKind::Fake,
            endpoint: None,
            model: "fixture-provider".into(),
            authentication: ProviderAuthentication::None,
            secret: None::<SecretReference>,
            limits: limits.clone(),
        };
        let providers = ProviderCatalog::create(
            Uuid::new_v4(),
            project_id,
            1,
            None,
            vec![
                provider(ProviderRole::Generation),
                provider(ProviderRole::Advisor),
            ],
            "operator",
            "configure providers",
            Utc::now(),
        )
        .unwrap();
        (setup, providers, benchmark)
    }

    #[test]
    fn agent_settings_bind_limits_without_rewriting_legacy_fingerprints() {
        let (setup, providers, benchmark) = fixture();
        let legacy = OptimizationLaunchScope::bind(&setup, &providers, &benchmark).unwrap();
        let json = serde_json::to_value(&legacy).unwrap();
        assert!(json.get("agentic").is_none());
        assert_eq!(
            serde_json::from_value::<OptimizationLaunchScope>(json)
                .unwrap()
                .reproduce()
                .unwrap(),
            legacy.fingerprint
        );
        let settings = OptimizationAgentSettings::quick_test();
        let quick = legacy.clone().with_agentic_settings(settings).unwrap();
        quick.validate(&setup, &providers, &benchmark).unwrap();
        assert_eq!(quick.limits.maximum_iterations, 1);
        assert_eq!(quick.limits.maximum_training_seconds, 120);
        assert_eq!(quick.limits.maximum_development_evaluations, 2);
        assert_eq!(quick.limits.maximum_final_evaluations, 0);
        assert_eq!(
            quick.final_evaluation,
            FinalEvaluationAuthorization::DevelopmentOnly
        );
        assert_ne!(quick.fingerprint, legacy.fingerprint);
        let mut forged = quick.clone();
        forged.limits.maximum_iterations = 2;
        forged.fingerprint = forged.reproduce().unwrap();
        assert!(forged.validate_identity().is_err());
        forged = quick;
        forged.final_evaluation = FinalEvaluationAuthorization::SelectedCandidateOnce;
        forged.limits.maximum_final_evaluations = 1;
        forged.fingerprint = forged.reproduce().unwrap();
        assert!(forged.validate_identity().is_err());
        let mut invalid = legacy;
        invalid.limits.maximum_models = 0;
        assert!(
            invalid
                .with_agentic_settings(OptimizationAgentSettings::default())
                .is_err()
        );
    }

    #[test]
    fn run_provider_ceilings_are_explicit_bounded_and_fingerprint_compatible() {
        let (setup, providers, benchmark) = fixture();
        let original = OptimizationLaunchScope::bind(&setup, &providers, &benchmark)
            .unwrap()
            .with_agentic_settings(OptimizationAgentSettings::default())
            .unwrap();
        let bytes = serde_json::to_value(&original).unwrap();
        assert!(bytes["agentic"].get("providerLimits").is_none());
        let restored: OptimizationLaunchScope = serde_json::from_value(bytes.clone()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), bytes);
        assert_eq!(restored.reproduce().unwrap(), original.fingerprint);
        let lower = ProviderLimits {
            maximum_requests: 2,
            maximum_input_tokens: 5_000,
            maximum_output_tokens: 500,
            maximum_cost_microusd: 0,
        };
        let settings = OptimizationAgentSettings {
            provider_limits: Some(crate::OptimizationProviderLimits {
                advisor: lower.clone(),
                generation: lower.clone(),
            }),
            ..OptimizationAgentSettings::default()
        };
        let limited = original
            .clone()
            .with_agentic_settings(settings.clone())
            .unwrap();
        limited.validate(&setup, &providers, &benchmark).unwrap();
        assert_eq!(limited.advisor, lower);
        assert_eq!(limited.generation, lower);
        assert_ne!(limited.fingerprint, original.fingerprint);
        for role in ["advisor", "generation"] {
            for (key, value) in [
                ("maximumRequests", 6),
                ("maximumInputTokens", 10_001),
                ("maximumOutputTokens", 2_001),
                ("maximumCostMicrousd", 1),
            ] {
                let mut changed = serde_json::to_value(&settings).unwrap();
                changed["providerLimits"][role][key] = value.into();
                let changed = serde_json::from_value(changed).unwrap();
                assert!(original.clone().with_agentic_settings(changed).is_err());
            }
        }
        let mut forged = limited;
        forged.advisor.maximum_requests += 1;
        forged.fingerprint = forged.reproduce().unwrap();
        assert!(forged.validate_identity().is_err());
        forged.advisor.maximum_requests = 6;
        forged
            .agentic
            .as_mut()
            .unwrap()
            .provider_limits
            .as_mut()
            .unwrap()
            .advisor
            .maximum_requests = 6;
        forged.fingerprint = forged.reproduce().unwrap();
        forged.validate_identity().unwrap();
        assert!(forged.validate(&setup, &providers, &benchmark).is_err());
    }

    #[test]
    fn one_action_pins_inputs_providers_defaults_and_one_final_evaluation() {
        let (setup, providers, benchmark) = fixture();
        let scope = OptimizationLaunchScope::bind(&setup, &providers, &benchmark).unwrap();
        assert_eq!(scope.limits.maximum_iterations, 3);
        assert_eq!(scope.limits.maximum_models, 3);
        assert_eq!(scope.limits.maximum_development_evaluations, 6);
        assert_eq!(scope.limits.maximum_final_evaluations, 1);
        assert_eq!(
            scope.generation,
            providers.provider(ProviderRole::Generation).unwrap().limits
        );
        let authorization = OptimizationLaunchAuthorization::create(
            Uuid::new_v4(),
            scope.clone(),
            "operator",
            Utc::now(),
        )
        .unwrap();
        authorization
            .validate(&setup, &providers, &benchmark)
            .unwrap();
        assert!(
            !serde_json::to_string(&authorization)
                .unwrap()
                .contains("apiKey")
        );

        let mut changed = authorization.clone();
        changed.scope.limits.maximum_models += 1;
        changed.scope.fingerprint = changed.scope.reproduce().unwrap();
        changed.fingerprint = changed.reproduce().unwrap();
        assert!(changed.validate(&setup, &providers, &benchmark).is_err());
        let mut changed = authorization.clone();
        changed.scope.generation.maximum_requests += 1;
        changed.scope.fingerprint = changed.scope.reproduce().unwrap();
        changed.fingerprint = changed.reproduce().unwrap();
        assert!(changed.validate(&setup, &providers, &benchmark).is_err());
        let mut unknown = serde_json::to_value(&authorization).unwrap();
        unknown["scope"]["apiKey"] = "must not enter authority".into();
        assert!(serde_json::from_value::<OptimizationLaunchAuthorization>(unknown).is_err());
        let mut empty_development = scope.clone();
        empty_development.limits.maximum_development_evaluations = 0;
        empty_development.fingerprint = empty_development.reproduce().unwrap();
        assert!(
            OptimizationLaunchAuthorization::create(
                Uuid::new_v4(),
                empty_development,
                "operator",
                Utc::now()
            )
            .is_err()
        );
        for actor in ["", " operator", "line\nbreak"] {
            assert!(
                OptimizationLaunchAuthorization::create(
                    Uuid::new_v4(),
                    scope.clone(),
                    actor,
                    Utc::now()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn launch_requires_exact_current_artifacts_and_one_final_suite() {
        let (setup, providers, benchmark) = fixture();
        let scope = OptimizationLaunchScope::bind(&setup, &providers, &benchmark).unwrap();
        let mut changed = providers.clone();
        changed.providers[0].limits.maximum_requests += 1;
        changed.fingerprint = changed.reproduce_fingerprint().unwrap();
        assert!(scope.validate(&setup, &changed, &benchmark).is_err());
        let mut changed = setup.clone();
        changed.inputs.dataset.id = Uuid::new_v4();
        changed.fingerprint = changed.reproduce().unwrap();
        assert!(scope.validate(&changed, &providers, &benchmark).is_err());
        let mut no_final = benchmark.clone();
        no_final
            .definition
            .suites
            .retain(|suite| suite.role == EvidenceRole::Development);
        no_final.definition.fingerprint = no_final.definition.reproduce_fingerprint().unwrap();
        no_final.fingerprint = no_final.reproduce().unwrap();
        assert!(OptimizationLaunchScope::bind(&setup, &providers, &no_final).is_err());
    }
}
