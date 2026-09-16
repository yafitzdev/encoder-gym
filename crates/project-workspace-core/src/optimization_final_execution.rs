//! Final holdout is a one-way handoff. These records never enter Agent evidence.
use crate::{BoundIdentity, Invalid, optimization_final::AgentFinalAuthorization, require};
use chrono::{DateTime, Utc};
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalProjectSnapshot},
    metrics::{CandidateAssessment, CandidateVerdict, EvaluationReport, assess_candidate},
    protocol::ExperimentProtocol,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A durable reservation of the only dispatch and report/assessment identities.
/// Presence without a result means unknown outcome, never a reusable allowance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFinalDispatch {
    pub authorization: BoundIdentity,
    pub report_id: Uuid,
    pub assessment_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AgentFinalDispatch {
    pub fn create(
        authorization: &AgentFinalAuthorization,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            authorization: bound(authorization.id, &authorization.fingerprint),
            report_id: Uuid::new_v4(),
            assessment_id: Uuid::new_v4(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = fingerprint(&value)?;
        value.validate(authorization)?;
        Ok(value)
    }

    pub fn identity(&self) -> BoundIdentity {
        BoundIdentity {
            id: self.authorization.id.clone(),
            fingerprint: self.fingerprint.clone(),
        }
    }

    pub fn validate(&self, authorization: &AgentFinalAuthorization) -> Result<(), Invalid> {
        authorization.validate_identity()?;
        require(
            self.authorization == bound(authorization.id, &authorization.fingerprint)
                && !self.report_id.is_nil()
                && !self.assessment_id.is_nil()
                && self.report_id != self.assessment_id
                && self.created_at >= authorization.created_at
                && self.fingerprint == fingerprint(self)?,
            "Final dispatch changed or lacks exact consent",
        )
    }
}

/// Protected final evidence. Do not project this into development history,
/// Agent tools, generation prompts or another adaptive protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFinalResult {
    pub dispatch: BoundIdentity,
    pub report: EvaluationReport,
    pub assessment: CandidateAssessment,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// Row-free project custody. Protected scores remain in the native scientific
/// evidence; reads reconstruct the normalized report and deterministic verdict
/// from that evidence and verify these exact identities before presenting it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentFinalReceipt {
    pub dispatch: BoundIdentity,
    pub report: BoundIdentity,
    pub assessment: BoundIdentity,
    pub accepted: bool,
    pub report_created_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl AgentFinalReceipt {
    pub fn from_result(result: &AgentFinalResult) -> Result<Self, Invalid> {
        let mut value = Self {
            dispatch: result.dispatch.clone(),
            report: bound(result.report.id, &result.report.fingerprint),
            assessment: bound(result.assessment.id, &result.assessment.fingerprint),
            accepted: result.accepted(),
            report_created_at: result.report.created_at,
            created_at: result.created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = fingerprint(&value)?;
        Ok(value)
    }

    pub fn validate(
        &self,
        authorization: &AgentFinalAuthorization,
        dispatch: &AgentFinalDispatch,
    ) -> Result<(), Invalid> {
        dispatch.validate(authorization)?;
        self.report.validate("final report")?;
        self.assessment.validate("final assessment")?;
        require(
            self.dispatch == dispatch.identity()
                && self.report.id == dispatch.report_id.to_string()
                && self.assessment.id == dispatch.assessment_id.to_string()
                && self.report_created_at >= dispatch.created_at
                && self.created_at >= self.report_created_at
                && self.fingerprint == fingerprint(self)?,
            "Final receipt changed or differs from its dispatch",
        )
    }

    pub fn recover(
        &self,
        authorization: &AgentFinalAuthorization,
        dispatch: &AgentFinalDispatch,
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
        mut native: EvaluationReport,
    ) -> Result<AgentFinalResult, Invalid> {
        self.validate(authorization, dispatch)?;
        native
            .validate_integrity(project, &protocol.metric_contract)
            .map_err(invalid)?;
        native.created_at = self.report_created_at;
        native.fingerprint = native.reproduce_fingerprint().map_err(invalid)?;
        let result = AgentFinalResult::create(
            authorization,
            dispatch,
            project,
            protocol,
            native,
            self.created_at,
        )?;
        require(
            Self::from_result(&result)? == *self,
            "Final receipt differs from its completed native evidence",
        )?;
        Ok(result)
    }
}

impl AgentFinalResult {
    pub fn create(
        authorization: &AgentFinalAuthorization,
        dispatch: &AgentFinalDispatch,
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
        mut report: EvaluationReport,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        dispatch.validate(authorization)?;
        report
            .validate_integrity(project, &protocol.metric_contract)
            .map_err(invalid)?;
        // The report identity was reserved before any external dispatch. Native
        // re-normalization after a lost response cannot invent another identity.
        report.id = dispatch.report_id;
        report.fingerprint = report.reproduce_fingerprint().map_err(invalid)?;
        let mut assessment = assess_candidate(
            project,
            &protocol.metric_contract,
            &protocol.baseline_sealed_report,
            &report,
            created_at,
        )
        .map_err(invalid)?;
        assessment.id = dispatch.assessment_id;
        assessment.fingerprint = assessment.reproduce_fingerprint().map_err(invalid)?;
        let mut value = Self {
            dispatch: dispatch.identity(),
            report,
            assessment,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = fingerprint(&value)?;
        value.validate(authorization, dispatch, project, protocol)?;
        Ok(value)
    }

    pub fn validate(
        &self,
        authorization: &AgentFinalAuthorization,
        dispatch: &AgentFinalDispatch,
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
    ) -> Result<(), Invalid> {
        dispatch.validate(authorization)?;
        protocol.validate_integrity(project).map_err(invalid)?;
        let scope = &authorization.scope;
        self.report
            .validate_integrity(project, &protocol.metric_contract)
            .map_err(invalid)?;
        self.assessment
            .validate_integrity(
                project,
                &protocol.metric_contract,
                &protocol.baseline_sealed_report,
                &self.report,
            )
            .map_err(invalid)?;
        require(
            scope.scientific_project == bound(project.id, &project.fingerprint)
                && scope.protocol == bound(protocol.id, &protocol.fingerprint)
                && scope.metric_contract_fingerprint == protocol.metric_contract.fingerprint
                && scope.baseline_final_report
                    == bound(
                        protocol.baseline_sealed_report.id,
                        &protocol.baseline_sealed_report.fingerprint,
                    )
                && scope.final_suite == protocol.sealed_suite_key
                && scope.maximum_evaluation_seconds == protocol.maximum_evaluation_seconds
                && protocol.budget.maximum_sealed_evaluations == 0
                && self.report.evidence_role == EvidenceRole::SealedAcceptance
                && self.report.suite_key == scope.final_suite
                && bound(self.report.model.id, &self.report.model.fingerprint) == scope.model
                && self.report.reference.is_none()
                && self.dispatch == dispatch.identity()
                && self.report.id == dispatch.report_id
                && self.assessment.id == dispatch.assessment_id
                && self.report.created_at >= dispatch.created_at
                && self.created_at >= self.report.created_at
                && self.assessment.created_at == self.created_at
                && self.fingerprint == fingerprint(self)?,
            "Final result does not match the selected checkpoint, consent or reserved evidence",
        )
    }

    pub fn accepted(&self) -> bool {
        self.assessment.verdict == CandidateVerdict::Passed
    }
}

fn fingerprint(value: &impl Serialize) -> Result<String, Invalid> {
    let mut value = serde_json::to_value(value).map_err(invalid)?;
    value.as_object_mut().expect("record").remove("fingerprint");
    artifact_core::fingerprint(&value).map_err(invalid)
}
fn bound(id: Uuid, fingerprint: &str) -> BoundIdentity {
    BoundIdentity {
        id: id.to_string(),
        fingerprint: fingerprint.into(),
    }
}
fn invalid(error: impl std::fmt::Display) -> Invalid {
    Invalid(error.to_string())
}
