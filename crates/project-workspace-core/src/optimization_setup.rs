//! User-selected immutable optimization inputs, not execution authorization.
use crate::{BoundIdentity, Invalid, ModelCatalog, ProjectBenchmarkVersion, require};
use chrono::{DateTime, Utc};
use dataset_core::{
    domain::SnapshotSplit,
    versions::{DatasetVersion, DatasetVersionRef},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationInputs {
    pub project_id: Uuid,
    pub baseline_revision: BoundIdentity,
    pub model: BoundIdentity,
    pub dataset: DatasetVersionRef,
    pub benchmark: BoundIdentity,
}

impl OptimizationInputs {
    pub fn bind(
        project_id: Uuid,
        model_id: Uuid,
        catalog: &ModelCatalog,
        dataset: &DatasetVersion,
        benchmark: &ProjectBenchmarkVersion,
    ) -> Result<Self, Invalid> {
        catalog.validate()?;
        dataset
            .validate_integrity()
            .map_err(|e| Invalid(e.to_string()))?;
        benchmark
            .definition
            .validate_integrity()
            .map_err(|e| Invalid(e.to_string()))?;
        benchmark.source.validate()?;
        require(
            benchmark.reproduce()? == benchmark.fingerprint,
            "Benchmark version changed.",
        )?;
        let model = catalog.active_model();
        let baseline = catalog.active_revision();
        require(
            model.id == model_id
                && model.project_id == project_id
                && dataset.project_id == project_id
                && benchmark.project_id == project_id,
            "Choose this project's active baseline, training dataset and benchmark.",
        )?;
        require(
            !dataset.members.is_empty()
                && dataset
                    .members
                    .iter()
                    .all(|row| row.split != SnapshotSplit::Test),
            "The starting dataset must contain training input only, not test rows.",
        )?;
        let inputs = Self {
            project_id,
            baseline_revision: BoundIdentity {
                id: baseline.id.to_string(),
                fingerprint: baseline.fingerprint.clone(),
            },
            model: BoundIdentity {
                id: model.id.to_string(),
                fingerprint: model.fingerprint.clone(),
            },
            dataset: dataset.reference(),
            benchmark: BoundIdentity {
                id: benchmark.id.to_string(),
                fingerprint: benchmark.fingerprint.clone(),
            },
        };
        inputs.validate()?;
        Ok(inputs)
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.project_id.is_nil(),
            "Optimization project identity is missing.",
        )?;
        self.dataset
            .validate()
            .map_err(|e| Invalid(e.to_string()))?;
        require(
            self.dataset.project_id == self.project_id,
            "Training dataset belongs to another project.",
        )?;
        for reference in [&self.baseline_revision, &self.model, &self.benchmark] {
            reference.validate("Optimization input")?;
            require(
                Uuid::parse_str(&reference.id).is_ok_and(|id| !id.is_nil()),
                "Optimization input requires an artifact UUID.",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationSetup {
    pub id: Uuid,
    pub number: u64,
    pub parent: Option<BoundIdentity>,
    pub inputs: OptimizationInputs,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl OptimizationSetup {
    pub fn create(
        id: Uuid,
        parent: Option<&Self>,
        inputs: OptimizationInputs,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        let mut setup = Self {
            id,
            number: parent.map_or(Ok(1), |p| {
                p.number
                    .checked_add(1)
                    .ok_or_else(|| Invalid("Setup revision overflow.".into()))
            })?,
            parent: parent.map(|p| BoundIdentity {
                id: p.id.to_string(),
                fingerprint: p.fingerprint.clone(),
            }),
            inputs,
            created_at,
            fingerprint: String::new(),
        };
        setup.fingerprint = setup.reproduce()?;
        setup.validate(parent)?;
        Ok(setup)
    }

    pub fn reproduce(&self) -> Result<String, Invalid> {
        artifact_core::fingerprint(
            &serde_json::json!({"id":self.id,"number":self.number,"parent":self.parent,
            "inputs":self.inputs,"createdAt":self.created_at}),
        )
        .map_err(|e| Invalid(e.to_string()))
    }

    pub fn validate_identity(&self) -> Result<(), Invalid> {
        self.inputs.validate()?;
        require(
            !self.id.is_nil() && self.number > 0 && self.reproduce()? == self.fingerprint,
            "Optimization setup identity changed.",
        )
    }

    pub fn validate(&self, parent: Option<&Self>) -> Result<(), Invalid> {
        self.validate_identity()?;
        match (parent, &self.parent) {
            (None, None) => require(self.number == 1, "Setup history must begin at revision 1."),
            (Some(previous), Some(reference)) => {
                previous.inputs.validate()?;
                require(
                    !previous.id.is_nil()
                        && previous.number > 0
                        && previous.reproduce()? == previous.fingerprint
                        && previous.id != self.id
                        && previous.inputs.project_id == self.inputs.project_id
                        && previous.number.checked_add(1) == Some(self.number)
                        && reference.id == previous.id.to_string()
                        && reference.fingerprint == previous.fingerprint
                        && previous.created_at <= self.created_at
                        && previous.inputs != self.inputs,
                    "Setup predecessor is invalid, unchanged or belongs to another project.",
                )
            }
            _ => Err(Invalid(
                "Setup predecessor is missing or unexpected.".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> OptimizationInputs {
        let reference = || BoundIdentity {
            id: Uuid::new_v4().to_string(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        };
        let project_id = Uuid::new_v4();
        OptimizationInputs {
            project_id,
            baseline_revision: reference(),
            model: reference(),
            benchmark: reference(),
            dataset: DatasetVersionRef {
                id: Uuid::new_v4(),
                dataset_id: Uuid::new_v4(),
                project_id,
                number: 1,
                fingerprint: reference().fingerprint,
            },
        }
    }
    #[test]
    fn revisions_pin_exact_inputs_and_reject_changed_history() {
        let first = OptimizationSetup::create(Uuid::new_v4(), None, inputs(), Utc::now()).unwrap();
        assert!(
            OptimizationSetup::create(
                Uuid::new_v4(),
                Some(&first),
                first.inputs.clone(),
                Utc::now()
            )
            .is_err()
        );
        let mut changed = first.inputs.clone();
        changed.dataset.id = Uuid::new_v4();
        let second =
            OptimizationSetup::create(Uuid::new_v4(), Some(&first), changed, Utc::now()).unwrap();
        second.validate(Some(&first)).unwrap();
        assert!(second.validate(None).is_err());
        let mut tampered = second.clone();
        tampered.inputs.model.fingerprint = format!("sha256:{}", "b".repeat(64));
        assert!(tampered.validate(Some(&first)).is_err());
        let mut foreign = first.clone();
        foreign.inputs.project_id = Uuid::new_v4();
        foreign.fingerprint = foreign.reproduce().unwrap();
        assert!(second.validate(Some(&foreign)).is_err());
    }
    #[test]
    fn setup_requests_reject_foreign_or_non_identity_payloads() {
        let original = inputs();
        original.validate().unwrap();
        let mut bad = original.clone();
        bad.dataset.project_id = Uuid::new_v4();
        assert!(bad.validate().is_err());
        let mut bad = original.clone();
        bad.model.id = "a path or command".into();
        assert!(bad.validate().is_err());
        let mut json = serde_json::to_value(original).unwrap();
        json["apiKey"] = "secret".into();
        assert!(serde_json::from_value::<OptimizationInputs>(json).is_err());
    }
}
