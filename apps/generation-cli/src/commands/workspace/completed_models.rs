use super::*;
use anyhow::ensure;
use encoder_experiment_core::journal::{ExperimentEventKind, replay_experiment};
use encoder_experiment_nomos::VerifiedTrainingData;
use project_workspace_core::{ModelArtifact, ModelDatasetLink, ModelOrigin};
use project_workspace_local::{CompletedModelRegistration, register_completed_model};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectCandidateRegistration {
    run_id: Uuid,
    model: ModelArtifact,
    dataset: ModelDatasetLink,
}

/// Transfer a completed input-first candidate into the ordinary Models and
/// Data inventories. Scientific acceptance is deliberately irrelevant: a
/// trained artifact remains inspectable even when evaluation rejects it.
pub(super) async fn register_project(folder: &Path, run_id: Uuid) -> anyhow::Result<()> {
    use project_workspace_core::{
        ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    };
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let initial = vec![ActivityReference::new("run", run_id.to_string())?];
    let event = |state, references, failure| AppendActivity {
        action_id,
        operation: "optimization.register_candidate".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references,
        failure,
        created_at: Utc::now(),
    };
    append_activity(
        folder,
        event(ActivityEventState::Started, initial.clone(), None),
    )
    .await?;
    let result = register_project_output(folder, run_id).await;
    let terminal = match &result {
        Ok(value) => event(
            ActivityEventState::Succeeded,
            vec![
                initial[0].clone(),
                ActivityReference::new("model", value.model.id.to_string())?,
                ActivityReference::new("dataset_version", value.dataset.version.id.to_string())?,
            ],
            None,
        ),
        Err(_) => event(
            ActivityEventState::Failed,
            initial,
            Some(ActivityFailure::new(
                "candidate_registration_failed",
                "The trained candidate could not be added to Models. The scientific run and any already-copied artifacts were retained for an exact retry.",
            )?),
        ),
    };
    append_activity(folder, terminal).await?;
    super::print(&serde_json::json!({"actionId":action_id,"registration":result?}))
}

