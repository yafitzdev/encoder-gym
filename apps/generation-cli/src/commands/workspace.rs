mod completed_models;

use crate::{
    cli::{
        EncoderOptimizeAuthorizeArgs, EncoderOptimizeCancelArgs, EncoderOptimizeCommand,
        EncoderOptimizeManifestArgs, EncoderOptimizeRunArgs, ManagedOptimizeCommand,
        ManagedProviderCommand, NomosWorkspaceArgs, WorkspaceActivityCommand, WorkspaceCommand,
    },
    presentation::print,
};
use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::optimization::OptimizationRunState;
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalProjectSnapshot},
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_sqlite::{
    SCHEMA_ID, ScientificStoreInventory, SqliteExperimentStore, schema_fingerprint,
};
use project_workspace_core::{
    AdapterBinding, BoundIdentity, ModelArtifact, ModelOrigin, ProviderAuthentication,
    ProviderCatalog, ProviderConfiguration, ProviderKind, ProviderLimits, ProviderRole,
    ReadinessAction, ReadinessCategory, ReadinessCheck, ReadinessReport, ReadinessState,
    RuntimeBinding, RuntimeKind, ScientificBinding, ScientificStoreBinding, SecretReference,
};
use project_workspace_local::{
    AcceptedModelPromotion, AppendActivity, append_activity, backfill_nomos, create_workspace,
    export_activity, import_dataset, initialize_activity, inspect_dataset, inspect_model,
    open_workspace, read_action, read_activity, record_accepted_model_promotion,
    record_provider_catalog, record_scientific_binding, upgrade_workspace,
};
use uuid::Uuid;

