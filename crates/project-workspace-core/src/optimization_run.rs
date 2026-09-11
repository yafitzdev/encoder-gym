//! Project-owned root for one input-first optimization run.
//!
//! Slice-specific generation, training, evaluation, and advisor records remain
//! in their owning stores. This root binds them to the exact launch authority
//! without translating the user's inputs into a legacy repair recipe.

use crate::{BoundIdentity, Invalid, OptimizationLaunchAuthorization, require};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationRun {
    pub id: Uuid,
    pub project_id: Uuid,
    pub launch: BoundIdentity,
    pub setup: BoundIdentity,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationRun {
    pub fn reserve(
        id: Uuid,
        launch: &OptimizationLaunchAuthorization,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        launch.validate_identity()?;
        let mut value = Self {
            id,
            project_id: launch.scope.project_id,
            launch: BoundIdentity {
                id: launch.id.to_string(),
                fingerprint: launch.fingerprint.clone(),
            },
            setup: launch.scope.setup.clone(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate(launch)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "id": self.id,
            "projectId": self.project_id,
            "launch": self.launch,
            "setup": self.setup,
            "createdAt": self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    pub fn validate(&self, launch: &OptimizationLaunchAuthorization) -> Result<(), Invalid> {
        launch.validate_identity()?;
        self.launch.validate("Optimization launch")?;
        self.setup.validate("Optimization setup")?;
        require(
            !self.id.is_nil()
                && self.project_id == launch.scope.project_id
                && self.launch.id == launch.id.to_string()
                && self.launch.fingerprint == launch.fingerprint
                && self.setup == launch.scope.setup
                && self.reproduce()? == self.fingerprint,
            "Project optimization run changed or does not match its launch authority.",
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOptimizationEventKind {
    Reserved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationEvent {
    pub id: Uuid,
    pub run_id: Uuid,
    pub sequence: u64,
    pub previous_event_fingerprint: Option<String>,
    pub kind: ProjectOptimizationEventKind,
    pub launch: BoundIdentity,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProjectOptimizationEvent {
    pub fn reserved(
        id: Uuid,
        run: &ProjectOptimizationRun,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            id,
            run_id: run.id,
            sequence: 1,
            previous_event_fingerprint: None,
            kind: ProjectOptimizationEventKind::Reserved,
            launch: run.launch.clone(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce()?;
        value.validate_first(run)?;
        Ok(value)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(&serde_json::json!({
            "id": self.id,
            "runId": self.run_id,
            "sequence": self.sequence,
            "previousEventFingerprint": self.previous_event_fingerprint,
            "kind": self.kind,
            "launch": self.launch,
            "createdAt": self.created_at,
        }))
        .map_err(|error| Invalid(error.to_string()))
    }

    fn validate_first(&self, run: &ProjectOptimizationRun) -> Result<(), Invalid> {
        self.launch.validate("Optimization launch")?;
        require(
            !self.id.is_nil()
                && self.run_id == run.id
                && self.sequence == 1
                && self.previous_event_fingerprint.is_none()
                && self.kind == ProjectOptimizationEventKind::Reserved
                && self.launch == run.launch
                && self.created_at == run.created_at
                && self.reproduce()? == self.fingerprint,
            "Project optimization reservation event changed or is invalid.",
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOptimizationRunState {
    Queued,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectOptimizationRunView {
    pub run: ProjectOptimizationRun,
    pub state: ProjectOptimizationRunState,
    pub last_sequence: u64,
    pub head_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

pub fn replay_project_optimization(
    run: &ProjectOptimizationRun,
    launch: &OptimizationLaunchAuthorization,
    events: &[ProjectOptimizationEvent],
) -> Result<ProjectOptimizationRunView, Invalid> {
    run.validate(launch)?;
    require(
        events.len() == 1,
        "Project optimization journal is incomplete or unsupported.",
    )?;
    let event = &events[0];
    event.validate_first(run)?;
    Ok(ProjectOptimizationRunView {
        run: run.clone(),
        state: ProjectOptimizationRunState::Queued,
        last_sequence: event.sequence,
        head_fingerprint: event.fingerprint.clone(),
        updated_at: event.created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FinalEvaluationAuthorization, OptimizationExecutionLimits, OptimizationLaunchScope,
        ProviderLimits,
    };

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn launch() -> OptimizationLaunchAuthorization {
        let provider = ProviderLimits {
            maximum_requests: 2,
            maximum_input_tokens: 1_000,
            maximum_output_tokens: 500,
            maximum_cost_microusd: 0,
        };
        let mut scope = OptimizationLaunchScope {
            project_id: Uuid::new_v4(),
            setup: BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('a'),
            },
            provider_catalog: BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('b'),
            },
            limits: OptimizationExecutionLimits {
                maximum_iterations: 3,
                maximum_models: 3,
                maximum_dataset_row_changes: 5_000,
                maximum_training_seconds: 21_600,
                maximum_development_evaluations: 6,
                maximum_final_evaluations: 1,
            },
            generation: provider.clone(),
            advisor: provider,
            final_evaluation: FinalEvaluationAuthorization::SelectedCandidateOnce,
            fingerprint: String::new(),
        };
        scope.fingerprint = scope.reproduce().unwrap();
        OptimizationLaunchAuthorization::create(Uuid::new_v4(), scope, "operator", Utc::now())
            .unwrap()
    }

    #[test]
    fn reservation_is_a_separate_root_bound_to_one_exact_launch() {
        let launch = launch();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        let event = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        let view =
            replay_project_optimization(&run, &launch, std::slice::from_ref(&event)).unwrap();
        assert_eq!(view.state, ProjectOptimizationRunState::Queued);
        assert_eq!(view.head_fingerprint, event.fingerprint);
        assert_eq!(run.setup, launch.scope.setup);
        assert_ne!(run.id, launch.id);

        let mut substituted = run.clone();
        substituted.setup.id = Uuid::new_v4().to_string();
        substituted.fingerprint = substituted.reproduce().unwrap();
        assert!(replay_project_optimization(&substituted, &launch, &[event]).is_err());
    }

    #[test]
    fn journal_rejects_missing_duplicate_and_tampered_reservations() {
        let launch = launch();
        let now = Utc::now();
        let run = ProjectOptimizationRun::reserve(Uuid::new_v4(), &launch, now).unwrap();
        let event = ProjectOptimizationEvent::reserved(Uuid::new_v4(), &run, now).unwrap();
        assert!(replay_project_optimization(&run, &launch, &[]).is_err());
        assert!(
            replay_project_optimization(&run, &launch, &[event.clone(), event.clone()]).is_err()
        );
        let mut changed = event;
        changed.launch.fingerprint = digest('f');
        changed.fingerprint = changed.reproduce().unwrap();
        assert!(replay_project_optimization(&run, &launch, &[changed]).is_err());
    }
}
