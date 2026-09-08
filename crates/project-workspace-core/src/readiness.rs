//! Presentation-neutral readiness facts for one managed project.
//!
//! Readiness is deliberately recomputed. These values are reports over facts
//! owned by the project registry and scientific slices, never mutable launch
//! authorization or a cached success flag.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Invalid, require, validate_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCategory {
    Workspace,
    Models,
    Scientific,
    Data,
    Evaluation,
    Optimization,
    Providers,
    Recovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Ready,
    ActionRequired,
    Blocked,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessAction {
    /// Stable application intent, not a shell command.
    pub key: String,
    pub label: String,
}

impl ReadinessAction {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Result<Self, Invalid> {
        let value = Self {
            key: key.into(),
            label: label.into(),
        };
        validate_key(&value.key, "Readiness action")?;
        validate_name(&value.label)?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessCheck {
    pub key: String,
    pub category: ReadinessCategory,
    pub state: ReadinessState,
    /// Optional informational checks do not decide whether launch is runnable.
    pub required: bool,
    pub summary: String,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<ReadinessAction>,
}

impl ReadinessCheck {
    pub fn new(
        key: impl Into<String>,
        category: ReadinessCategory,
        state: ReadinessState,
        required: bool,
        summary: impl Into<String>,
        evidence: impl Into<String>,
        next_action: Option<ReadinessAction>,
    ) -> Result<Self, Invalid> {
        let value = Self {
            key: key.into(),
            category,
            state,
            required,
            summary: summary.into(),
            evidence: evidence.into(),
            next_action,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        validate_key(&self.key, "Readiness check")?;
        validate_name(&self.summary)?;
        require(
            !self.evidence.trim().is_empty()
                && self.evidence.chars().count() <= 1_000
                && !self.evidence.chars().any(char::is_control),
            "Readiness evidence must be a concise plain-language explanation.",
        )?;
        if let Some(action) = &self.next_action {
            validate_key(&action.key, "Readiness action")?;
            validate_name(&action.label)?;
        }
        require(
            self.state != ReadinessState::Ready || self.next_action.is_none(),
            "A ready check cannot request another action.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadinessReport {
    pub project_id: Uuid,
    pub baseline_revision_id: Option<Uuid>,
    pub computed_at: DateTime<Utc>,
    pub overall: ReadinessState,
    pub runnable: bool,
    pub checks: Vec<ReadinessCheck>,
}

impl ReadinessReport {
    pub fn derive(
        project_id: Uuid,
        baseline_revision_id: Option<Uuid>,
        computed_at: DateTime<Utc>,
        checks: Vec<ReadinessCheck>,
    ) -> Result<Self, Invalid> {
        require(
            !project_id.is_nil(),
            "Readiness requires a project identity.",
        )?;
        require(!checks.is_empty(), "Readiness requires explicit checks.")?;
        let mut keys = BTreeSet::new();
        for check in &checks {
            check.validate()?;
            require(
                keys.insert(check.key.as_str()),
                "Readiness check keys must be unique.",
            )?;
        }
        let required = checks.iter().filter(|check| check.required);
        let overall = required
            .filter(|check| check.state != ReadinessState::Ready)
            .map(|check| check.state)
            .max_by_key(|state| severity(*state))
            .unwrap_or(ReadinessState::Ready);
        let value = Self {
            project_id,
            baseline_revision_id,
            computed_at,
            overall,
            runnable: overall == ReadinessState::Ready,
            checks,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        let reproduced = Self::derive_without_validation(&self.checks);
        require(
            !self.project_id.is_nil()
                && !self.checks.is_empty()
                && self.overall == reproduced
                && self.runnable == (self.overall == ReadinessState::Ready),
            "Readiness summary does not match its required checks.",
        )?;
        let mut keys = BTreeSet::new();
        for check in &self.checks {
            check.validate()?;
            require(
                keys.insert(check.key.as_str()),
                "Readiness check keys must be unique.",
            )?;
        }
        Ok(())
    }

    fn derive_without_validation(checks: &[ReadinessCheck]) -> ReadinessState {
        checks
            .iter()
            .filter(|check| check.required && check.state != ReadinessState::Ready)
            .map(|check| check.state)
            .max_by_key(|state| severity(*state))
            .unwrap_or(ReadinessState::Ready)
    }
}

const fn severity(state: ReadinessState) -> u8 {
    match state {
        ReadinessState::Ready => 0,
        ReadinessState::ActionRequired => 1,
        ReadinessState::Unavailable => 2,
        ReadinessState::Stale => 3,
        ReadinessState::Blocked => 4,
    }
}

fn validate_key(value: &str, label: &str) -> Result<(), Invalid> {
    require(
        !value.is_empty()
            && value.len() <= 120
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            }),
        &format!("{label} key is invalid."),
    )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn check(key: &str, state: ReadinessState, required: bool) -> ReadinessCheck {
        ReadinessCheck::new(
            key,
            ReadinessCategory::Optimization,
            state,
            required,
            "Optimization authority",
            "The exact immutable launch authority was inspected.",
            (state != ReadinessState::Ready).then(|| {
                ReadinessAction::new("prepare-optimization", "Prepare optimization").unwrap()
            }),
        )
        .unwrap()
    }

    #[test]
    fn only_required_checks_determine_runnable_state() {
        let report = ReadinessReport::derive(
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0).unwrap(),
            vec![
                check("workspace.integrity", ReadinessState::Ready, true),
                check("providers.optional", ReadinessState::Unavailable, false),
            ],
        )
        .unwrap();
        assert!(report.runnable);
        assert_eq!(report.overall, ReadinessState::Ready);
    }

    #[test]
    fn blocked_and_stale_facts_outrank_missing_actions() {
        let report = ReadinessReport::derive(
            Uuid::new_v4(),
            None,
            Utc::now(),
            vec![
                check("optimization.preview", ReadinessState::ActionRequired, true),
                check("scientific.binding", ReadinessState::Stale, true),
                check("workspace.integrity", ReadinessState::Blocked, true),
            ],
        )
        .unwrap();
        assert_eq!(report.overall, ReadinessState::Blocked);
        assert!(!report.runnable);
    }

    #[test]
    fn duplicate_keys_and_actions_on_ready_checks_are_rejected() {
        assert!(
            ReadinessCheck::new(
                "workspace.integrity",
                ReadinessCategory::Workspace,
                ReadinessState::Ready,
                true,
                "Workspace integrity",
                "Every managed artifact matches its immutable inventory.",
                Some(ReadinessAction::new("verify-workspace", "Verify workspace").unwrap()),
            )
            .is_err()
        );
        let one = check("same", ReadinessState::ActionRequired, true);
        assert!(
            ReadinessReport::derive(Uuid::new_v4(), None, Utc::now(), vec![one.clone(), one])
                .is_err()
        );
    }
}
