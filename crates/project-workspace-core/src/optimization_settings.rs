//! User-controlled limits for the input-first agent loop.
//!
//! These are run inputs, not mutable process preferences. Iterations count
//! dataset/train/evaluate cycles; model turns count advisor calls per cycle.
use crate::{Invalid, ProviderLimits, require};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationMode {
    Standard,
    QuickTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationDevice {
    Auto,
    Cpu,
    Cuda,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationTrainingSettings {
    pub device: OptimizationDevice,
    pub maximum_epochs: u32,
    pub batch_size: u32,
    /// Fixed-point representation avoids floating-point identity ambiguity.
    pub learning_rate_nanos: u32,
    pub maximum_seconds_per_iteration: u32,
    /// A deterministic sample of training data only. Evaluation is unchanged.
    pub maximum_training_rows: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationAgentSettings {
    pub mode: OptimizationMode,
    pub objective: String,
    /// Core-owned inspection semantics. Historical launches omitted this field
    /// and deserialize as V1 without changing their fingerprints.
    #[serde(
        default = "legacy_analysis_protocol",
        skip_serializing_if = "is_legacy_analysis_protocol"
    )]
    pub analysis_protocol: u32,
    pub maximum_iterations: u32,
    pub maximum_agent_turns_per_iteration: u32,
    pub generation_concurrency: u32,
    /// Missing historical policy must not silently change an authorized run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_canary: Option<encoder_optimization_core::generation::GenerationCanaryPolicy>,
    /// Additions plus removals, across the whole run, including rejected edits.
    pub maximum_row_changes: u32,
    pub training: OptimizationTrainingSettings,
    /// Optional per-run ceilings; never expand the pinned project's allowances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_limits: Option<OptimizationProviderLimits>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationProviderLimits {
    pub advisor: ProviderLimits,
    pub generation: ProviderLimits,
}

impl OptimizationProviderLimits {
    pub(crate) fn validate_within(
        &self,
        advisor: &ProviderLimits,
        generation: &ProviderLimits,
    ) -> Result<(), Invalid> {
        self.advisor.validate()?;
        self.generation.validate()?;
        let within = |requested: &ProviderLimits, limit: &ProviderLimits| {
            requested.maximum_requests <= limit.maximum_requests
                && requested.maximum_input_tokens <= limit.maximum_input_tokens
                && requested.maximum_output_tokens <= limit.maximum_output_tokens
                && requested.maximum_cost_microusd <= limit.maximum_cost_microusd
        };
        require(
            within(&self.advisor, advisor) && within(&self.generation, generation),
            "Run provider ceilings cannot exceed the pinned project limits.",
        )
    }
}

impl Default for OptimizationAgentSettings {
    fn default() -> Self {
        Self {
            mode: OptimizationMode::Standard,
            objective: String::new(),
            analysis_protocol: 2,
            maximum_iterations: 3,
            maximum_agent_turns_per_iteration: 8,
            generation_concurrency: 1,
            generation_canary: Some(encoder_optimization_core::generation::GenerationCanaryPolicy::FirstBatchAllAdmittedV1),
            maximum_row_changes: 192,
            training: OptimizationTrainingSettings {
                device: OptimizationDevice::Auto,
                maximum_epochs: 1,
                batch_size: 64,
                learning_rate_nanos: 3_000,
                maximum_seconds_per_iteration: 7_200,
                maximum_training_rows: None,
            },
            provider_limits: None,
        }
    }
}

impl OptimizationAgentSettings {
    pub fn quick_test() -> Self {
        Self {
            mode: OptimizationMode::QuickTest,
            maximum_iterations: 1,
            maximum_agent_turns_per_iteration: 4,
            maximum_row_changes: 8,
            training: OptimizationTrainingSettings {
                maximum_seconds_per_iteration: 120,
                maximum_training_rows: Some(64),
                batch_size: 8,
                ..Self::default().training
            },
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        if let Some(limits) = &self.provider_limits {
            limits.advisor.validate()?;
            limits.generation.validate()?;
        }
        require(
            self.objective.chars().count() <= 4_000
                && !self
                    .objective
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "Agent objective must contain at most 4,000 characters and no control codes.",
        )?;
        require(
            matches!(self.analysis_protocol, 1 | 2),
            "Agent analysis protocol must be version 1 or 2.",
        )?;
        require(
            self.analysis_protocol != 2 || self.maximum_agent_turns_per_iteration >= 3,
            "Agent analysis protocol version 2 requires at least three turns per iteration.",
        )?;
        require(
            (1..=10).contains(&self.maximum_iterations)
                && (1..=32).contains(&self.maximum_agent_turns_per_iteration)
                && (1..=16).contains(&self.generation_concurrency)
                && (1..=5_000).contains(&self.maximum_row_changes),
            "Use 1–10 iterations, 1–32 agent turns, 1–16 generation concurrency and 1–5,000 row changes.",
        )?;
        let training = &self.training;
        require(
            (1..=10).contains(&training.maximum_epochs)
                && (1..=256).contains(&training.batch_size)
                && (1..=1_000_000).contains(&training.learning_rate_nanos)
                && (1..=21_600).contains(&training.maximum_seconds_per_iteration)
                && training
                    .maximum_training_rows
                    .is_none_or(|rows| (1..=1_000_000).contains(&rows)),
            "Training settings exceed the supported finite limits.",
        )?;
        require(
            self.mode != OptimizationMode::QuickTest
                || (self.maximum_iterations == 1
                    && self.maximum_agent_turns_per_iteration <= 4
                    && self.maximum_row_changes <= 8
                    && training.maximum_epochs == 1
                    && training.maximum_seconds_per_iteration <= 120
                    && training
                        .maximum_training_rows
                        .is_some_and(|rows| rows <= 64)),
            "Quick test requires one iteration, at most four agent turns, eight edits, 64 training rows and two training minutes.",
        )
    }

    /// Sampled training is diagnostic, never eligible for final acceptance.
    pub fn permits_final_evaluation(&self) -> bool {
        self.mode == OptimizationMode::Standard && self.training.maximum_training_rows.is_none()
    }
}

const fn legacy_analysis_protocol() -> u32 {
    1
}

const fn is_legacy_analysis_protocol(value: &u32) -> bool {
    *value == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_bounded_and_quick_test_is_not_one_full_slow_iteration() {
        let standard = OptimizationAgentSettings::default();
        let quick = OptimizationAgentSettings::quick_test();
        standard.validate().unwrap();
        quick.validate().unwrap();
        assert!(standard.permits_final_evaluation());
        assert!(!quick.permits_final_evaluation());
        assert_eq!(standard.generation_concurrency, 1);
        assert_eq!(quick.training.maximum_training_rows, Some(64));
        let mut invalid = quick.clone();
        invalid.training.maximum_training_rows = None;
        assert!(invalid.validate().is_err());
        invalid = quick;
        invalid.maximum_iterations = 2;
        assert!(invalid.validate().is_err());
        let too_few_turns = OptimizationAgentSettings {
            maximum_agent_turns_per_iteration: 2,
            ..OptimizationAgentSettings::default()
        };
        assert!(too_few_turns.validate().is_err());
    }

    #[test]
    fn rejects_unbounded_limits_unknown_fields_and_invalid_objectives() {
        let original = serde_json::to_value(OptimizationAgentSettings::default()).unwrap();
        for (key, bad) in [
            ("maximumIterations", 0),
            ("maximumIterations", 11),
            ("maximumAgentTurnsPerIteration", 33),
            ("generationConcurrency", 0),
            ("generationConcurrency", 17),
            ("maximumRowChanges", 5_001),
        ] {
            let mut value = original.clone();
            value[key] = bad.into();
            assert!(
                serde_json::from_value::<OptimizationAgentSettings>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut value = original.clone();
        value["apiKey"] = "secret".into();
        assert!(serde_json::from_value::<OptimizationAgentSettings>(value).is_err());
        let mut value: OptimizationAgentSettings = serde_json::from_value(original).unwrap();
        value.objective = "x".repeat(4_001);
        assert!(value.validate().is_err());
        value.objective = "normal\nobjective".into();
        value.validate().unwrap();
        value.objective.push('\0');
        assert!(value.validate().is_err());
    }

    #[test]
    fn historical_settings_remain_v1_while_new_presets_pin_v2() {
        let current = serde_json::to_value(OptimizationAgentSettings::default()).unwrap();
        assert_eq!(current["analysisProtocol"], 2);
        let mut historical = current;
        historical
            .as_object_mut()
            .unwrap()
            .remove("analysisProtocol");
        historical
            .as_object_mut()
            .unwrap()
            .remove("generationCanary");
        let decoded: OptimizationAgentSettings =
            serde_json::from_value(historical.clone()).unwrap();
        assert_eq!(decoded.analysis_protocol, 1);
        assert_eq!(decoded.generation_canary, None);
        assert_eq!(serde_json::to_value(decoded).unwrap(), historical);
    }
}