use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use super::encoder_optimize::{
    ManagedOptimizationAuthority, ManagedOptimizationPreparation, ManagedOptimizationReadiness,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedReadinessOutput {
    report: ReadinessReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    optimization_authority: Option<ManagedOptimizationAuthority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_preview: Option<ManagedOptimizationReadiness>,
    /// Main-process recovery material. Electron replaces the contained path
    /// with a project-scoped opaque token before returning data to a renderer.
    #[serde(skip_serializing_if = "Option::is_none")]
    prepared_optimization: Option<ManagedOptimizationPreparation>,
}

#[derive(Debug, serde::Deserialize)]
struct RawPythonInspection {
    version: String,
    major: u32,
    minor: u32,
    modules: BTreeMap<String, bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PythonCapability {
    key: &'static str,
    label: &'static str,
    ready: bool,
    missing_modules: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PythonRuntimeInspection {
    executable: String,
    version: String,
    compatible_version: bool,
    capabilities: Vec<PythonCapability>,
    ready: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PythonPreparationResult {
    installed_packages: Vec<&'static str>,
    python: PythonRuntimeInspection,
    pip_bootstrapped: bool,
    network_used: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BindingModelPreview {
    name: String,
    format: String,
    bytes: u64,
    fingerprint: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BindingStorePreview {
    database_path: String,
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    imported_history: Option<BindingHistoryPreview>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BindingHistoryPreview {
    source_name: String,
    project_snapshot: BoundIdentity,
    inventory: ScientificStoreInventory,
    verification: &'static str,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NomosBindingPreview {
    project_id: Uuid,
    project_name: String,
    baseline_revision_id: Uuid,
    active_model: BindingModelPreview,
    adapter: AdapterBinding,
    runtime_location: String,
    source_revision: String,
    source_fingerprint: String,
    project_snapshot: BoundIdentity,
    python: PythonRuntimeInspection,
    store: BindingStorePreview,
    previous_binding_id: Option<Uuid>,
    ready: bool,
}

struct VerifiedNomosBinding {
    workspace: project_workspace_local::ManagedWorkspace,
    backend: NomosBackend,
    project: ExternalProjectSnapshot,
    runtime_root: PathBuf,
    python: PythonRuntimeInspection,
    history: Option<VerifiedScientificHistory>,
    existing_store: Option<PathBuf>,
}

struct VerifiedScientificHistory {
    source: PathBuf,
    project: ExternalProjectSnapshot,
    inventory: ScientificStoreInventory,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderSettingsInput {
    version: u32,
    generation: ProviderInput,
    advisor: ProviderInput,
    #[serde(default)]
    evaluator: Option<ProviderInput>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderInput {
    kind: ProviderKind,
    #[serde(default)]
    endpoint: Option<String>,
    model: String,
    authentication: ProviderAuthentication,
    #[serde(default)]
    environment_fallback: Option<String>,
    limits: ProviderLimits,
}

impl ProviderInput {
    fn resolve(
        self,
        project_id: Uuid,
        role: ProviderRole,
    ) -> anyhow::Result<ProviderConfiguration> {
        let secret = match self.authentication {
            ProviderAuthentication::Bearer => Some(SecretReference::for_role(
                project_id,
                role,
                self.environment_fallback,
            )?),
            ProviderAuthentication::None => {
                anyhow::ensure!(
                    self.environment_fallback.is_none(),
                    "Unauthenticated providers cannot configure a credential fallback."
                );
                None
            }
        };
        let value = ProviderConfiguration {
            role,
            kind: self.kind,
            endpoint: self.endpoint,
            model: self.model,
            authentication: self.authentication,
            secret,
            limits: self.limits,
        };
        value.validate(project_id)?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum SecretAvailability {
    Missing,
    Available,
    Unavailable,
}

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
        WorkspaceCommand::Activity { folder, command } => activity(&folder, command).await,
        WorkspaceCommand::Readiness { folder, manifest } => {
            print(&readiness(&folder, manifest.as_deref()).await?)
        }
        WorkspaceCommand::PrepareOptimization { folder } => {
            print(&prepare_optimization(&folder).await?)
        }
        WorkspaceCommand::Optimize { folder, command } => managed_optimize(&folder, *command).await,
        WorkspaceCommand::RegisterRunModels { folder, run_id } => {
            completed_models::register(&folder, run_id).await?;
            print(&open_workspace(&folder, false).await?)
        }
        WorkspaceCommand::Promote {
            folder,
            run_id,
            expected_baseline_revision_id,
            actor,
            reason,
        } => {
            promote_accepted(
                &folder,
                run_id,
                expected_baseline_revision_id,
                actor,
                reason,
            )
            .await
        }
        WorkspaceCommand::Providers { folder, command } => providers(&folder, command).await,
        WorkspaceCommand::PreviewNomosBinding {
            folder,
            runtime,
            python,
            history_database,
        } => preview_nomos_binding(&folder, &runtime, &python, history_database.as_deref()).await,
        WorkspaceCommand::BindNomos {
            folder,
            runtime,
            python,
            history_database,
            actor,
            reason,
        } => {
            bind_nomos(
                &folder,
                &runtime,
                &python,
                history_database.as_deref(),
                &actor,
                &reason,
            )
            .await
        }
        WorkspaceCommand::PrepareNomosPython {
            folder,
            runtime,
            python,
            allow_network_install,
        } => prepare_nomos_python(&folder, &runtime, &python, allow_network_install).await,
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

async fn activity(folder: &Path, command: WorkspaceActivityCommand) -> anyhow::Result<()> {
    match command {
        WorkspaceActivityCommand::Init => {
            print(&initialize_activity(folder).await?)?;
        }
        WorkspaceActivityCommand::List { limit } => {
            print(&read_activity(folder, usize::try_from(limit)?).await?)?;
        }
        WorkspaceActivityCommand::Show { action_id } => {
            print(&read_action(folder, action_id).await?)?;
        }
        WorkspaceActivityCommand::Append { file } => {
            anyhow::ensure!(
                std::fs::metadata(&file)?.len() <= 64 * 1024,
                "Activity request exceeds 64 KiB."
            );
            let request: AppendActivity = serde_json::from_reader(File::open(file)?)?;
            print(&append_activity(folder, request).await?)?;
        }
        WorkspaceActivityCommand::Export { output } => {
            print(&serde_json::json!({
                "output": std::path::absolute(&output)?.to_string_lossy(),
                "events": export_activity(folder, &output).await?,
            }))?;
        }
    }
    Ok(())
}

async fn providers(
    folder: &std::path::Path,
    command: ManagedProviderCommand,
) -> anyhow::Result<()> {
    // Provider settings are project-scoped metadata. Their identities do not
    // derive from model or dataset bytes, so saving or displaying them must not
    // turn into a full artifact rehash. Explicit verify/readiness operations
    // remain responsible for deep workspace integrity checks.
    let workspace = open_workspace(folder, false).await?;
    match command {
        ManagedProviderCommand::Show => print(&provider_status(&workspace)),
        ManagedProviderCommand::Configure {
            file,
            expected_revision_id,
            actor,
            reason,
        } => {
            let input = read_provider_input(&file)?;
            anyhow::ensure!(input.version == 1, "Unsupported provider settings version.");
            let current = workspace.provider_catalog.as_ref();
            anyhow::ensure!(
                current.map(|value| value.id) == expected_revision_id,
                "Provider settings changed after inspection. Reload before saving."
            );
            let mut configured = vec![
                input
                    .generation
                    .resolve(workspace.manifest.id, ProviderRole::Generation)?,
                input
                    .advisor
                    .resolve(workspace.manifest.id, ProviderRole::Advisor)?,
            ];
            if let Some(evaluator) = input.evaluator {
                configured.push(evaluator.resolve(workspace.manifest.id, ProviderRole::Evaluator)?);
            }
            let catalog = ProviderCatalog::create(
                Uuid::new_v4(),
                workspace.manifest.id,
                current.map_or(1, |value| value.sequence + 1),
                current.map(|value| value.id),
                configured,
                actor,
                reason,
                Utc::now(),
            )?;
            eprintln!(
                "Saving non-secret provider settings; credentials are not read, stored, or tested."
            );
            let updated = record_provider_catalog(folder, catalog, expected_revision_id).await?;
            print(&provider_status(&updated))
        }
    }
}

fn read_provider_input(path: &std::path::Path) -> anyhow::Result<ProviderSettingsInput> {
    let metadata = path.metadata()?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= 1_048_576,
        "Provider settings must be a JSON file of at most 1 MiB."
    );
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn provider_status(workspace: &project_workspace_local::ManagedWorkspace) -> serde_json::Value {
    let catalog = workspace.provider_catalog.as_ref();
    let credentials = catalog
        .map(|catalog| {
            catalog
                .providers
                .iter()
                .map(|provider| {
                    serde_json::json!({
                        "role": provider.role,
                        "authentication": provider.authentication,
                        "availability": secret_availability(provider),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "projectId": workspace.manifest.id,
        "configured": catalog.is_some(),
        "catalog": catalog,
        "credentialAvailability": credentials,
        "liveProbePerformed": false,
    })
}

fn secret_availability(provider: &ProviderConfiguration) -> SecretAvailability {
    if provider.authentication == ProviderAuthentication::None {
        return SecretAvailability::Available;
    }
    match provider
        .secret
        .as_ref()
        .and_then(|secret| secret.environment_fallback.as_deref())
    {
        Some(name) if std::env::var_os(name).is_some_and(|value| !value.is_empty()) => {
            SecretAvailability::Available
        }
        Some(_) => SecretAvailability::Missing,
        None => SecretAvailability::Unavailable,
    }
}

async fn prepare_optimization(
    folder: &std::path::Path,
) -> anyhow::Result<super::encoder_optimize::ManagedOptimizationPreparation> {
    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Upgrade this managed workspace before optimization."))?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Configure a scientific binding before optimization."))?;
    anyhow::ensure!(
        binding.baseline_revision_id == catalog.active_baseline_revision_id,
        "The scientific binding is stale for the active baseline; rebind it before optimization."
    );
    // A completed preparation is a persisted, read-only recovery operation.
    // Do not open the scientific store for mutation or require the native
    // runtime merely to reissue the desktop's opaque definition token.
    let passive = open_bound_store(&workspace.folder, binding).await?;
    let passive_project = load_bound_project(&passive, binding).await?;
    let authority = super::encoder_optimize::managed_authority(&passive, &passive_project).await?;
    let recovered = match authority.as_ref() {
        Some(authority) => {
            super::encoder_optimize::recover_managed(
                &passive,
                &passive_project,
                std::path::Path::new(&workspace.folder),
                authority,
            )
            .await?
        }
        None => None,
    };
    passive.pool().close().await;
    if let Some(recovered) = recovered {
        return Ok(recovered);
    }

    let store = open_bound_store_mutable(&workspace.folder, binding).await?;
    let project = load_bound_project(&store, binding).await?;
    let backend = open_nomos_binding(binding, &project)?;
    let result = super::encoder_optimize::prepare_managed(
        &store,
        &backend,
        &project,
        std::path::Path::new(&workspace.folder),
    )
    .await;
    store.pool().close().await;
    result
}

async fn managed_optimize(
    folder: &std::path::Path,
    command: ManagedOptimizeCommand,
) -> anyhow::Result<()> {
    let completed_run = match &command {
        ManagedOptimizeCommand::Resume { run_id } => Some(*run_id),
        _ => None,
    };
    // Polling observes journal state; it must not rehash the model and datasets.
    let passive = matches!(
        command,
        ManagedOptimizeCommand::Status { .. }
            | ManagedOptimizeCommand::Inspect { .. }
            | ManagedOptimizeCommand::Report { .. }
            | ManagedOptimizeCommand::Provenance { .. }
    );
    let workspace = open_workspace(folder, !passive).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Upgrade this managed workspace before optimization."))?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Configure a scientific binding before optimization."))?;
    anyhow::ensure!(
        binding.baseline_revision_id == catalog.active_baseline_revision_id,
        "The scientific binding is stale for the active baseline; rebind it before optimization."
    );
    let store = open_bound_store(&workspace.folder, binding).await?;
    let project = load_bound_project(&store, binding).await?;
    store.pool().close().await;

    let executable = binding
        .runtime
        .executable
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Rebind the runtime with an executable selection."))?;
    let backend = NomosWorkspaceArgs {
        workspace: binding.runtime.location.clone().into(),
        python: executable.into(),
    };
    let command = managed_command(command, backend);
    let root = std::path::Path::new(&workspace.folder);
    let database_url = bound_store_url(root, binding)?;
    super::encoder_optimize::execute_managed(
        command,
        &database_url,
        root,
        project.id,
        &project.fingerprint,
    )
    .await?;
    if let Some(run_id) = completed_run {
        completed_models::register(folder, run_id).await?;
    }
    Ok(())
}

async fn promote_accepted(
    folder: &std::path::Path,
    run_id: Uuid,
    expected_baseline_revision_id: Uuid,
    actor: String,
    reason: String,
) -> anyhow::Result<()> {
    let workspace = open_workspace(folder, true).await?;
    workspace
        .model_catalog
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Upgrade this managed workspace before promotion."))?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Configure a scientific binding before promotion."))?;
    anyhow::ensure!(
        binding.baseline_revision_id == expected_baseline_revision_id,
        "The accepted run was not evaluated against the requested baseline revision."
    );
    let store = open_bound_store(&workspace.folder, binding).await?;
    let project = load_bound_project(&store, binding).await?;
    let backend = open_nomos_binding(binding, &project)?;
    let evidence =
        super::encoder_optimize::accepted_promotion_evidence(&store, &backend, run_id).await?;
    store.pool().close().await;
    anyhow::ensure!(
        evidence.project.id.to_string() == binding.runtime.project_snapshot.id
            && evidence.project.fingerprint == binding.runtime.project_snapshot.fingerprint,
        "The accepted run belongs to another bound scientific project."
    );
    let source = backend.verified_model_path(&evidence.model)?;
    let request = AcceptedModelPromotion {
        expected_baseline_revision_id,
        name: format!("Accepted candidate {}", evidence.candidate.sequence),
        source_model: BoundIdentity {
            id: evidence.model.id.to_string(),
            fingerprint: evidence.model.fingerprint.clone(),
        },
        source_model_format: evidence.model.format,
        source_model_bytes: evidence.model.bytes,
        producing_run: BoundIdentity {
            id: evidence.experiment_run_id.to_string(),
            fingerprint: evidence.experiment_head_fingerprint,
        },
        training_snapshot: BoundIdentity {
            id: evidence.training_snapshot_id.to_string(),
            fingerprint: evidence.training_snapshot_fingerprint,
        },
        trainer: BoundIdentity {
            id: format!("{}:{}", binding.adapter.key, binding.adapter.protocol),
            fingerprint: binding.adapter.configuration_fingerprint.clone(),
        },
        effective_configuration_fingerprint: evidence.candidate.fingerprint,
        source_revision: evidence.project.source_revision,
        decision_id: evidence.decision_id,
        decision_fingerprint: evidence.decision_fingerprint,
        actor,
        reason,
    };
    eprintln!(
        "Copying the sealed-accepted checkpoint into managed custody and advancing the audited baseline pointer."
    );
    print(&record_accepted_model_promotion(folder, &source, request).await?)
}

fn managed_command(
    command: ManagedOptimizeCommand,
    backend: NomosWorkspaceArgs,
) -> EncoderOptimizeCommand {
    let run = |run_id| EncoderOptimizeRunArgs {
        run_id,
        backend: backend.clone(),
    };
    match command {
        ManagedOptimizeCommand::Preview { manifest } => {
            EncoderOptimizeCommand::Preview(EncoderOptimizeManifestArgs { manifest, backend })
        }
        ManagedOptimizeCommand::Start { manifest } => {
            EncoderOptimizeCommand::Start(EncoderOptimizeManifestArgs { manifest, backend })
        }
        ManagedOptimizeCommand::Status { run_id } => EncoderOptimizeCommand::Status(run(run_id)),
        ManagedOptimizeCommand::Inspect { run_id } => EncoderOptimizeCommand::Inspect(run(run_id)),
        ManagedOptimizeCommand::ReviewRepair { run_id } => {
            EncoderOptimizeCommand::ReviewRepair(run(run_id))
        }
        ManagedOptimizeCommand::ReviewDelta { run_id } => {
            EncoderOptimizeCommand::ReviewDelta(run(run_id))
        }
        ManagedOptimizeCommand::Resume { run_id } => EncoderOptimizeCommand::Resume(run(run_id)),
        ManagedOptimizeCommand::AuthorizeExternal {
            run_id,
            authorized_by,
        } => EncoderOptimizeCommand::AuthorizeExternal(EncoderOptimizeAuthorizeArgs {
            run_id,
            authorized_by,
            backend,
        }),
        ManagedOptimizeCommand::AuthorizeSealed {
            run_id,
            authorized_by,
        } => EncoderOptimizeCommand::AuthorizeSealed(EncoderOptimizeAuthorizeArgs {
            run_id,
            authorized_by,
            backend,
        }),
        ManagedOptimizeCommand::Cancel { run_id, reason } => {
            EncoderOptimizeCommand::Cancel(EncoderOptimizeCancelArgs {
                run_id,
                reason,
                backend,
            })
        }
        ManagedOptimizeCommand::Doctor { run_id } => EncoderOptimizeCommand::Doctor(run(run_id)),
        ManagedOptimizeCommand::Provenance { run_id } => {
            EncoderOptimizeCommand::Provenance(run(run_id))
        }
        ManagedOptimizeCommand::Report { run_id } => EncoderOptimizeCommand::Report(run(run_id)),
    }
}

async fn readiness(
    folder: &std::path::Path,
    manifest: Option<&std::path::Path>,
) -> anyhow::Result<ManagedReadinessOutput> {
    // Readiness is a passive projection: validate the registry envelopes,
    // artifact paths, and recorded byte sizes without re-reading every model
    // and dataset byte on every screen refresh. The explicit workspace doctor
    // and every mutating launch boundary still perform the full checksum and
    // content validation through `open_workspace(folder, true)`.
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace.model_catalog.as_ref();
    let baseline_revision_id = catalog.map(|value| value.active_baseline_revision_id);
    let mut checks = Vec::new();

    checks.push(check(
        "workspace.integrity",
        ReadinessCategory::Workspace,
        ReadinessState::Ready,
        true,
        "Workspace registry is coherent",
        "The manifest, project registry, artifact paths, and recorded byte sizes agree. Full artifact checksums and dataset contents are verified by Prepare before any run can be reserved.",
        None,
    )?);

    checks.push(match catalog {
        Some(catalog) => check(
            "models.active-baseline",
            ReadinessCategory::Models,
            ReadinessState::Ready,
            true,
            "One active baseline is registered",
            format!(
                "Baseline revision {} points to immutable model {} ({} bytes).",
                catalog.active_baseline_revision_id,
                catalog.active_model().id,
                catalog.active_model().bytes
            ),
            None,
        )?,
        None => check(
            "models.active-baseline",
            ReadinessCategory::Models,
            ReadinessState::ActionRequired,
            true,
            "The project registry needs an upgrade",
            "This older workspace has no immutable model catalog or baseline revision chain.",
            Some(action("upgrade-workspace", "Upgrade project registry")?),
        )?,
    });

    checks.push(check(
        "data.imported-sources",
        ReadinessCategory::Data,
        if workspace.datasets.is_empty() {
            ReadinessState::ActionRequired
        } else {
            ReadinessState::Ready
        },
        false,
        if workspace.datasets.is_empty() {
            "No source datasets are in managed custody"
        } else {
            "Managed source datasets are available"
        },
        format!(
            "The project registry contains {} immutable dataset import(s). Imports are custody only and do not imply training admission.",
            workspace.datasets.len()
        ),
        workspace
            .datasets
            .is_empty()
            .then(|| action("import-dataset", "Import source data"))
            .transpose()?,
    )?);

    let mut runtime_project: Option<ExternalProjectSnapshot> = None;
    let mut store: Option<SqliteExperimentStore> = None;
    match (&workspace.scientific_binding, catalog) {
        (None, _) => {
            checks.push(check(
                "scientific.binding",
                ReadinessCategory::Scientific,
                ReadinessState::ActionRequired,
                true,
                "No scientific runtime is bound",
                "Workspace custody is valid, but no compiled task adapter and scientific store have been selected.",
                Some(action("bind-scientific-runtime", "Configure scientific runtime")?),
            )?);
            for (key, summary) in [
                ("scientific.runtime", "Scientific runtime is unavailable"),
                ("scientific.store", "Scientific store is unavailable"),
                (
                    "scientific.project",
                    "Scientific project snapshot is unavailable",
                ),
            ] {
                checks.push(check(
                    key,
                    ReadinessCategory::Scientific,
                    ReadinessState::ActionRequired,
                    true,
                    summary,
                    "Configure and verify the project scientific binding first.",
                    Some(action(
                        "bind-scientific-runtime",
                        "Configure scientific runtime",
                    )?),
                )?);
            }
        }
        (Some(binding), Some(catalog)) => {
            let binding_current =
                binding.baseline_revision_id == catalog.active_baseline_revision_id;
            checks.push(check(
                "scientific.binding",
                ReadinessCategory::Scientific,
                if binding_current {
                    ReadinessState::Ready
                } else {
                    ReadinessState::Stale
                },
                true,
                if binding_current {
                    "Scientific binding matches the active baseline"
                } else {
                    "Scientific binding belongs to an older baseline"
                },
                format!(
                    "Binding {} was verified for baseline revision {}; active revision is {}.",
                    binding.id, binding.baseline_revision_id, catalog.active_baseline_revision_id
                ),
                (!binding_current)
                    .then(|| action("rebind-scientific-runtime", "Rebind scientific runtime"))
                    .transpose()?,
            )?);

            let store_result = open_bound_store(&workspace.folder, binding).await;
            match store_result {
                Ok(value) => {
                    checks.push(check(
                        "scientific.store",
                        ReadinessCategory::Scientific,
                        ReadinessState::Ready,
                        true,
                        "Scientific store schema is current",
                        format!(
                            "{} matches schema {}.",
                            binding.store.database_path, binding.store.schema.id
                        ),
                        None,
                    )?);
                    store = Some(value);
                }
                Err(error) => checks.push(check(
                    "scientific.store",
                    ReadinessCategory::Scientific,
                    ReadinessState::Unavailable,
                    true,
                    "Scientific store cannot be verified",
                    plain_error(&error),
                    Some(action(
                        "repair-scientific-store",
                        "Inspect scientific store",
                    )?),
                )?),
            }

            let bound_project = match &store {
                Some(store) => load_bound_project(store, binding).await,
                None => Err(anyhow::anyhow!(
                    "Verify the bound scientific store before reproducing its project snapshot."
                )),
            };
            let (project_matches, project_evidence) = match bound_project {
                Ok(project) => {
                    runtime_project = Some(project.clone());
                    match open_nomos_binding(binding, &project) {
                        Ok(_) => {
                            checks.push(check(
                            "scientific.runtime",
                            ReadinessCategory::Scientific,
                            ReadinessState::Ready,
                            true,
                            "Compiled runtime binding is available",
                            format!(
                                "Adapter {} {} opens with persisted project snapshot {}. Deep native evidence is replayed only at its owning mutation or doctor boundary.",
                                binding.adapter.key, binding.adapter.protocol, project.id
                            ),
                            None,
                        )?);
                            (
                            true,
                            "The persisted scientific project and configured adapter identities match the active binding."
                                .into(),
                        )
                        }
                        Err(error) => {
                            checks.push(check(
                                "scientific.runtime",
                                ReadinessCategory::Scientific,
                                ReadinessState::Unavailable,
                                true,
                                "Compiled runtime cannot be verified",
                                plain_error(&error),
                                Some(action(
                                    "rebind-scientific-runtime",
                                    "Rebind scientific runtime",
                                )?),
                            )?);
                            (
                                true,
                                "The persisted scientific project identity matches its binding; the configured native runtime is currently unavailable."
                                    .into(),
                            )
                        }
                    }
                }
                Err(error) => {
                    checks.push(check(
                        "scientific.runtime",
                        ReadinessCategory::Scientific,
                        ReadinessState::Unavailable,
                        true,
                        "Compiled runtime cannot be verified",
                        "The exact persisted scientific project must be available before runtime content can be reproduced.",
                        Some(action(
                            "repair-scientific-store",
                            "Inspect scientific store",
                        )?),
                    )?);
                    (false, plain_error(&error))
                }
            };
            checks.push(check(
                "scientific.project",
                ReadinessCategory::Scientific,
                if project_matches {
                    ReadinessState::Ready
                } else {
                    ReadinessState::Unavailable
                },
                true,
                if project_matches {
                    "Scientific project snapshot is exact"
                } else {
                    "Scientific project snapshot cannot be reproduced"
                },
                project_evidence,
                (!project_matches)
                    .then(|| action("rebind-scientific-runtime", "Rebind scientific runtime"))
                    .transpose()?,
            )?);
        }
        (Some(_), None) => {
            checks.push(check(
                "scientific.binding",
                ReadinessCategory::Scientific,
                ReadinessState::Blocked,
                true,
                "Scientific binding cannot be evaluated",
                "Upgrade the project registry to restore the active baseline chain first.",
                Some(action("upgrade-workspace", "Upgrade project registry")?),
            )?);
            for (key, summary) in [
                (
                    "scientific.runtime",
                    "Scientific runtime cannot be evaluated",
                ),
                ("scientific.store", "Scientific store cannot be evaluated"),
                (
                    "scientific.project",
                    "Scientific project snapshot cannot be evaluated",
                ),
            ] {
                checks.push(check(
                    key,
                    ReadinessCategory::Scientific,
                    ReadinessState::Blocked,
                    true,
                    summary,
                    "Upgrade the project registry to restore the active baseline chain first.",
                    Some(action("upgrade-workspace", "Upgrade project registry")?),
                )?);
            }
        }
    }

    for (key, category, role, summary) in [
        (
            "data.training-authority",
            ReadinessCategory::Data,
            EvidenceRole::Training,
            "Immutable training input is registered",
        ),
        (
            "evaluation.development-authority",
            ReadinessCategory::Evaluation,
            EvidenceRole::Development,
            "Development evidence is registered",
        ),
        (
            "evaluation.sealed-authority",
            ReadinessCategory::Evaluation,
            EvidenceRole::SealedAcceptance,
            "Sealed evidence is registered",
        ),
    ] {
        let count = runtime_project
            .as_ref()
            .map(|project| {
                project
                    .inputs
                    .iter()
                    .filter(|input| input.role == role)
                    .count()
            })
            .unwrap_or(0);
        checks.push(check(
            key,
            category,
            if count > 0 {
                ReadinessState::Ready
            } else {
                ReadinessState::Unavailable
            },
            true,
            if count > 0 { summary } else { "Required scientific evidence is unavailable" },
            if count > 0 {
                format!("The verified scientific snapshot contains {count} input artifact(s) in this evidence role.")
            } else {
                "No verified scientific snapshot currently supplies this evidence role.".into()
            },
            (count == 0)
                .then(|| action("rebind-scientific-runtime", "Inspect scientific binding"))
                .transpose()?,
        )?);
    }

    // Provider settings become required while preparing new evidence. A fully
    // reviewed optimize manifest itself authorizes no implicit provider call.
    for role in [
        ProviderRole::Generation,
        ProviderRole::Advisor,
        ProviderRole::Evaluator,
    ] {
        checks.push(provider_readiness_check(
            workspace.provider_catalog.as_ref(),
            role,
        )?);
    }

    let (optimization_authority, authority_error) = match (store.as_ref(), runtime_project.as_ref())
    {
        (Some(store), Some(project)) => {
            match super::encoder_optimize::managed_authority_summary(store, project).await {
                Ok(value) => (value, None),
                Err(error) => (None, Some(plain_error(&error))),
            }
        }
        _ => (None, None),
    };
    let latest_project_run = match (store.as_ref(), runtime_project.as_ref()) {
        (Some(store), Some(project)) => {
            super::encoder_optimize::managed_project_recovery(store, project).await
        }
        _ => Ok(None),
    };
    let mut launch_preview = None;
    let mut prepared_optimization = None;
    match (manifest, store.as_ref(), runtime_project.as_ref()) {
        (None, _, _) => {
            let recovered = match (
                optimization_authority.as_ref(),
                store.as_ref(),
                runtime_project.as_ref(),
            ) {
                (Some(authority), Some(store), Some(project)) => {
                    super::encoder_optimize::recover_managed_summary(
                        store,
                        project,
                        std::path::Path::new(&workspace.folder),
                        authority,
                    )
                    .await
                }
                _ => Ok(None),
            };
            if let Err(error) = &latest_project_run {
                checks.push(check(
                    "optimization.preview",
                    ReadinessCategory::Optimization,
                    ReadinessState::Stale,
                    true,
                    "Existing optimization journal cannot be recovered",
                    plain_error(error),
                    Some(action(
                        "inspect-scientific-history",
                        "Inspect scientific history",
                    )?),
                )?);
                checks.push(recovery_unavailable()?);
            } else if let Ok(Some(prepared)) = recovered.as_ref() {
                let prepared = prepared.clone();
                checks.push(check(
                    "optimization.preview",
                    ReadinessCategory::Optimization,
                    ReadinessState::Ready,
                    true,
                    "Prepared optimization definition resolves exactly",
                    format!(
                        "Training snapshot {}, benchmark generation {}, and {} candidate(s) remain bound to specification {}.",
                        prepared.readiness.training_snapshot_id,
                        prepared.readiness.benchmark_generation_id,
                        prepared.readiness.candidate_count,
                        prepared.readiness.specification_fingerprint
                    ),
                    None,
                )?);
                checks.push(recovery_check(&prepared.readiness)?);
                launch_preview = Some(prepared.readiness.clone());
                prepared_optimization = Some(prepared);
            } else if let Ok(Some(preview)) = latest_project_run.as_ref()
                && (optimization_authority.is_none()
                    || preview.existing_run.as_ref().is_some_and(|run| {
                        matches!(
                            run.state,
                            OptimizationRunState::Planned | OptimizationRunState::CampaignActive
                        )
                    }))
            {
                let preview = preview.clone();
                checks.push(check(
                    "optimization.preview",
                    ReadinessCategory::Optimization,
                    ReadinessState::Ready,
                    true,
                    "Existing optimization journal recovered",
                    format!(
                        "Run {} retains training snapshot {}, benchmark generation {}, and specification {} without relying on an expired launch approval or a local manifest token.",
                        preview.existing_run.as_ref().map(|run| run.run_id).ok_or_else(|| anyhow::anyhow!("recovered optimization has no run"))?,
                        preview.training_snapshot_id,
                        preview.benchmark_generation_id,
                        preview.specification_fingerprint
                    ),
                    None,
                )?);
                checks.push(recovery_check(&preview)?);
                launch_preview = Some(preview);
            } else {
                let preparation_check = if let Err(error) = &recovered {
                    check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::Stale,
                        true,
                        "Prepared optimization definition is no longer valid",
                        plain_error(&error),
                        Some(action(
                            "prepare-optimization",
                            "Repair optimization preparation",
                        )?),
                    )?
                } else if let Some(authority) = &optimization_authority {
                    check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::ActionRequired,
                        true,
                        if authority.training_snapshot_id.is_some() {
                            "Approved repair is recorded and ready to resolve"
                        } else {
                            "Approved repair is recorded and ready to freeze"
                        },
                        format!(
                            "The directly bound repair records define {} candidate(s), {} qualified delta rows, an active successor benchmark, and finite authority through {}. Prepare performs the complete historical replay; it makes no provider call and does not train or evaluate a model.",
                            authority.candidate_count, authority.delta_rows, authority.valid_until
                        ),
                        Some(action("prepare-optimization", "Prepare approved run")?),
                    )?
                } else if let Some(error) = &authority_error {
                    check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::Blocked,
                        true,
                        "Scientific optimization authority is ambiguous or invalid",
                        error,
                        Some(action(
                            "inspect-scientific-history",
                            "Inspect scientific history",
                        )?),
                    )?
                } else {
                    check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::ActionRequired,
                        true,
                        "No current approved repair is available",
                        "Create and approve a task-compatible repair proposal and native delta against an active, unused benchmark generation before preparing a run.",
                        Some(action("prepare-repair", "Prepare a repair hypothesis")?),
                    )?
                };
                checks.push(preparation_check);
                checks.push(check(
                    "recovery.current-run",
                    ReadinessCategory::Recovery,
                    ReadinessState::Ready,
                    false,
                    "No selected run needs recovery",
                    "Run recovery is evaluated after an exact optimization request is selected.",
                    None,
                )?);
            }
        }
        (Some(path), Some(store), Some(project)) => {
            match super::encoder_optimize::managed_readiness(store, path).await {
                Ok(preview)
                    if preview.project_id == project.id
                        && preview.project_fingerprint == project.fingerprint =>
                {
                    checks.push(check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::Ready,
                        true,
                        "Optimization request resolves exactly",
                        format!(
                            "Training snapshot {}, benchmark generation {}, and {} candidate(s) resolve to specification {}.",
                            preview.training_snapshot_id,
                            preview.benchmark_generation_id,
                            preview.candidate_count,
                            preview.specification_fingerprint
                        ),
                        None,
                    )?);
                    checks.push(recovery_check(&preview)?);
                    launch_preview = Some(preview);
                }
                Ok(_) => {
                    checks.push(check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::Blocked,
                        true,
                        "Optimization request belongs to another project",
                        "The resolved immutable project does not match this managed project's scientific binding.",
                        Some(action("prepare-optimization", "Prepare project optimization")?),
                    )?);
                    checks.push(recovery_unavailable()?);
                }
                Err(error) => {
                    checks.push(check(
                        "optimization.preview",
                        ReadinessCategory::Optimization,
                        ReadinessState::Stale,
                        true,
                        "Optimization request no longer resolves",
                        plain_error(&error),
                        Some(action(
                            "prepare-optimization",
                            "Repair optimization preparation",
                        )?),
                    )?);
                    checks.push(recovery_unavailable()?);
                }
            }
        }
        (Some(_), _, _) => {
            checks.push(check(
                "optimization.preview",
                ReadinessCategory::Optimization,
                ReadinessState::Unavailable,
                true,
                "Optimization request cannot be resolved yet",
                "Verify the scientific runtime and store before resolving the selected manifest.",
                Some(action(
                    "repair-scientific-binding",
                    "Repair scientific binding",
                )?),
            )?);
            checks.push(recovery_unavailable()?);
        }
    }

    let report = ReadinessReport::derive(
        workspace.manifest.id,
        baseline_revision_id,
        Utc::now(),
        checks,
    )?;
    if let Some(store) = store {
        store.pool().close().await;
    }
    Ok(ManagedReadinessOutput {
        report,
        optimization_authority,
        launch_preview,
        prepared_optimization,
    })
}

fn open_nomos_binding(
    binding: &ScientificBinding,
    project: &ExternalProjectSnapshot,
) -> anyhow::Result<NomosBackend> {
    anyhow::ensure!(
        binding.adapter.key == "nomos",
        "This executable has no compiled adapter for '{}'.",
        binding.adapter.key
    );
    let executable = binding.runtime.executable.as_ref().ok_or_else(|| {
        anyhow::anyhow!("The runtime binding predates executable selection; rebind it.")
    })?;
    let backend = NomosBackend::open(&binding.runtime.location, executable)?
        .with_baseline_model(project.baseline_model.clone())?;
    let identity = backend.identity();
    anyhow::ensure!(
        identity.name == binding.adapter.key
            && identity.protocol_version == binding.adapter.protocol
            && identity.configuration_fingerprint == binding.adapter.configuration_fingerprint,
        "The compiled adapter identity changed since binding."
    );
    anyhow::ensure!(
        project.id.to_string() == binding.runtime.project_snapshot.id
            && project.fingerprint == binding.runtime.project_snapshot.fingerprint,
        "The scientific project identity differs from this runtime binding."
    );
    Ok(backend)
}

async fn load_bound_project(
    store: &SqliteExperimentStore,
    binding: &ScientificBinding,
) -> anyhow::Result<ExternalProjectSnapshot> {
    let project_id = Uuid::parse_str(&binding.runtime.project_snapshot.id)
        .map_err(|_| anyhow::anyhow!("The bound scientific project identity is invalid."))?;
    let project = store
        .get_project(project_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("The bound scientific project snapshot is missing."))?;
    anyhow::ensure!(
        project.fingerprint == binding.runtime.project_snapshot.fingerprint,
        "The bound scientific project fingerprint changed."
    );
    Ok(project)
}

async fn open_bound_store(
    workspace_folder: &str,
    binding: &ScientificBinding,
) -> anyhow::Result<SqliteExperimentStore> {
    anyhow::ensure!(
        binding.store.schema.id == SCHEMA_ID
            && binding.store.schema.fingerprint == schema_fingerprint(),
        "The bound scientific-store schema differs from this executable."
    );
    let root = std::path::Path::new(workspace_folder);
    let url = bound_store_url(root, binding)?;
    Ok(SqliteExperimentStore::connect_read_only(&url).await?)
}

async fn open_bound_store_mutable(
    workspace_folder: &str,
    binding: &ScientificBinding,
) -> anyhow::Result<SqliteExperimentStore> {
    anyhow::ensure!(
        binding.store.schema.id == SCHEMA_ID
            && binding.store.schema.fingerprint == schema_fingerprint(),
        "The bound scientific-store schema differs from this executable."
    );
    let root = std::path::Path::new(workspace_folder);
    let url = bound_store_url(root, binding)?;
    Ok(SqliteExperimentStore::connect(&url).await?)
}

fn bound_store_url(
    workspace_root: &std::path::Path,
    binding: &ScientificBinding,
) -> anyhow::Result<String> {
    Ok(sqlite_file_url(&bound_store_path(workspace_root, binding)?))
}

fn bound_store_path(
    workspace_root: &std::path::Path,
    binding: &ScientificBinding,
) -> anyhow::Result<PathBuf> {
    let root = workspace_root.canonicalize()?;
    let path = root.join(&binding.store.database_path).canonicalize()?;
    anyhow::ensure!(
        path.starts_with(&root),
        "Scientific store escapes the managed project."
    );
    Ok(path)
}

fn sqlite_file_url(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    let ordinary = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|value| format!(r"\\{value}"))
        .or_else(|| raw.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| raw.into_owned());
    format!("sqlite://{}", ordinary.replace('\\', "/"))
}

fn recovery_check(preview: &ManagedOptimizationReadiness) -> anyhow::Result<ReadinessCheck> {
    match &preview.existing_run {
        None => check(
            "recovery.current-run",
            ReadinessCategory::Recovery,
            ReadinessState::Ready,
            true,
            "No duplicate optimization exists",
            "The exact manifest has not reserved a run and can be started idempotently.",
            None,
        ),
        Some(run)
            if matches!(
                run.state,
                OptimizationRunState::Planned | OptimizationRunState::CampaignActive
            ) =>
        {
            check(
                "recovery.current-run",
                ReadinessCategory::Recovery,
                ReadinessState::ActionRequired,
                true,
                "An existing optimization must be resumed",
                format!(
                    "Run {} is {:?}; starting a duplicate is not allowed.",
                    run.run_id, run.state
                ),
                Some(action(
                    "resume-optimization",
                    "Resume existing optimization",
                )?),
            )
        }
        Some(run) => check(
            "recovery.current-run",
            ReadinessCategory::Recovery,
            ReadinessState::Stale,
            true,
            "This optimization request is already terminal",
            format!(
                "Run {} finished in state {:?}; prepare a new immutable request.",
                run.run_id, run.state
            ),
            Some(action(
                "prepare-optimization",
                "Prepare successor optimization",
            )?),
        ),
    }
}

fn recovery_unavailable() -> anyhow::Result<ReadinessCheck> {
    check(
        "recovery.current-run",
        ReadinessCategory::Recovery,
        ReadinessState::Unavailable,
        false,
        "Run recovery cannot be evaluated",
        "Resolve the exact project optimization request before checking for an existing run.",
        Some(action("prepare-optimization", "Prepare optimization")?),
    )
}

fn provider_readiness_check(
    catalog: Option<&ProviderCatalog>,
    role: ProviderRole,
) -> anyhow::Result<ReadinessCheck> {
    let key = match role {
        ProviderRole::Generation => "providers.generation",
        ProviderRole::Advisor => "providers.advisor",
        ProviderRole::Evaluator => "providers.evaluator",
    };
    let Some(provider) = catalog.and_then(|catalog| catalog.provider(role)) else {
        let optional = role == ProviderRole::Evaluator;
        return check(
            key,
            ReadinessCategory::Providers,
            if optional {
                ReadinessState::Ready
            } else {
                ReadinessState::ActionRequired
            },
            false,
            if optional {
                "No external evaluator is configured"
            } else {
                "Provider is not configured"
            },
            if optional {
                "Evaluator credentials are unnecessary until an external evaluator is selected."
            } else {
                "Configure this authority separately before preparing new generated or agent-authored evidence."
            },
            (!optional)
                .then(|| action("configure-providers", "Configure providers"))
                .transpose()?,
        );
    };
    let availability = secret_availability(provider);
    let ready = availability == SecretAvailability::Available;
    check(
        key,
        ReadinessCategory::Providers,
        if ready {
            ReadinessState::Ready
        } else {
            ReadinessState::ActionRequired
        },
        false,
        if ready {
            "Provider configuration is usable"
        } else {
            "Provider credential is not available to this process"
        },
        format!(
            "Role {} uses {:?} model '{}' with {} maximum request(s); secret availability is {:?} and no secret value was read into this report.",
            role.key(),
            provider.kind,
            provider.model,
            provider.limits.maximum_requests,
            availability
        ),
        (!ready)
            .then(|| action("configure-provider-secret", "Configure provider credential"))
            .transpose()?,
    )
}

fn check(
    key: &str,
    category: ReadinessCategory,
    state: ReadinessState,
    required: bool,
    summary: impl Into<String>,
    evidence: impl Into<String>,
    next_action: Option<ReadinessAction>,
) -> anyhow::Result<ReadinessCheck> {
    Ok(ReadinessCheck::new(
        key,
        category,
        state,
        required,
        summary,
        evidence,
        next_action,
    )?)
}

fn action(key: &str, label: &str) -> anyhow::Result<ReadinessAction> {
    Ok(ReadinessAction::new(key, label)?)
}

fn plain_error(error: &dyn std::fmt::Display) -> String {
    let mut value = error
        .to_string()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(900)
        .collect::<String>();
    if value.trim().is_empty() {
        value = "The check failed without a usable diagnostic.".into();
    }
    value
}

async fn bind_nomos(
    folder: &std::path::Path,
    runtime: &std::path::Path,
    python: &std::path::Path,
    history_database: Option<&std::path::Path>,
    actor: &str,
    reason: &str,
) -> anyhow::Result<()> {
    let verified = verify_nomos_binding(folder, runtime, python, history_database).await?;
    let VerifiedNomosBinding {
        workspace,
        backend,
        project,
        runtime_root,
        python: python_inspection,
        history,
        existing_store,
    } = verified;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .expect("verified model catalog");
    anyhow::ensure!(
        python_inspection.ready,
        "The selected Python runtime is incomplete. Preview it and resolve every missing capability before binding."
    );

    let root = Path::new(&workspace.folder);
    let (store_path, store_fingerprint, store_bytes) = match (history, existing_store) {
        (Some(history), None) => import_scientific_history(root, &backend, &history).await?,
        (None, Some(path)) => {
            let previous = workspace
                .scientific_binding
                .as_ref()
                .context("The promoted baseline has no previous scientific binding.")?;
            let store = SqliteExperimentStore::connect(&sqlite_file_url(&path)).await?;
            if let Some(existing) = store.get_project(project.id).await? {
                anyhow::ensure!(
                    existing == project,
                    "The reused scientific store contains a different promoted-baseline project at this identity."
                );
            } else {
                store.create_project(project.clone()).await?;
            }
            store.pool().close().await;
            (
                path,
                previous.store.snapshot_fingerprint.clone(),
                previous.store.snapshot_bytes,
            )
        }
        (None, None) => {
            let path = root.join("runs/scientific.sqlite");
            let database_url = sqlite_file_url(&path);
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
            (path, None, None)
        }
        (Some(_), Some(_)) => unreachable!("verified binding store source is exclusive"),
    };
    let database_path = store_path
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/");

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
            executable: Some(python.to_string_lossy().into_owned()),
            project_snapshot: BoundIdentity {
                id: project.id.to_string(),
                fingerprint: project.fingerprint,
            },
        },
        ScientificStoreBinding {
            database_path,
            schema: BoundIdentity {
                id: SCHEMA_ID.into(),
                fingerprint: schema_fingerprint(),
            },
            snapshot_fingerprint: store_fingerprint,
            snapshot_bytes: store_bytes,
        },
        actor,
        reason,
        Utc::now(),
    )?;
    eprintln!(
        "Binding the verified isolated Nomos runtime to a contained scientific store; no training or evaluation will run."
    );
    print(&record_scientific_binding(folder, binding, previous).await?)
}