async fn register_project_output(
    folder: &Path,
    run_id: Uuid,
) -> anyhow::Result<ProjectCandidateRegistration> {
    let run = project_workspace_local::optimization_runs::show(folder, run_id).await?;
    ensure!(
        run.state.has_outcome(),
        "The optimization has not produced a candidate result yet."
    );
    let preparation = run
        .preparation
        .as_ref()
        .context("Optimization input verification is missing.")?;
    let materialization = run
        .materialization
        .as_ref()
        .context("Optimization dataset materialization is missing.")?;
    let experiment = run
        .experiment
        .as_ref()
        .context("Optimization experiment is missing.")?;
    let outcome = run
        .outcome
        .as_ref()
        .context("Optimization candidate outcome is missing.")?;

    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model history is not initialized.")?;
    let parent_model_id: Uuid = preparation.model.id.parse()?;
    let parent = catalog
        .artifacts
        .iter()
        .find(|model| model.id == parent_model_id)
        .context("The selected starting model is missing.")?;
    ensure!(
        parent.fingerprint == preparation.model.fingerprint,
        "The selected starting model changed."
    );
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the native training runtime.")?;
    ensure!(
        binding.id.to_string() == preparation.execution_binding.id
            && binding.fingerprint == preparation.execution_binding.fingerprint,
        "Native runtime changed after input verification."
    );
    let store = open_bound_store(&workspace.folder, binding).await?;
    let bound_project = load_bound_project(&store, binding).await?;
    let backend = open_nomos_binding(binding, &bound_project)?;
    let project_id: Uuid = materialization.scientific_project.id.parse()?;
    let project = store
        .get_project(project_id)
        .await?
        .context("The optimization project snapshot is missing.")?;
    ensure!(
        project.fingerprint == materialization.scientific_project.fingerprint
            && experiment.scientific_project == materialization.scientific_project,
        "The optimization project snapshot changed."
    );
    // Registration must reopen the same managed training population as execution,
    // not the runtime's original training inputs.
    let native = backend.load_training_dataset(
        run.run.id,
        preparation.dataset.id,
        &preparation.dataset.fingerprint,
    )?;
    ensure!(
        materialization.native_materialization.id
            == format!("{}:{}", native.run_id, native.dataset_version_id)
            && materialization.native_materialization.fingerprint == native.fingerprint
            && materialization.training_artifact == native.artifact,
        "Native training materialization changed."
    );
    let backend = backend.with_training_dataset(native)?;
    backend.verify_current_snapshot(project.clone()).await?;
    let protocol_id: Uuid = experiment.protocol.id.parse()?;
    let protocol = store
        .get_protocol(protocol_id)
        .await?
        .context("The optimization protocol is missing.")?;
    ensure!(
        protocol.fingerprint == experiment.protocol.fingerprint
            && protocol.project_snapshot_id == project.id
            && protocol.candidates.len() == 1,
        "The optimization protocol changed."
    );
    let candidate_id: Uuid = experiment.candidate.id.parse()?;
    let candidate = protocol
        .candidates
        .iter()
        .find(|candidate| candidate.id == candidate_id)
        .context("The optimization candidate is missing.")?;
    ensure!(
        candidate.fingerprint == experiment.candidate.fingerprint,
        "The optimization candidate changed."
    );
    let experiment_run_id: Uuid = experiment.experiment_run.id.parse()?;
    let events = store.load_events(experiment_run_id).await?;
    let scientific = replay_experiment(&project, &protocol, &events)?;
    ensure!(
        scientific.run_id == experiment_run_id,
        "The optimization journal belongs to another run."
    );
    let output = scientific
        .candidates
        .get(&candidate.id)
        .and_then(|execution| execution.train_output.as_ref())
        .context("The optimization did not produce a completed model.")?;
    if let Some(selected) = &outcome.selected_model {
        ensure!(
            selected.id == output.model.id.to_string()
                && selected.fingerprint == output.model.fingerprint,
            "The selected candidate differs from its training output."
        );
    }
    if let Some(final_result) = &run.final_result {
        ensure!(
            final_result.model.id == output.model.id.to_string()
                && final_result.model.fingerprint == output.model.fingerprint,
            "The final result belongs to another model."
        );
    }
    let receipt = events
        .iter()
        .find(|event| {
            matches!(
                &event.event,
                ExperimentEventKind::CandidateTrainingCompleted { candidate_id, output: recorded }
                    if *candidate_id == candidate.id && recorded.model == output.model
            )
        })
        .context("The completed model has no immutable training receipt.")?;
    let source = backend.verified_model_path(&output.model)?;
    let native = backend.verified_training_data(&project, candidate, &output.model)?;
    store.pool().close().await;

    register_verified_project_candidate(
        folder,
        run_id,
        preparation.dataset.id,
        parent_model_id,
        BoundIdentity {
            id: format!("{}:{}", binding.adapter.key, binding.adapter.protocol),
            fingerprint: binding.adapter.configuration_fingerprint.clone(),
        },
        &project.source_revision,
        &candidate.fingerprint,
        &output.model,
        experiment_run_id,
        &receipt.fingerprint,
        &source,
        native,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn register_verified_project_candidate(
    folder: &Path,
    project_run_id: Uuid,
    parent_version_id: Uuid,
    parent_model_id: Uuid,
    trainer: BoundIdentity,
    source_revision: &str,
    candidate_fingerprint: &str,
    source_model: &encoder_experiment_core::domain::ModelArtifactIdentity,
    experiment_run_id: Uuid,
    training_receipt_fingerprint: &str,
    source: &Path,
    native: VerifiedTrainingData,
) -> anyhow::Result<ProjectCandidateRegistration> {
    use project_workspace_local::model_datasets::{
        self, CompletedTrainingData, RecordedTrainingInput,
    };
    let source_identity = BoundIdentity {
        id: source_model.id.to_string(),
        fingerprint: source_model.fingerprint.clone(),
    };
    let producing_run = BoundIdentity {
        id: experiment_run_id.to_string(),
        fingerprint: training_receipt_fingerprint.into(),
    };
    let snapshot = BoundIdentity {
        id: native.snapshot_id.to_string(),
        fingerprint: native.snapshot_fingerprint,
    };
    let registered = register_completed_model(
        folder,
        source,
        CompletedModelRegistration {
            parent_model_id,
            name: format!("Candidate {}", &project_run_id.to_string()[..8]),
            source_model: source_identity.clone(),
            source_model_format: source_model.format.clone(),
            source_model_bytes: source_model.bytes,
            producing_run: producing_run.clone(),
            training_snapshot: snapshot.clone(),
            trainer,
            effective_configuration_fingerprint: candidate_fingerprint.into(),
            source_revision: source_revision.into(),
        },
    )
    .await?;
    let model = registered
        .model_catalog
        .as_ref()
        .and_then(|catalog| {
            catalog.artifacts.iter().find(|model| {
                model.source_model.as_ref() == Some(&source_identity)
                    && model.producing_run.as_ref() == Some(&producing_run)
            })
        })
        .context("The registered candidate is missing from Models.")?
        .clone();
    if !registered
        .model_dataset_links
        .iter()
        .any(|link| link.model_id == parent_model_id)
    {
        let parent = registered
            .model_catalog
            .as_ref()
            .and_then(|catalog| {
                catalog
                    .artifacts
                    .iter()
                    .find(|model| model.id == parent_model_id)
            })
            .context("The candidate's starting model is missing.")?;
        ensure!(
            parent.origin == ModelOrigin::Imported,
            "The candidate's starting model has no recorded training dataset."
        );
        model_datasets::adopt_baseline(folder).await?;
    }
    let dataset = model_datasets::adopt_completed(
        folder,
        CompletedTrainingData {
            model_id: model.id,
            parent_version_id: Some(parent_version_id),
            snapshot,
            run: producing_run,
            manifest: project_workspace_core::FileIdentity {
                path: "nomos_training_manifest.json".into(),
                bytes: native.manifest_bytes,
                fingerprint: native.manifest_fingerprint,
            },
            inputs: native
                .inputs
                .into_iter()
                .map(|input| RecordedTrainingInput {
                    key: input.key,
                    path: input.path,
                    bytes: input.bytes,
                    fingerprint: input.fingerprint,
                    rows: input.rows,
                })
                .collect(),
        },
    )
    .await?;
    Ok(ProjectCandidateRegistration {
        run_id: project_run_id,
        model,
        dataset,
    })
}

pub(super) async fn register(folder: &std::path::Path, run_id: Uuid) -> anyhow::Result<()> {
    use project_workspace_core::{
        ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    };
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let event = |state, failure| AppendActivity {
        action_id,
        operation: "models.register_run".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references: vec![
            ActivityReference::new("run", run_id.to_string()).expect("UUID reference"),
        ],
        failure,
        created_at: Utc::now(),
    };
    append_activity(folder, event(ActivityEventState::Started, None)).await?;
    let result = register_outputs(folder, run_id, false).await;
    let (state, failure) = if result.is_ok() {
        (ActivityEventState::Succeeded, None)
    } else {
        (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "model_registration_failed",
                "Completed artifacts could not all be registered. Existing models, dataset versions, and the original run are retained; retry reuses them.",
            )?),
        )
    };
    append_activity(folder, event(state, failure)).await?;
    result.map(|_| ())
}

