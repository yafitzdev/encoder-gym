use anyhow::Context;
use dataset_core::{
    domain::{DatasetSnapshot, SnapshotSplit, SplitConfiguration, SplitRatios},
    export::{to_csv, to_jsonl},
    ports::{AcceptedRowSource, SnapshotQuery, SnapshotStore},
    splitting::build_snapshot,
    statistics::{SnapshotStatistics, calculate_statistics},
};
use dataset_quality_core::{
    curation::CurationApplication,
    ports::{DatasetQualityStore, QualityCandidateSource},
};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::cli::{ExportFormat, SnapshotCommand, SnapshotSplitArg};

use super::{config, quality};

pub async fn execute(command: SnapshotCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        SnapshotCommand::Create {
            dataset_id,
            name,
            description,
            train_ratio,
            validation_ratio,
            test_ratio,
            seed,
            group_dimension,
            config: config_path,
            quality_manifest,
        } => {
            let configured = config_path
                .as_deref()
                .map(config::resolve_path)
                .transpose()?;
            let defaults = SplitRatios::default();
            let ratios = SplitRatios::new(
                train_ratio.unwrap_or_else(|| {
                    configured
                        .as_ref()
                        .map_or(defaults.train, |config| config.snapshot.train_ratio)
                }),
                validation_ratio.unwrap_or_else(|| {
                    configured.as_ref().map_or(defaults.validation, |config| {
                        config.snapshot.validation_ratio
                    })
                }),
                test_ratio.unwrap_or_else(|| {
                    configured
                        .as_ref()
                        .map_or(defaults.test, |config| config.snapshot.test_ratio)
                }),
            )?;
            let name = name
                .or_else(|| {
                    configured
                        .as_ref()
                        .map(|config| config.snapshot.name.clone())
                })
                .context("provide --name or a project --config")?;
            let description = description.or_else(|| {
                configured
                    .as_ref()
                    .and_then(|config| config.snapshot.description.clone())
            });
            let seed = seed.unwrap_or_else(|| {
                configured
                    .as_ref()
                    .map_or(42, |config| config.snapshot.seed)
            });
            let split =
                SplitConfiguration::new(ratios, seed).with_group_dimension(group_dimension)?;
            match quality_manifest {
                Some(manifest_id) => {
                    let evidence = quality::load_verified_manifest(store, manifest_id).await?;
                    anyhow::ensure!(
                        evidence.manifest.dataset_definition_id == dataset_id,
                        "quality manifest {manifest_id} belongs to dataset {}, not {dataset_id}",
                        evidence.manifest.dataset_definition_id
                    );
                    if let Some(application) =
                        store.curation_application_for_manifest(manifest_id).await?
                    {
                        let snapshot = require_snapshot(store, application.snapshot_id).await?;
                        let members = store.list_snapshot_members(snapshot.id).await?;
                        application.verify_against(
                            &evidence.manifest,
                            &evidence.report,
                            &evidence.proposal,
                            evidence.predecessor.as_ref(),
                            &evidence.row_reviews,
                            &evidence.manifest_reviews,
                            &snapshot,
                            &members,
                        )?;
                        let requested_description = description
                            .as_ref()
                            .map(|value| value.trim())
                            .filter(|value| !value.is_empty());
                        anyhow::ensure!(
                            snapshot.source_dataset_id == dataset_id
                                && snapshot.name == name.trim()
                                && snapshot.description.as_deref() == requested_description
                                && snapshot.split_configuration == split,
                            "quality manifest {manifest_id} was already applied with different snapshot settings"
                        );
                        return print_json(&serde_json::json!({
                            "snapshot": snapshot,
                            "quality_application": application,
                            "quality_manifest_id": manifest_id,
                            "qualified": true,
                            "curation_application_id": application.id,
                            "manifest_id": manifest_id,
                            "statistics": calculate_statistics(&members),
                            "idempotent_replay": true,
                        }));
                    }
                    require_current_manifest_for_new_application(store, &evidence).await?;
                    let selected = evidence.manifest.selected_source_row_ids();
                    anyhow::ensure!(
                        !selected.is_empty(),
                        "quality manifest {manifest_id} selects no source rows"
                    );
                    let source_rows = store.get_source_rows(dataset_id, selected).await?;
                    let (snapshot, members) =
                        build_snapshot(dataset_id, name, description, split, source_rows)?;
                    let application = CurationApplication::create(
                        &evidence.manifest,
                        &evidence.report,
                        &evidence.proposal,
                        evidence.predecessor.as_ref(),
                        &evidence.row_reviews,
                        &evidence.manifest_reviews,
                        &snapshot,
                        &members,
                    )?;
                    store
                        .apply_manifest(&application, &snapshot, &members)
                        .await?;
                    print_json(&serde_json::json!({
                        "snapshot": snapshot,
                        "quality_application": application,
                        "quality_manifest_id": manifest_id,
                        "qualified": true,
                        "curation_application_id": application.id,
                        "manifest_id": manifest_id,
                        "statistics": calculate_statistics(&members),
                    }))?;
                }
                None => {
                    let source_rows = store.list_accepted_source_rows(dataset_id).await?;
                    let (snapshot, members) =
                        build_snapshot(dataset_id, name, description, split, source_rows)?;
                    store.create_snapshot(&snapshot, &members).await?;
                    print_json(&serde_json::json!({
                        "snapshot": snapshot,
                        "qualified": false,
                        "curation_application_id": null,
                        "manifest_id": null,
                        "statistics": calculate_statistics(&members),
                    }))?;
                }
            }
        }
        SnapshotCommand::List { dataset_id, page } => {
            let snapshots = store
                .query_snapshots(SnapshotQuery {
                    dataset_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            let mut visible = Vec::with_capacity(snapshots.len());
            for snapshot in snapshots {
                let qualification = snapshot_qualification(store, &snapshot).await?;
                visible.push(SnapshotListItem {
                    snapshot,
                    qualification,
                });
            }
            crate::presentation::print_page(&visible, visible.len(), page)?;
        }
        SnapshotCommand::Show { id } => {
            let snapshot = require_snapshot(store, id).await?;
            let members = store.list_snapshot_members(id).await?;
            let qualification = snapshot_qualification(store, &snapshot).await?;
            print_json(&serde_json::json!({
                "snapshot": snapshot,
                "statistics": calculate_statistics(&members),
                "qualified": qualification.qualified,
                "curation_application_id": qualification.curation_application_id,
                "manifest_id": qualification.manifest_id,
            }))?;
        }
        SnapshotCommand::Members { id, split } => {
            require_snapshot(store, id).await?;
            let expected = split.map(snapshot_split);
            let members = store
                .list_snapshot_members(id)
                .await?
                .into_iter()
                .filter(|member| expected.is_none_or(|split| member.split == split))
                .collect::<Vec<_>>();
            print_json(&members)?;
        }
        SnapshotCommand::Stats { id } => {
            let snapshot = require_snapshot(store, id).await?;
            let qualification = snapshot_qualification(store, &snapshot).await?;
            print_json(&SnapshotStatisticsView {
                statistics: calculate_statistics(&store.list_snapshot_members(id).await?),
                qualification,
            })?;
        }
        SnapshotCommand::Export { id, format, output } => {
            require_snapshot(store, id).await?;
            let members = store.list_snapshot_members(id).await?;
            let contents = match format {
                ExportFormat::Jsonl => to_jsonl(&members)?,
                ExportFormat::Csv => to_csv(&members)?,
            };
            std::fs::write(&output, contents)
                .with_context(|| format!("could not write {}", output.display()))?;
            crate::presentation::print(&serde_json::json!({
                "exported": true,
                "artifact": "snapshot",
                "output": output,
            }))?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct WorkflowSnapshot {
    pub snapshot: DatasetSnapshot,
    pub curation_application: Option<CurationApplication>,
}

pub(crate) async fn create_workflow(
    dataset_id: Uuid,
    workflow_run_id: Uuid,
    iteration: u32,
    configured: &project_config::ResolvedProjectConfig,
    manifest_id: Option<Uuid>,
    store: &SqliteStore,
) -> anyhow::Result<WorkflowSnapshot> {
    let name = format!(
        "{}-workflow-{workflow_run_id}-{iteration}",
        configured.snapshot.name
    );
    let split = configured.split_configuration()?;
    let existing = store
        .query_snapshots(SnapshotQuery {
            dataset_id: Some(dataset_id),
            limit: 10_000,
            offset: 0,
        })
        .await?
        .into_iter()
        .find(|snapshot| snapshot.name == name);
    if let Some(snapshot) = existing {
        anyhow::ensure!(
            snapshot.source_dataset_id == dataset_id
                && snapshot.description == configured.snapshot.description
                && snapshot.split_configuration == split,
            "existing workflow snapshot has different immutable settings"
        );
        let qualification = snapshot_qualification(store, &snapshot).await?;
        if let Some(manifest_id) = manifest_id {
            anyhow::ensure!(
                qualification.qualified && qualification.manifest_id == Some(manifest_id),
                "existing workflow snapshot is not qualified by the exact approved manifest {manifest_id}"
            );
            let application = store
                .curation_application_for_snapshot(snapshot.id)
                .await?
                .context("qualified workflow snapshot has no curation application")?;
            return Ok(WorkflowSnapshot {
                snapshot,
                curation_application: Some(application),
            });
        }
        return Ok(WorkflowSnapshot {
            snapshot,
            curation_application: None,
        });
    }

    match manifest_id {
        Some(manifest_id) => {
            let evidence = quality::load_verified_manifest(store, manifest_id).await?;
            anyhow::ensure!(
                evidence.manifest.dataset_definition_id == dataset_id,
                "quality manifest {manifest_id} belongs to dataset {}, not {dataset_id}",
                evidence.manifest.dataset_definition_id
            );
            if let Some(application) = store.curation_application_for_manifest(manifest_id).await? {
                let snapshot = require_snapshot(store, application.snapshot_id).await?;
                let members = store.list_snapshot_members(snapshot.id).await?;
                application.verify_against(
                    &evidence.manifest,
                    &evidence.report,
                    &evidence.proposal,
                    evidence.predecessor.as_ref(),
                    &evidence.row_reviews,
                    &evidence.manifest_reviews,
                    &snapshot,
                    &members,
                )?;
                anyhow::ensure!(
                    snapshot.name == name
                        && snapshot.description == configured.snapshot.description
                        && snapshot.split_configuration == split,
                    "quality manifest {manifest_id} was already applied with different workflow snapshot settings"
                );
                return Ok(WorkflowSnapshot {
                    snapshot,
                    curation_application: Some(application),
                });
            }
            require_current_manifest_for_new_application(store, &evidence).await?;
            let selected = evidence.manifest.selected_source_row_ids();
            anyhow::ensure!(
                !selected.is_empty(),
                "quality manifest {manifest_id} selects no source rows"
            );
            let source_rows = store.get_source_rows(dataset_id, selected).await?;
            let (snapshot, members) = build_snapshot(
                dataset_id,
                name,
                configured.snapshot.description.clone(),
                split,
                source_rows,
            )?;
            let application = CurationApplication::create(
                &evidence.manifest,
                &evidence.report,
                &evidence.proposal,
                evidence.predecessor.as_ref(),
                &evidence.row_reviews,
                &evidence.manifest_reviews,
                &snapshot,
                &members,
            )?;
            store
                .apply_manifest(&application, &snapshot, &members)
                .await?;
            Ok(WorkflowSnapshot {
                snapshot,
                curation_application: Some(application),
            })
        }
        None => {
            let source_rows = store.list_accepted_source_rows(dataset_id).await?;
            let (snapshot, members) = build_snapshot(
                dataset_id,
                name,
                configured.snapshot.description.clone(),
                split,
                source_rows,
            )?;
            store.create_snapshot(&snapshot, &members).await?;
            Ok(WorkflowSnapshot {
                snapshot,
                curation_application: None,
            })
        }
    }
}

pub(crate) async fn require_workflow_qualification(
    store: &SqliteStore,
    snapshot_id: Uuid,
    manifest_id: Uuid,
) -> anyhow::Result<CurationApplication> {
    let snapshot = require_snapshot(store, snapshot_id).await?;
    let application = store
        .curation_application_for_snapshot(snapshot_id)
        .await?
        .context("quality-gated workflow snapshot has no curation application")?;
    anyhow::ensure!(
        application.manifest_id == manifest_id,
        "workflow snapshot was not qualified by the exact approved manifest {manifest_id}"
    );
    let evidence = quality::load_verified_manifest(store, manifest_id).await?;
    let members = store.list_snapshot_members(snapshot_id).await?;
    application.verify_against(
        &evidence.manifest,
        &evidence.report,
        &evidence.proposal,
        evidence.predecessor.as_ref(),
        &evidence.row_reviews,
        &evidence.manifest_reviews,
        &snapshot,
        &members,
    )?;
    Ok(application)
}

async fn require_current_manifest_for_new_application(
    store: &SqliteStore,
    evidence: &quality::VerifiedManifestEvidence,
) -> anyhow::Result<()> {
    // Materializing the current proposal is intentionally idempotent. If append-only row reviews
    // advanced after approval, this creates (or reuses) their successor before comparing exact
    // immutable identities. Existing applications are checked before this function so historical
    // qualified snapshots remain reproducible even after later human review activity.
    let current = quality::create_or_reuse_curation_proposal(store, evidence.report.run_id).await?;
    anyhow::ensure!(
        current.id == evidence.proposal.id && current.fingerprint == evidence.proposal.fingerprint,
        "quality manifest {} is stale because curation proposal {} is now current; review and approve that exact proposal before creating a new snapshot",
        evidence.manifest.id,
        current.id
    );
    Ok(())
}

async fn require_snapshot(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<dataset_core::domain::DatasetSnapshot> {
    store
        .get_snapshot(id)
        .await?
        .with_context(|| format!("snapshot not found: {id}"))
}

#[derive(Debug, Clone, serde::Serialize)]
struct SnapshotQualification {
    qualified: bool,
    curation_application_id: Option<Uuid>,
    manifest_id: Option<Uuid>,
}

#[derive(Debug, serde::Serialize)]
struct SnapshotListItem {
    #[serde(flatten)]
    snapshot: DatasetSnapshot,
    #[serde(flatten)]
    qualification: SnapshotQualification,
}

#[derive(Debug, serde::Serialize)]
struct SnapshotStatisticsView {
    #[serde(flatten)]
    statistics: SnapshotStatistics,
    #[serde(flatten)]
    qualification: SnapshotQualification,
}

async fn snapshot_qualification(
    store: &SqliteStore,
    snapshot: &DatasetSnapshot,
) -> anyhow::Result<SnapshotQualification> {
    let Some(application) = store.curation_application_for_snapshot(snapshot.id).await? else {
        return Ok(SnapshotQualification {
            qualified: false,
            curation_application_id: None,
            manifest_id: None,
        });
    };
    let evidence = quality::load_verified_manifest(store, application.manifest_id).await?;
    let members = store.list_snapshot_members(snapshot.id).await?;
    application.verify_against(
        &evidence.manifest,
        &evidence.report,
        &evidence.proposal,
        evidence.predecessor.as_ref(),
        &evidence.row_reviews,
        &evidence.manifest_reviews,
        snapshot,
        &members,
    )?;
    Ok(SnapshotQualification {
        qualified: true,
        curation_application_id: Some(application.id),
        manifest_id: Some(application.manifest_id),
    })
}

const fn snapshot_split(split: SnapshotSplitArg) -> SnapshotSplit {
    match split {
        SnapshotSplitArg::Train => SnapshotSplit::Train,
        SnapshotSplitArg::Validation => SnapshotSplit::Validation,
        SnapshotSplitArg::Test => SnapshotSplit::Test,
    }
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}