async fn preview_nomos_binding(
    folder: &std::path::Path,
    runtime: &std::path::Path,
    python: &std::path::Path,
    history_database: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let verified = verify_nomos_binding(folder, runtime, python, history_database).await?;
    let catalog = verified
        .workspace
        .model_catalog
        .as_ref()
        .expect("verified model catalog");
    let active_model = catalog.active_model();
    let adapter = verified.backend.identity();
    let default_store_path = Path::new(&verified.workspace.folder).join("runs/scientific.sqlite");
    let existing_store_path = verified.existing_store.as_ref();
    let imported_history = verified
        .history
        .as_ref()
        .map(|history| BindingHistoryPreview {
            source_name: history
                .source
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("selected scientific database")
                .to_owned(),
            project_snapshot: BoundIdentity {
                id: history.project.id.to_string(),
                fingerprint: history.project.fingerprint.clone(),
            },
            inventory: history.inventory.clone(),
            verification: "current_schema_integrity_and_runtime_project_match",
        });
    let preview = NomosBindingPreview {
        project_id: verified.workspace.manifest.id,
        project_name: verified.workspace.manifest.name.clone(),
        baseline_revision_id: catalog.active_baseline_revision_id,
        active_model: BindingModelPreview {
            name: active_model.name.clone(),
            format: active_model.format.clone(),
            bytes: active_model.bytes,
            fingerprint: active_model.fingerprint.clone(),
        },
        adapter: AdapterBinding {
            key: adapter.name,
            protocol: adapter.protocol_version,
            configuration_fingerprint: adapter.configuration_fingerprint,
        },
        runtime_location: verified.runtime_root.to_string_lossy().into_owned(),
        source_revision: verified.project.source_revision.clone(),
        source_fingerprint: verified.project.source_fingerprint.clone(),
        project_snapshot: BoundIdentity {
            id: verified.project.id.to_string(),
            fingerprint: verified.project.fingerprint.clone(),
        },
        store: BindingStorePreview {
            database_path: existing_store_path
                .and_then(|path| path.strip_prefix(&verified.workspace.folder).ok())
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| {
                    if imported_history.is_some() {
                        "runs/scientific-<snapshot-sha256>.sqlite".into()
                    } else {
                        "runs/scientific.sqlite".into()
                    }
                }),
            action: if imported_history.is_some() {
                "import_verified_history"
            } else if existing_store_path.is_some() {
                "extend_existing_store_for_promoted_baseline"
            } else if default_store_path.exists() {
                "verify_existing_store"
            } else {
                "initialize_new_store"
            },
            imported_history,
        },
        previous_binding_id: verified
            .workspace
            .scientific_binding
            .as_ref()
            .map(|value| value.id),
        ready: verified.python.ready,
        python: verified.python,
    };
    print(&preview)
}

