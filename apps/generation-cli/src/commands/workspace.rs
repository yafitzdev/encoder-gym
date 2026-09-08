use crate::{cli::WorkspaceCommand, presentation::print};
use chrono::Utc;
use encoder_experiment_core::ports::{EncoderTaskBackend, ExperimentStore};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_sqlite::{SCHEMA_ID, SqliteExperimentStore, schema_fingerprint};
use project_workspace_core::{
    AdapterBinding, BoundIdentity, RuntimeBinding, RuntimeKind, ScientificBinding,
    ScientificStoreBinding,
};
use project_workspace_local::{
    backfill_nomos, create_workspace, import_dataset, inspect_dataset, inspect_model,
    open_workspace, record_scientific_binding, upgrade_workspace,
};
use uuid::Uuid;

pub async fn execute(command: WorkspaceCommand) -> anyhow::Result<()> {
    match command {
        WorkspaceCommand::InspectModel { source } => print(&inspect_model(&source)?),
        WorkspaceCommand::Create {
            destination,
            name,
            model,
            expected_fingerprint,
            task,
        } => {
            eprintln!("Copying and verifying the local baseline; no training will run.");
            print(
                &create_workspace(&destination, &name, &model, &expected_fingerprint, task).await?,
            )
        }
        WorkspaceCommand::Open { folder } => print(&open_workspace(&folder, false).await?),
        WorkspaceCommand::Verify { folder } => print(&open_workspace(&folder, true).await?),
        WorkspaceCommand::Upgrade { folder } => {
            eprintln!("Upgrading the project registry; model and dataset artifacts are unchanged.");
            print(&upgrade_workspace(&folder).await?)
        }
        WorkspaceCommand::BindNomos {
            folder,
            runtime,
            python,
            actor,
            reason,
        } => bind_nomos(&folder, &runtime, &python, &actor, &reason).await,
        WorkspaceCommand::InspectDataset { source, purpose } => {
            print(&inspect_dataset(&source, purpose.into())?)
        }
        WorkspaceCommand::ImportDataset {
            folder,
            source,
            name,
            purpose,
            expected_fingerprint,
        } => {
            eprintln!(
                "Copying and verifying JSONL data; no training membership or evaluation is created."
            );
            print(
                &import_dataset(
                    &folder,
                    &source,
                    &name,
                    purpose.into(),
                    &expected_fingerprint,
                )
                .await?,
            )
        }
        WorkspaceCommand::BackfillNomos {
            folder,
            source_root,
        } => {
            eprintln!("Backfilling final-stage inputs named by this baseline's training manifest.");
            print(&backfill_nomos(&folder, &source_root).await?)
        }
    }
}

async fn bind_nomos(
    folder: &std::path::Path,
    runtime: &std::path::Path,
    python: &std::path::Path,
    actor: &str,
    reason: &str,
) -> anyhow::Result<()> {
    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Upgrade this managed workspace before binding Nomos."))?;
    let backend = NomosBackend::open(runtime, python.to_path_buf())?;
    let project = backend.project_snapshot()?;
    let runtime_root = runtime.canonicalize()?;
    let runtime_baseline = inspect_model(&runtime_root.join(&project.baseline_model.key))?;
    let active_model = catalog.active_model();
    anyhow::ensure!(
        runtime_baseline.fingerprint == active_model.fingerprint
            && runtime_baseline.bytes == active_model.bytes
            && runtime_baseline.format == active_model.format,
        "The isolated Nomos runtime baseline does not match this project's active model."
    );

    let store_path = std::path::Path::new(&workspace.folder).join("runs/scientific.sqlite");
    let database_url = format!(
        "sqlite://{}",
        store_path.to_string_lossy().replace('\\', "/")
    );
    let store = SqliteExperimentStore::connect(&database_url).await?;
    if let Some(existing) = store.get_project(project.id).await? {
        anyhow::ensure!(
            existing == project,
            "The managed scientific store contains a different project at this identity."
        );
    } else {
        store.create_project(project.clone()).await?;
    }
    store.pool().close().await;

    let adapter = backend.identity();
    let previous = workspace
        .scientific_binding
        .as_ref()
        .map(|binding| binding.id);
    let binding = ScientificBinding::new(
        Uuid::new_v4(),
        workspace.manifest.id,
        catalog.active_baseline_revision_id,
        previous,
        AdapterBinding {
            key: adapter.name,
            protocol: adapter.protocol_version,
            configuration_fingerprint: adapter.configuration_fingerprint,
        },
        RuntimeBinding {
            kind: RuntimeKind::ExternalIsolated,
            location: runtime_root.to_string_lossy().into_owned(),
            project_snapshot: BoundIdentity {
                id: project.id.to_string(),
                fingerprint: project.fingerprint,
            },
        },
        ScientificStoreBinding {
            database_path: "runs/scientific.sqlite".into(),
            schema: BoundIdentity {
                id: SCHEMA_ID.into(),
                fingerprint: schema_fingerprint(),
            },
        },
        actor,
        reason,
        Utc::now(),
    )?;
    eprintln!(
        "Binding the verified isolated Nomos runtime to a new managed scientific store; no training or evaluation will run."
    );
    print(&record_scientific_binding(folder, binding, previous).await?)
}