async fn register_outputs(
    folder: &std::path::Path,
    run_id: Uuid,
    datasets_only: bool,
) -> anyhow::Result<Vec<project_workspace_core::ModelDatasetLink>> {
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model history is not initialized.")?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("Connect the scientific runtime first.")?;
    let parent = catalog
        .baseline_revisions
        .iter()
        .find(|revision| revision.id == binding.baseline_revision_id)
        .context("The bound baseline revision is missing.")?;
    let store = open_bound_store(&workspace.folder, binding).await?;
    let project = load_bound_project(&store, binding).await?;
    let backend = open_nomos_binding(binding, &project)?;
    let outputs =
        super::super::encoder_optimize::completed_model_evidence(&store, run_id, &project).await?;
    store.pool().close().await;
    ensure!(
        !datasets_only || !outputs.is_empty(),
        "This run has no completed model outputs to link."
    );
    let mut links = Vec::new();
    for evidence in outputs {
        if !datasets_only {
            let source = backend.verified_model_path(&evidence.model)?;
            register_completed_model(
                folder,
                &source,
                CompletedModelRegistration {
                    parent_model_id: parent.model_artifact_id,
                    name: format!("Candidate {:02}", evidence.candidate.sequence),
                    source_model: BoundIdentity {
                        id: evidence.model.id.to_string(),
                        fingerprint: evidence.model.fingerprint.clone(),
                    },
                    source_model_format: evidence.model.format.clone(),
                    source_model_bytes: evidence.model.bytes,
                    producing_run: BoundIdentity {
                        id: evidence.experiment_run_id.to_string(),
                        fingerprint: evidence.training_receipt_fingerprint.clone(),
                    },
                    training_snapshot: BoundIdentity {
                        id: evidence.training_snapshot_id.to_string(),
                        fingerprint: evidence.training_snapshot_fingerprint.clone(),
                    },
                    trainer: BoundIdentity {
                        id: format!("{}:{}", binding.adapter.key, binding.adapter.protocol),
                        fingerprint: binding.adapter.configuration_fingerprint.clone(),
                    },
                    effective_configuration_fingerprint: evidence.candidate.fingerprint.clone(),
                    source_revision: evidence.project.source_revision.clone(),
                },
            )
            .await?;
        }
        links.push(link_dataset(folder, &backend, &evidence).await?);
    }
    Ok(links)
}