async fn verify_nomos_binding(
    folder: &std::path::Path,
    runtime: &std::path::Path,
    python: &std::path::Path,
    history_database: Option<&std::path::Path>,
) -> anyhow::Result<VerifiedNomosBinding> {
    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Upgrade this managed workspace before binding Nomos."))?;
    let backend = NomosBackend::open(runtime, python.to_path_buf())?;
    let current_project = backend.project_snapshot()?;
    let runtime_root = runtime.canonicalize()?;
    let runtime_baseline = inspect_model(&runtime_root.join(&current_project.baseline_model.key))?;
    let active_model = catalog.active_model();
    let (backend, current_project, existing_store) = if runtime_baseline.fingerprint
        == active_model.fingerprint
        && runtime_baseline.bytes == active_model.bytes
        && runtime_baseline.format == active_model.format
    {
        (backend, current_project, None)
    } else {
        anyhow::ensure!(
            history_database.is_none(),
            "A promoted Nomos baseline must first be resolved from its existing accepted run; do not replace that history during rebind."
        );
        resolve_promoted_nomos_baseline(&workspace, backend, active_model).await?
    };
    let history = match history_database {
        Some(source) => {
            let source = source.canonicalize()?;
            let metadata = source.metadata()?;
            anyhow::ensure!(
                metadata.is_file() && metadata.len() > 0,
                "Choose a non-empty SQLite scientific database."
            );
            let managed_root = Path::new(&workspace.folder).canonicalize()?;
            anyhow::ensure!(
                !source.starts_with(&managed_root),
                "Choose external Encoder Gym history, not a database already contained by this project."
            );
            let store = SqliteExperimentStore::connect_read_only(&sqlite_file_url(&source)).await?;
            store.verify_integrity().await?;
            let project = store
                .find_project_by_source_fingerprint(current_project.source_fingerprint.clone())
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "The selected history does not contain this exact current runtime revision."
                    )
                })?;
            backend.verify_current_snapshot(project.clone()).await?;
            let inventory = store.inventory().await?;
            store.pool().close().await;
            Some(VerifiedScientificHistory {
                source,
                project,
                inventory,
            })
        }
        None => None,
    };
    let project = history
        .as_ref()
        .map(|value| value.project.clone())
        .unwrap_or(current_project);
    let python = inspect_python_runtime(python, &runtime_root).await?;
    Ok(VerifiedNomosBinding {
        workspace,
        backend,
        project,
        runtime_root,
        python,
        history,
        existing_store,
    })
}

