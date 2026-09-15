use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, bail};
use dataset_core::ports::SnapshotStore;
use generation_core::ports::DatasetStore;
use project_preparation::{
    CohortEvidence, PreparationEvidence, PreparationManifest, PreparationStore, compile_project,
    preview_project,
};
use synthetic_data_sqlite::SqliteStore;
use workflow_core::ports::WorkflowRunStore;

use crate::cli::ProjectCommand;

pub async fn execute(command: ProjectCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        ProjectCommand::BootstrapPreview { manifest } => {
            super::project_bootstrap::preview(&manifest)
        }
        ProjectCommand::Bootstrap { manifest } => {
            super::project_bootstrap::create(&manifest, store).await
        }
        ProjectCommand::BootstrapShow { id } => super::project_bootstrap::show(id, store).await,
        ProjectCommand::BootstrapList { page } => super::project_bootstrap::list(page, store).await,
        ProjectCommand::Preview { manifest } => {
            let manifest = load_manifest(&manifest)?;
            let evidence = load_evidence(store, &manifest).await?;
            crate::presentation::print(&preview_project(&manifest, &evidence)?)
        }
        ProjectCommand::Prepare { manifest } => {
            let manifest = load_manifest(&manifest)?;
            let manifest_fingerprint = manifest.fingerprint()?;
            if let Some(existing) = store
                .get_preparation_by_manifest(&manifest_fingerprint)
                .await?
            {
                return crate::presentation::print(&serde_json::json!({
                    "created": false,
                    "preparation": existing,
                    "next_command": format!(
                        "synth workflow start {}",
                        existing.workflow_definition_id
                    ),
                }));
            }
            let evidence = load_evidence(store, &manifest).await?;
            let preview = preview_project(&manifest, &evidence)?;
            if !preview.eligible {
                bail!(
                    "project preparation is blocked: {}",
                    preview
                        .issues
                        .iter()
                        .map(|issue| issue.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            let bundle = compile_project(&manifest, &evidence)?;
            let preparation = store.create_preparation(&bundle).await?;
            crate::presentation::print(&serde_json::json!({
                "created": preparation.id == bundle.preparation.id,
                "preparation": preparation,
                "next_command": format!(
                    "synth workflow start {}",
                    preparation.workflow_definition_id
                ),
            }))
        }
        ProjectCommand::Show { id } => {
            let preparation = store
                .get_preparation(id)
                .await?
                .with_context(|| format!("prepared project not found: {id}"))?;
            let workflow_definition = store
                .get_workflow_definition(preparation.workflow_definition_id)
                .await?
                .with_context(|| {
                    format!(
                        "prepared workflow definition not found: {}",
                        preparation.workflow_definition_id
                    )
                })?;
            crate::presentation::print(&serde_json::json!({
                "preparation": preparation,
                "workflow_definition": workflow_definition,
            }))
        }
        ProjectCommand::List { page } => {
            let values = store.list_preparations(page.limit, page.offset).await?;
            crate::presentation::print_page(&values, values.len(), page)
        }
    }
}

fn load_manifest(path: &Path) -> anyhow::Result<PreparationManifest> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("could not read preparation manifest {}", path.display()))?;
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        PreparationManifest::parse_json(&source).map_err(Into::into)
    } else {
        PreparationManifest::parse_toml(&source).map_err(Into::into)
    }
}

async fn load_evidence(
    store: &SqliteStore,
    manifest: &PreparationManifest,
) -> anyhow::Result<PreparationEvidence> {
    let mut evidence = BTreeMap::new();
    for cohort in manifest
        .development
        .cohorts
        .iter()
        .chain(manifest.sealed.iter().flat_map(|suite| &suite.cohorts))
    {
        if evidence.contains_key(&cohort.snapshot_id) {
            continue;
        }
        let snapshot = store
            .get_snapshot(cohort.snapshot_id)
            .await?
            .with_context(|| format!("snapshot not found: {}", cohort.snapshot_id))?;
        let source_dataset = store
            .get_dataset(snapshot.source_dataset_id)
            .await?
            .with_context(|| {
                format!(
                    "source dataset not found for snapshot {}: {}",
                    snapshot.id, snapshot.source_dataset_id
                )
            })?;
        let members = store.list_snapshot_members(snapshot.id).await?;
        evidence.insert(
            snapshot.id,
            CohortEvidence {
                snapshot,
                source_dataset,
                members,
            },
        );
    }
    Ok(PreparationEvidence { cohorts: evidence })
}

#[cfg(test)]
mod tests {
    use super::load_manifest;

    #[test]
    fn checked_in_example_is_a_strict_manifest() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/project-preparation.toml");
        let manifest = load_manifest(&path).expect("fixture manifest");
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.workflow.total_rows, 1_000);
    }
}
