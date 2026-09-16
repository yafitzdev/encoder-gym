//! Manual promotion consumes verified final evidence and existing model custody.
use crate::{
    BaselineRevision, BoundIdentity, Invalid, ModelArtifact, ModelOrigin,
    optimization_final::AgentFinalAuthorization,
    optimization_final_execution::{AgentFinalDispatch, AgentFinalReceipt, AgentFinalResult},
    optimization_iteration_execution::{IterationDevelopmentResult, IterationTrainingBinding},
    require,
};
use encoder_experiment_core::{
    domain::ExternalProjectSnapshot,
    journal::{ExperimentEvent, ExperimentEventKind},
    protocol::ExperimentProtocol,
};
use uuid::Uuid;

pub struct AgentFinalPromotionEvidence<'a> {
    pub authorization: &'a AgentFinalAuthorization,
    pub dispatch: &'a AgentFinalDispatch,
    pub result: &'a AgentFinalResult,
    pub training: &'a IterationTrainingBinding,
    pub project: &'a ExternalProjectSnapshot,
    pub protocol: &'a ExperimentProtocol,
    pub events: &'a [ExperimentEvent],
    pub model: &'a ModelArtifact,
    pub baseline: &'a BaselineRevision,
}

impl AgentFinalPromotionEvidence<'_> {
    pub fn decision(&self) -> Result<BoundIdentity, Invalid> {
        self.result.validate(
            self.authorization,
            self.dispatch,
            self.project,
            self.protocol,
        )?;
        self.model.validate()?;
        self.baseline.validate()?;
        let scope = &self.authorization.scope;
        let development = IterationDevelopmentResult::from_journal(
            self.training,
            self.project,
            self.protocol,
            self.events,
        )?;
        let trained = self.events.iter().find(|event| {
            matches!(&event.event, ExperimentEventKind::CandidateTrainingCompleted { candidate_id, output }
                if *candidate_id == self.protocol.candidates[0].id && output == &development.output)
        }).ok_or_else(|| Invalid("Selected model has no training-completion receipt".into()))?;
        require(
            self.result.accepted()
                && development.development_passed
                && bound(development.experiment_run_id, &development.fingerprint)
                    == scope.development_result
                && bound(
                    development.output.model.id,
                    &development.output.model.fingerprint,
                ) == scope.model
                && self.training.reproduce()? == self.training.fingerprint
                && self.training.fingerprint == scope.training_binding_fingerprint
                && self.training.iteration == scope.iteration
                && self.training.candidate == scope.candidate
                && self.training.qualified_dataset == self.training.training_dataset
                && self.training.training_dataset == scope.dataset
                && self.model.project_id == scope.dataset.project_id
                && self.baseline.project_id == self.model.project_id
                && self.model.origin == ModelOrigin::Trained
                && self.model.source_model.as_ref() == Some(&scope.model)
                && self.model.parent_model_id == Some(self.baseline.model_artifact_id)
                && self.baseline.id.to_string() == scope.comparison_baseline_revision.id
                && self.baseline.fingerprint == scope.comparison_baseline_revision.fingerprint
                && self.model.producing_run.as_ref()
                    == Some(&bound(
                        self.training.experiment_run_id,
                        &trained.fingerprint,
                    ))
                && self.model.training_snapshot.as_ref()
                    == Some(&bound(scope.dataset.id, &scope.dataset.fingerprint))
                && self.model.trainer.as_ref()
                    == Some(&BoundIdentity {
                        id: format!(
                            "{}:{}",
                            self.project.backend.name, self.project.backend.protocol_version
                        ),
                        fingerprint: self.project.backend.configuration_fingerprint.clone(),
                    })
                && self.model.effective_configuration_fingerprint.as_ref()
                    == Some(&self.training.candidate.fingerprint)
                && self.model.source_revision.as_ref() == Some(&self.project.source_revision)
                && self.model.bytes == development.output.model.bytes
                && self.model.format == development.output.model.format,
            "Promotion requires the exact registered full-data checkpoint and accepted final evidence",
        )?;
        Ok(BoundIdentity {
            // A typed locator permits exact replay after promotion/restoration,
            // without scanning unrelated runs or changing scientific journals.
            id: format!("agent-final:{}", scope.run.id),
            fingerprint: AgentFinalReceipt::from_result(self.result)?.fingerprint,
        })
    }
}

pub fn agent_final_promotion_run(decision_id: &str) -> Result<Option<Uuid>, Invalid> {
    decision_id
        .strip_prefix("agent-final:")
        .map(|value| {
            let id = Uuid::parse_str(value)
                .map_err(|_| Invalid("Invalid Agent final promotion identity".into()))?;
            require(
                !id.is_nil() && id.to_string() == value,
                "Invalid Agent final promotion identity",
            )?;
            Ok(id)
        })
        .transpose()
}

fn bound(id: Uuid, fingerprint: &str) -> BoundIdentity {
    BoundIdentity {
        id: id.to_string(),
        fingerprint: fingerprint.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_locator_preserves_legacy_decisions() {
        let id = Uuid::new_v4();
        assert_eq!(
            agent_final_promotion_run(&format!("agent-final:{id}")).unwrap(),
            Some(id)
        );
        assert_eq!(agent_final_promotion_run(&id.to_string()).unwrap(), None);
        assert_eq!(agent_final_promotion_run("legacy-decision").unwrap(), None);
    }

    #[test]
    fn promotion_locator_rejects_ambiguous_or_invalid_uuid_spellings() {
        for value in [
            "",
            "00000000-0000-0000-0000-000000000000",
            "11111111222243338444555555555555",
            "AAAAAAAA-2222-4333-8444-555555555555",
            "../another-run",
        ] {
            assert!(agent_final_promotion_run(&format!("agent-final:{value}")).is_err());
        }
    }
}