async fn resolve_promoted_nomos_baseline(
    workspace: &project_workspace_local::ManagedWorkspace,
    backend: NomosBackend,
    active_model: &ModelArtifact,
) -> anyhow::Result<(NomosBackend, ExternalProjectSnapshot, Option<PathBuf>)> {
    anyhow::ensure!(
        active_model.origin == ModelOrigin::Trained,
        "The isolated Nomos runtime baseline does not match this project's active imported model."
    );
    let source_model = active_model
        .source_model
        .as_ref()
        .context("The promoted baseline has no scientific checkpoint identity.")?;
    let producing_run = active_model
        .producing_run
        .as_ref()
        .context("The promoted baseline has no producing-run identity.")?;
    let binding = workspace
        .scientific_binding
        .as_ref()
        .context("The promoted baseline has no previous scientific binding to rebind.")?;
    let store = open_bound_store(&workspace.folder, binding).await?;
    let previous_project = load_bound_project(&store, binding).await?;
    let backend = backend.with_baseline_model(previous_project.baseline_model.clone())?;
    backend
        .verify_current_snapshot(previous_project.clone())
        .await?;
    let store_path = bound_store_path(Path::new(&workspace.folder), binding)?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("The promoted baseline has no model catalog.")?;
    if binding.baseline_revision_id == catalog.active_baseline_revision_id {
        anyhow::ensure!(
            previous_project.baseline_model.id.to_string() == source_model.id
                && previous_project.baseline_model.fingerprint == source_model.fingerprint,
            "The current scientific binding does not reference the managed promoted baseline."
        );
        let runtime_model =
            inspect_model(&backend.verified_model_path(&previous_project.baseline_model)?)?;
        anyhow::ensure!(
            runtime_model.fingerprint == active_model.fingerprint
                && runtime_model.bytes == active_model.bytes
                && runtime_model.format == active_model.format,
            "The promoted runtime checkpoint differs from the managed active baseline."
        );
        store.pool().close().await;
        return Ok((backend, previous_project, Some(store_path)));
    }

    let mut matches = Vec::new();
    for run_id in store
        .optimization_run_ids_for_project(previous_project.id)
        .await?
    {
        let Ok(evidence) =
            super::encoder_optimize::accepted_promotion_evidence(&store, &backend, run_id).await
        else {
            continue;
        };
        if evidence.model.id.to_string() == source_model.id
            && evidence.model.fingerprint == source_model.fingerprint
            && evidence.model.bytes == active_model.bytes
            && evidence.model.format == active_model.format
            && evidence.experiment_run_id.to_string() == producing_run.id
            && evidence.experiment_head_fingerprint == producing_run.fingerprint
        {
            let runtime_model = inspect_model(&backend.verified_model_path(&evidence.model)?)?;
            if runtime_model.fingerprint == active_model.fingerprint
                && runtime_model.bytes == active_model.bytes
                && runtime_model.format == active_model.format
            {
                matches.push(evidence.model);
            }
        }
    }
    store.pool().close().await;
    anyhow::ensure!(
        matches.len() == 1,
        "The promoted baseline does not resolve to exactly one sealed-accepted checkpoint in the existing scientific history."
    );
    let configured = backend.with_baseline_model(matches.pop().expect("one promoted model"))?;
    let project = configured.project_snapshot()?;
    anyhow::ensure!(
        project.baseline_model.fingerprint == source_model.fingerprint
            && project.baseline_model.bytes == active_model.bytes
            && project.baseline_model.format == active_model.format,
        "The promoted runtime checkpoint differs from the managed active baseline."
    );
    Ok((configured, project, Some(store_path)))
}

