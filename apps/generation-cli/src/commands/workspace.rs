use crate::{
    cli::{
        EncoderOptimizeAuthorizeArgs, EncoderOptimizeCancelArgs, EncoderOptimizeCommand,
        EncoderOptimizeManifestArgs, EncoderOptimizeRunArgs, ManagedOptimizeCommand,
        ManagedProviderCommand, NomosWorkspaceArgs, WorkspaceCommand,
    },
    presentation::print,
};
use chrono::Utc;
use encoder_campaign_core::optimization::OptimizationRunState;
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalProjectSnapshot},
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_sqlite::{SCHEMA_ID, SqliteExperimentStore, schema_fingerprint};
use project_workspace_core::{
    AdapterBinding, BoundIdentity, ProviderAuthentication, ProviderCatalog, ProviderConfiguration,
    ProviderKind, ProviderLimits, ProviderRole, ReadinessAction, ReadinessCategory, ReadinessCheck,
    ReadinessReport, ReadinessState, RuntimeBinding, RuntimeKind, ScientificBinding,
    ScientificStoreBinding, SecretReference,
};
use project_workspace_local::{
    backfill_nomos, create_workspace, import_dataset, inspect_dataset, inspect_model,
    open_workspace, record_provider_catalog, record_scientific_binding, upgrade_workspace,
};
use uuid::Uuid;

use super::encoder_optimize::ManagedOptimizationReadiness;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedReadinessOutput {
    report: ReadinessReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_preview: Option<ManagedOptimizationReadiness>,
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
        WorkspaceCommand::Readiness { folder, manifest } => {
            print(&readiness(&folder, manifest.as_deref()).await?)
        }
        WorkspaceCommand::Optimize { folder, command } => managed_optimize(&folder, *command).await,
        WorkspaceCommand::Providers { folder, command } => providers(&folder, command).await,
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

async fn providers(
    folder: &std::path::Path,
    command: ManagedProviderCommand,
) -> anyhow::Result<()> {
    let workspace = open_workspace(folder, true).await?;
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

async fn managed_optimize(
    folder: &std::path::Path,
    command: ManagedOptimizeCommand,
) -> anyhow::Result<()> {
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
    let project = verify_nomos_runtime(binding, catalog).await?;
    let store = open_bound_store(&workspace.folder, binding).await?;
    let stored_project = store
        .get_project(project.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("The bound scientific project snapshot is missing."))?;
    anyhow::ensure!(
        stored_project == project,
        "The bound runtime and scientific store project snapshots differ."
    );
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
    .await
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
    // Open without artifact rehashing first so an integrity failure can be
    // represented as a project-scoped fact instead of losing the whole report.
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace.model_catalog.as_ref();
    let baseline_revision_id = catalog.map(|value| value.active_baseline_revision_id);
    let mut checks = Vec::new();

    checks.push(match open_workspace(folder, true).await {
        Ok(_) => check(
            "workspace.integrity",
            ReadinessCategory::Workspace,
            ReadinessState::Ready,
            true,
            "Workspace artifacts are intact",
            "The manifest, project registry, baseline inventory, and imported dataset bytes match their immutable identities.",
            None,
        )?,
        Err(error) => check(
            "workspace.integrity",
            ReadinessCategory::Workspace,
            ReadinessState::Blocked,
            true,
            "Workspace integrity failed",
            plain_error(&error),
            Some(action("verify-workspace", "Inspect workspace integrity")?),
        )?,
    });

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

            let runtime_result = verify_nomos_runtime(binding, catalog).await;
            match runtime_result {
                Ok(project) => {
                    checks.push(check(
                        "scientific.runtime",
                        ReadinessCategory::Scientific,
                        ReadinessState::Ready,
                        true,
                        "Compiled runtime matches the binding",
                        format!(
                            "Adapter {} {} reproduced project snapshot {}.",
                            binding.adapter.key, binding.adapter.protocol, project.id
                        ),
                        None,
                    )?);
                    runtime_project = Some(project);
                }
                Err(error) => checks.push(check(
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
                )?),
            }

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

            let (project_matches, project_evidence) = match (&store, &runtime_project) {
                (Some(store), Some(project)) => match store.get_project(project.id).await {
                    Ok(Some(stored)) if stored == *project => (
                        true,
                        "The runtime and scientific store contain the same immutable project snapshot."
                            .into(),
                    ),
                    Ok(_) => (
                        false,
                        "The scientific store does not contain the exact runtime project snapshot."
                            .into(),
                    ),
                    Err(error) => (false, plain_error(&error)),
                },
                _ => (
                    false,
                    "Both a verified runtime and its exact registered project snapshot are required."
                        .into(),
                ),
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

    let mut launch_preview = None;
    match (manifest, store.as_ref(), runtime_project.as_ref()) {
        (None, _, _) => {
            checks.push(check(
                "optimization.preview",
                ReadinessCategory::Optimization,
                ReadinessState::ActionRequired,
                true,
                "No reviewed optimization request is selected",
                "Select an exact approved training snapshot, successor benchmark generation, candidate set, and finite budget to resolve a launch preview.",
                Some(action("prepare-optimization", "Prepare optimization")?),
            )?);
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
        launch_preview,
    })
}

async fn verify_nomos_runtime(
    binding: &ScientificBinding,
    catalog: &project_workspace_core::ModelCatalog,
) -> anyhow::Result<ExternalProjectSnapshot> {
    anyhow::ensure!(
        binding.adapter.key == "nomos",
        "This executable has no compiled adapter for '{}'.",
        binding.adapter.key
    );
    let executable = binding.runtime.executable.as_ref().ok_or_else(|| {
        anyhow::anyhow!("The runtime binding predates executable selection; rebind it.")
    })?;
    let backend = NomosBackend::open(&binding.runtime.location, executable)?;
    let identity = backend.identity();
    anyhow::ensure!(
        identity.name == binding.adapter.key
            && identity.protocol_version == binding.adapter.protocol
            && identity.configuration_fingerprint == binding.adapter.configuration_fingerprint,
        "The compiled adapter identity changed since binding."
    );
    let project = backend.project_snapshot()?;
    anyhow::ensure!(
        project.id.to_string() == binding.runtime.project_snapshot.id
            && project.fingerprint == binding.runtime.project_snapshot.fingerprint,
        "The isolated runtime project snapshot changed since binding."
    );
    let runtime_root = std::path::Path::new(&binding.runtime.location).canonicalize()?;
    let runtime_baseline = inspect_model(&runtime_root.join(&project.baseline_model.key))?;
    let active = catalog.active_model();
    anyhow::ensure!(
        runtime_baseline.fingerprint == active.fingerprint
            && runtime_baseline.bytes == active.bytes
            && runtime_baseline.format == active.format,
        "The runtime baseline no longer matches the active managed model."
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

fn bound_store_url(
    workspace_root: &std::path::Path,
    binding: &ScientificBinding,
) -> anyhow::Result<String> {
    let root = workspace_root.canonicalize()?;
    let path = root.join(&binding.store.database_path).canonicalize()?;
    anyhow::ensure!(
        path.starts_with(&root),
        "Scientific store escapes the managed project."
    );
    Ok(format!(
        "sqlite://{}",
        path.to_string_lossy().replace('\\', "/")
    ))
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
            executable: Some(python.to_string_lossy().into_owned()),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