async fn link_dataset(
    folder: &std::path::Path,
    backend: &encoder_experiment_nomos::NomosBackend,
    evidence: &super::super::encoder_optimize::CompletedModelEvidence,
) -> anyhow::Result<project_workspace_core::ModelDatasetLink> {
    use project_workspace_local::model_datasets::{
        self, CompletedTrainingData, RecordedTrainingInput,
    };
    let native =
        backend.verified_training_data(&evidence.project, &evidence.candidate, &evidence.model)?;
    ensure!(
        native.snapshot_id == evidence.training_snapshot_id
            && native.snapshot_fingerprint == evidence.training_snapshot_fingerprint,
        "The native training snapshot differs from the completed run."
    );
    let workspace = open_workspace(folder, false).await?;
    let source = BoundIdentity {
        id: evidence.model.id.to_string(),
        fingerprint: evidence.model.fingerprint.clone(),
    };
    let run = BoundIdentity {
        id: evidence.experiment_run_id.to_string(),
        fingerprint: evidence.training_receipt_fingerprint.clone(),
    };
    let model = workspace
        .model_catalog
        .as_ref()
        .and_then(|catalog| {
            catalog.artifacts.iter().find(|model| {
                model.source_model.as_ref() == Some(&source)
                    && model.producing_run.as_ref() == Some(&run)
            })
        })
        .context("Register the completed run's model before adopting its dataset.")?;
    if !workspace
        .model_dataset_links
        .iter()
        .any(|link| Some(link.model_id) == model.parent_model_id)
    {
        model_datasets::adopt_baseline(folder).await?;
    }
    model_datasets::adopt_completed(
        folder,
        CompletedTrainingData {
            model_id: model.id,
            parent_version_id: None,
            snapshot: BoundIdentity {
                id: native.snapshot_id.to_string(),
                fingerprint: native.snapshot_fingerprint,
            },
            run,
            manifest: project_workspace_core::FileIdentity {
                path: "nomos_training_manifest.json".into(),
                bytes: native.manifest_bytes,
                fingerprint: native.manifest_fingerprint,
            },
            inputs: native
                .inputs
                .into_iter()
                .map(|input| RecordedTrainingInput {
                    key: input.key,
                    path: input.path,
                    bytes: input.bytes,
                    fingerprint: input.fingerprint,
                    rows: input.rows,
                })
                .collect(),
        },
    )
    .await
}