async fn import_scientific_history(
    workspace_root: &Path,
    backend: &NomosBackend,
    history: &VerifiedScientificHistory,
) -> anyhow::Result<(PathBuf, Option<String>, Option<u64>)> {
    let runs = workspace_root.join("runs").canonicalize()?;
    let staging = runs.join(format!(".scientific-import-{}.sqlite", Uuid::new_v4()));
    if let Err(error) =
        SqliteExperimentStore::snapshot_database(&sqlite_file_url(&history.source), &staging).await
    {
        if staging.starts_with(&runs) && staging.exists() {
            let _ = std::fs::remove_file(&staging);
        }
        return Err(error.into());
    }
    let identity = hash_file(&staging)?;
    let hex = identity.0.strip_prefix("sha256:").expect("SHA-256 prefix");
    let destination = runs.join(format!("scientific-{hex}.sqlite"));
    let publish = async {
        if destination.exists() {
            let existing = hash_file(&destination)?;
            anyhow::ensure!(
                existing == identity,
                "A different scientific history already occupies the content-addressed destination."
            );
            std::fs::remove_file(&staging)?;
        } else {
            std::fs::rename(&staging, &destination)?;
        }
        let store =
            SqliteExperimentStore::connect_read_only(&sqlite_file_url(&destination)).await?;
        store.verify_integrity().await?;
        let project = store
            .get_project(history.project.id)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("The contained history lost its verified project snapshot.")
            })?;
        anyhow::ensure!(
            project == history.project,
            "The contained history project identity changed during import."
        );
        backend.verify_current_snapshot(project).await?;
        store.pool().close().await;
        Ok::<_, anyhow::Error>((destination, Some(identity.0), Some(identity.1)))
    }
    .await;
    if publish.is_err() && staging.starts_with(&runs) && staging.exists() {
        let _ = std::fs::remove_file(&staging);
    }
    publish
}

fn hash_file(path: &Path) -> anyhow::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        bytes = bytes
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow::anyhow!("Scientific history size overflow."))?;
    }
    Ok((format!("sha256:{:x}", digest.finalize()), bytes))
}

async fn inspect_python_runtime(
    executable: &std::path::Path,
    runtime: &std::path::Path,
) -> anyhow::Result<PythonRuntimeInspection> {
    const SCRIPT: &str = r#"import importlib.util,json,sys
names=['torch','sentence_transformers','transformers','datasets','accelerate','numpy','sklearn','psutil','onnxruntime_genai']
print(json.dumps({'version':'.'.join(map(str,sys.version_info[:3])),'major':sys.version_info[0],'minor':sys.version_info[1],'modules':{name:importlib.util.find_spec(name) is not None for name in names}}))"#;
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(executable)
            .args(["-B", "-c", SCRIPT])
            .current_dir(runtime)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("The selected Python runtime did not answer its offline capability check within 15 seconds."))?
    .map_err(|error| anyhow::anyhow!("Could not start the selected Python runtime: {error}"))?;
    anyhow::ensure!(
        output.status.success(),
        "The selected Python runtime failed its offline capability check: {}",
        plain_error(&String::from_utf8_lossy(&output.stderr))
    );
    let raw: RawPythonInspection = serde_json::from_slice(&output.stdout).map_err(|_| {
        anyhow::anyhow!("The selected Python runtime returned an unreadable capability report.")
    })?;
    Ok(python_runtime_inspection(
        executable.to_string_lossy().into_owned(),
        raw,
    ))
}

async fn prepare_nomos_python(
    folder: &Path,
    runtime: &Path,
    python: &Path,
    allow_network_install: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        allow_network_install,
        "Installing Python packages requires explicit --allow-network-install authorization."
    );
    let verified = verify_nomos_binding(folder, runtime, python, None).await?;
    anyhow::ensure!(
        verified.python.compatible_version,
        "Choose Python 3.11 or 3.12 before installing runtime packages."
    );
    let packages = missing_python_packages(&verified.python)?;
    anyhow::ensure!(
        !packages.is_empty(),
        "The selected Python runtime already provides every required capability."
    );
    let pip_bootstrapped = ensure_python_pip(python, &verified.runtime_root).await?;
    eprintln!(
        "Installing the fixed missing Nomos runtime package set into the explicitly selected Python environment; this may use the network."
    );
    let mut command = tokio::process::Command::new(python);
    command
        .args([
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--no-input",
            "--only-binary=:all:",
        ])
        .args(&packages)
        .current_dir(&verified.runtime_root)
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30 * 60), command.output())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "The fixed Python package installation exceeded 30 minutes and was stopped."
            )
        })?
        .map_err(|error| {
            anyhow::anyhow!("Could not start pip through the selected Python executable: {error}")
        })?;
    anyhow::ensure!(
        output.status.success(),
        "The fixed Python package installation failed with status {}. Check this interpreter's pip and network configuration, then retry.",
        output.status
    );
    let inspection = inspect_python_runtime(python, &verified.runtime_root).await?;
    anyhow::ensure!(
        inspection.ready,
        "Package installation completed, but the selected runtime still lacks a required capability. Verify the environment again."
    );
    print(&PythonPreparationResult {
        installed_packages: packages,
        python: inspection,
        pip_bootstrapped,
        network_used: true,
    })
}