pub(super) async fn adopt_datasets(folder: &std::path::Path, run_id: Uuid) -> anyhow::Result<()> {
    use project_workspace_core::{
        ActivityEventState, ActivityFailure, ActivityReference, ActivitySource,
    };
    initialize_activity(folder).await?;
    let action_id = Uuid::new_v4();
    let initial_refs = vec![ActivityReference::new("run", run_id.to_string())?];
    let event = |state, references, failure| AppendActivity {
        action_id,
        operation: "dataset.adopt_run".into(),
        source: ActivitySource::Cli,
        state,
        stage: None,
        completed: None,
        total: None,
        narrative: None,
        references,
        failure,
        created_at: Utc::now(),
    };
    append_activity(
        folder,
        event(ActivityEventState::Started, initial_refs.clone(), None),
    )
    .await?;
    let result = register_outputs(folder, run_id, true).await;
    let mut references = initial_refs;
    if let Ok(links) = &result {
        for link in links {
            references.push(ActivityReference::new("model", link.model_id.to_string())?);
            references.push(ActivityReference::new(
                "dataset_version",
                link.version.id.to_string(),
            )?);
        }
    }
    let (state, failure) = if result.is_ok() {
        (ActivityEventState::Succeeded, None)
    } else {
        (
            ActivityEventState::Failed,
            Some(ActivityFailure::new(
                "training_data_adoption_failed",
                "Run datasets could not all be linked. Retry reuses recorded imports, versions and links; existing model and run history are unchanged.",
            )?),
        )
    };
    append_activity(folder, event(state, references, failure)).await?;
    let links = result?;
    super::print(&serde_json::json!({"actionId":action_id,"runId":run_id,"links":links}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoder_experiment_core::domain::ModelArtifactIdentity;
    use encoder_experiment_nomos::VerifiedTrainingInput;
    use project_workspace_local::{
        backfill_nomos, create_workspace, dataset_versions as managed_dataset_versions,
        inspect_dataset, inspect_model, model_datasets,
    };
    use serde_json::json;
    use std::fs;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    #[tokio::test]
    async fn verified_project_candidate_enters_models_and_data_once() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let baseline = root.join("baseline");
        training_transformer::fixture::write_tiny_bert_bundle(&baseline).unwrap();
        let row = |id| {
            format!(
                "{}\n",
                json!({"evaluation_partition":"train","accepted":true,"decision_state_id":id,"tool_registry":[],"legal_candidate_ids":[]})
            )
        };
        fs::write(root.join("a.jsonl"), row("one") + &row("two")).unwrap();
        fs::write(root.join("b.jsonl"), row("three")).unwrap();
        fs::write(root.join("extra.jsonl"), row("four")).unwrap();
        fs::write(
            baseline.join("nomos_training_manifest.json"),
            serde_json::to_vec(&json!({
                "inputs":["a.jsonl","b.jsonl"],
                "input_state_counts":{"a.jsonl":2,"b.jsonl":1}
            }))
            .unwrap(),
        )
        .unwrap();
        let inspected = inspect_model(&baseline).unwrap();
        let folder = root.join("project");
        let created = create_workspace(
            &folder,
            "Candidate custody",
            &baseline,
            &inspected.fingerprint,
            None,
        )
        .await
        .unwrap();
        backfill_nomos(&folder, root).await.unwrap();
        let base = model_datasets::adopt_baseline(&folder).await.unwrap();

        let candidate_path = root.join("candidate");
        training_transformer::fixture::write_tiny_bert_bundle(&candidate_path).unwrap();
        let keys = ["a.jsonl", "b.jsonl", "extra.jsonl"];
        fs::write(
            candidate_path.join("nomos_training_manifest.json"),
            serde_json::to_vec(&json!({
                "inputs":keys,
                "input_row_counts":{"a.jsonl":2,"b.jsonl":1,"extra.jsonl":1}
            }))
            .unwrap(),
        )
        .unwrap();
        let checkpoint = inspect_model(&candidate_path).unwrap();
        let manifest = checkpoint
            .files
            .iter()
            .find(|file| file.path == "nomos_training_manifest.json")
            .unwrap();
        let inputs = keys
            .iter()
            .map(|key| {
                let path = root.join(key);
                let source =
                    inspect_dataset(&path, project_workspace_core::DatasetPurpose::Training)
                        .unwrap();
                VerifiedTrainingInput {
                    key: key.to_string(),
                    path,
                    bytes: source.artifact.bytes,
                    fingerprint: source.artifact.fingerprint,
                    rows: source.rows,
                }
            })
            .collect();
        let source_model = ModelArtifactIdentity::new(
            "runs/candidate",
            checkpoint.format.clone(),
            checkpoint.bytes,
            checkpoint.fingerprint.clone(),
        )
        .unwrap();
        let native = VerifiedTrainingData {
            snapshot_id: Uuid::new_v4(),
            snapshot_fingerprint: digest('1'),
            manifest_bytes: manifest.bytes,
            manifest_fingerprint: manifest.fingerprint.clone(),
            inputs,
        };
        let project_run_id = Uuid::new_v4();
        let scientific_run_id = Uuid::new_v4();
        let parent_model_id = created.model_catalog.unwrap().active_model().id;
        let candidate_fingerprint = digest('3');
        let receipt_fingerprint = digest('4');
        let register = || {
            register_verified_project_candidate(
                &folder,
                project_run_id,
                base.version.id,
                parent_model_id,
                BoundIdentity {
                    id: "nomos:nomos-ranking-v3".into(),
                    fingerprint: digest('2'),
                },
                "fixture-revision",
                &candidate_fingerprint,
                &source_model,
                scientific_run_id,
                &receipt_fingerprint,
                &candidate_path,
                native.clone(),
            )
        };

        let first = register().await.unwrap();
        let second = register().await.unwrap();
        assert_eq!(first, second);
        let workspace = open_workspace(&folder, true).await.unwrap();
        assert_eq!(workspace.model_catalog.unwrap().artifacts.len(), 2);
        assert_eq!(workspace.model_dataset_links.len(), 2);
        let entries = managed_dataset_versions::list(&folder).await.unwrap();
        let candidate_dataset = entries
            .iter()
            .find(|entry| {
                entry
                    .versions
                    .iter()
                    .any(|version| version.version == first.dataset.version)
            })
            .unwrap();
        assert_eq!(
            candidate_dataset.dataset.origin.as_ref(),
            Some(&base.version)
        );
        assert_eq!(candidate_dataset.versions[0].added, 1);
    }
}