async fn ensure_python_pip(python: &Path, runtime_root: &Path) -> anyhow::Result<bool> {
    async fn usable(python: &Path, runtime_root: &Path) -> anyhow::Result<bool> {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::process::Command::new(python)
                .args(["-m", "pip", "--version"])
                .current_dir(runtime_root)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "The selected Python runtime did not answer its pip check within 15 seconds."
            )
        })?
        .map_err(|error| {
            anyhow::anyhow!("Could not check pip through the selected Python executable: {error}")
        })?;
        Ok(output.status.success())
    }

    if usable(python, runtime_root).await? {
        return Ok(false);
    }
    eprintln!(
        "The selected Python environment has no usable pip; restoring it from Python's bundled offline ensurepip package."
    );
    let output = tokio::time::timeout(
        Duration::from_secs(5 * 60),
        tokio::process::Command::new(python)
            .args(["-m", "ensurepip", "--upgrade", "--default-pip"])
            .current_dir(runtime_root)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!("The offline pip bootstrap exceeded five minutes and was stopped.")
    })?
    .map_err(|error| {
        anyhow::anyhow!("Could not start Python's bundled ensurepip module: {error}")
    })?;
    anyhow::ensure!(
        output.status.success() && usable(python, runtime_root).await?,
        "Python's bundled ensurepip module could not restore pip. Repair this interpreter or choose another Python 3.11/3.12 environment."
    );
    Ok(true)
}

fn missing_python_packages(
    inspection: &PythonRuntimeInspection,
) -> anyhow::Result<Vec<&'static str>> {
    let mut packages = BTreeSet::new();
    for module in inspection
        .capabilities
        .iter()
        .flat_map(|capability| capability.missing_modules.iter())
    {
        let package = match module.as_str() {
            "torch" => "torch",
            "sentence_transformers" => "sentence-transformers",
            "transformers" => "transformers>=4.48",
            "datasets" => "datasets",
            "accelerate" => "accelerate",
            "numpy" => "numpy",
            "sklearn" => "scikit-learn",
            "psutil" => "psutil",
            "onnxruntime_genai" => "onnxruntime-genai>=0.8",
            _ => anyhow::bail!("The runtime reported an unsupported missing Python module."),
        };
        packages.insert(package);
    }
    Ok(packages.into_iter().collect())
}

fn python_runtime_inspection(
    executable: String,
    raw: RawPythonInspection,
) -> PythonRuntimeInspection {
    let groups = [
        (
            "training",
            "Encoder training",
            &[
                "torch",
                "sentence_transformers",
                "transformers",
                "datasets",
                "accelerate",
            ][..],
        ),
        (
            "retrieval",
            "Retrieval evaluation",
            &["numpy", "sklearn"][..],
        ),
        (
            "agent_evaluation",
            "Local agent evaluation",
            &["onnxruntime_genai", "transformers"][..],
        ),
        ("diagnostics", "Runtime diagnostics", &["psutil"][..]),
    ];
    let capabilities = groups
        .into_iter()
        .map(|(key, label, modules)| {
            let missing_modules = modules
                .iter()
                .filter(|module| !raw.modules.get(**module).copied().unwrap_or(false))
                .map(|module| (*module).to_owned())
                .collect::<Vec<_>>();
            PythonCapability {
                key,
                label,
                ready: missing_modules.is_empty(),
                missing_modules,
            }
        })
        .collect::<Vec<_>>();
    let compatible_version = raw.major == 3 && matches!(raw.minor, 11 | 12);
    let ready = compatible_version && capabilities.iter().all(|value| value.ready);
    PythonRuntimeInspection {
        executable,
        version: raw.version,
        compatible_version,
        capabilities,
        ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python_modules(available: bool) -> BTreeMap<String, bool> {
        [
            "torch",
            "sentence_transformers",
            "transformers",
            "datasets",
            "accelerate",
            "numpy",
            "sklearn",
            "psutil",
            "onnxruntime_genai",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), available))
        .collect()
    }

    fn scientific_project() -> ExternalProjectSnapshot {
        use encoder_experiment_core::domain::{
            BackendIdentity, EncoderTaskKind, ExternalArtifactIdentity, ModelArtifactIdentity,
        };
        ExternalProjectSnapshot::create(
            "persisted project",
            EncoderTaskKind::RetrievalRanking,
            "revision",
            format!("sha256:{}", "a".repeat(64)),
            BackendIdentity::new(
                "nomos",
                "nomos-ranking-v3",
                format!("sha256:{}", "b".repeat(64)),
            )
            .unwrap(),
            vec![
                ExternalArtifactIdentity::new(
                    "train.jsonl",
                    EvidenceRole::Training,
                    10,
                    format!("sha256:{}", "c".repeat(64)),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "development.jsonl",
                    EvidenceRole::Development,
                    10,
                    format!("sha256:{}", "e".repeat(64)),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed.jsonl",
                    EvidenceRole::SealedAcceptance,
                    10,
                    format!("sha256:{}", "f".repeat(64)),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new(
                "baseline",
                "sentence-transformers",
                10,
                format!("sha256:{}", "d".repeat(64)),
            )
            .unwrap(),
            serde_json::json!({"adapter_protocol":"nomos-ranking-v3"}),
            Utc::now(),
        )
        .unwrap()
    }

    fn backend() -> NomosWorkspaceArgs {
        NomosWorkspaceArgs {
            workspace: "C:/isolated/nomos".into(),
            python: "C:/python/python.exe".into(),
        }
    }

    #[test]
    fn managed_intents_inject_the_fixed_bound_backend() {
        let run_id = Uuid::new_v4();
        let command = managed_command(ManagedOptimizeCommand::Status { run_id }, backend());
        match command {
            EncoderOptimizeCommand::Status(args) => {
                assert_eq!(args.run_id, run_id);
                assert_eq!(
                    args.backend.workspace,
                    std::path::PathBuf::from("C:/isolated/nomos")
                );
                assert_eq!(
                    args.backend.python,
                    std::path::PathBuf::from("C:/python/python.exe")
                );
            }
            _ => panic!("managed status mapped to the wrong lifecycle command"),
        }

        let command = managed_command(
            ManagedOptimizeCommand::Cancel {
                run_id,
                reason: "operator stop".into(),
            },
            backend(),
        );
        assert!(matches!(
            command,
            EncoderOptimizeCommand::Cancel(EncoderOptimizeCancelArgs { run_id: id, .. }) if id == run_id
        ));
    }

    #[test]
    fn python_runtime_requires_supported_version_and_every_execution_capability() {
        let ready = python_runtime_inspection(
            "python".into(),
            RawPythonInspection {
                version: "3.12.4".into(),
                major: 3,
                minor: 12,
                modules: python_modules(true),
            },
        );
        assert!(ready.ready);
        assert!(ready.compatible_version);
        assert!(ready.capabilities.iter().all(|value| value.ready));

        let mut modules = python_modules(true);
        modules.insert("onnxruntime_genai".into(), false);
        let incomplete = python_runtime_inspection(
            "python".into(),
            RawPythonInspection {
                version: "3.10.9".into(),
                major: 3,
                minor: 10,
                modules,
            },
        );
        assert!(!incomplete.ready);
        assert!(!incomplete.compatible_version);
        let agent = incomplete
            .capabilities
            .iter()
            .find(|value| value.key == "agent_evaluation")
            .unwrap();
        assert!(!agent.ready);
        assert_eq!(agent.missing_modules, ["onnxruntime_genai"]);
        assert_eq!(
            missing_python_packages(&incomplete).unwrap(),
            ["onnxruntime-genai>=0.8"]
        );
    }

    #[tokio::test]
    async fn bound_project_is_loaded_by_persisted_identity_not_a_fresh_adapter_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("scientific.sqlite");
        let url = format!("sqlite://{}", database.to_string_lossy().replace('\\', "/"));
        let store = SqliteExperimentStore::connect(&url).await.unwrap();
        let project = scientific_project();
        store.create_project(project.clone()).await.unwrap();
        let binding = ScientificBinding::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
            AdapterBinding {
                key: "nomos".into(),
                protocol: "nomos-ranking-v3".into(),
                configuration_fingerprint: format!("sha256:{}", "b".repeat(64)),
            },
            RuntimeBinding {
                kind: RuntimeKind::ExternalIsolated,
                location: "runtime".into(),
                executable: Some("python".into()),
                project_snapshot: BoundIdentity {
                    id: project.id.to_string(),
                    fingerprint: project.fingerprint.clone(),
                },
            },
            ScientificStoreBinding {
                database_path: "runs/scientific.sqlite".into(),
                schema: BoundIdentity {
                    id: SCHEMA_ID.into(),
                    fingerprint: schema_fingerprint(),
                },
                snapshot_fingerprint: None,
                snapshot_bytes: None,
            },
            "operator",
            "test persisted binding",
            Utc::now(),
        )
        .unwrap();
        assert_eq!(load_bound_project(&store, &binding).await.unwrap(), project);
        store.pool().close().await;
    }

    #[test]
    fn sqlite_urls_remove_windows_extended_path_markers() {
        assert_eq!(
            sqlite_file_url(std::path::Path::new(
                r"\\?\C:\EncoderGym\runs\scientific.sqlite"
            )),
            "sqlite://C:/EncoderGym/runs/scientific.sqlite"
        );
        assert_eq!(
            sqlite_file_url(std::path::Path::new(
                r"\\?\UNC\server\share\scientific.sqlite"
            )),
            "sqlite:////server/share/scientific.sqlite"
        );
    }
}
