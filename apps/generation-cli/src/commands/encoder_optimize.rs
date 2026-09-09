mod report;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignState, CampaignStore, ProductionCampaign,
    adopt_run_event, bind_generation_event, complete_campaign_event, first_campaign_event,
    optimization::{
        OptimizationArtifactBinding, OptimizationEventKind, OptimizationLaunchStore,
        OptimizationRunState, OptimizationView, ProductionOptimizationDefinition,
        ProductionOptimizationRun, first_optimization_event, replay_optimization,
    },
    prepare_iteration_event, record_development_event, record_sealed_authorization_event,
    replay_campaign, start_run_event,
};
use encoder_experiment_core::{
    domain::{OptimizationBudget, TrainingCandidate},
    journal::{CandidateExecutionState, ExperimentRunState},
    ports::{EncoderTaskBackend, ExperimentStore},
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{
    ports::NativeRepairTrainingStore,
    proposal::{RepairBudget, RepairProposal},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysinfo::{Pid, System};
use uuid::Uuid;
use workflow_core::benchmark_generation::{BenchmarkGenerationState, BenchmarkGenerationView};

use crate::{
    cli::{
        EncoderOptimizeAuthorizeArgs, EncoderOptimizeCancelArgs, EncoderOptimizeCommand,
        EncoderOptimizeManifestArgs, EncoderOptimizeRunArgs, NomosWorkspaceArgs,
    },
    commands::{
        experiment::{
            database_file_path, ensure_database_belongs_to_managed_root,
            ensure_database_belongs_to_workspace, load_persisted_view,
        },
        production_campaign::{
            CampaignContext, campaign_provenance, finalize_campaign_iteration, load_generation,
        },
        production_repair::{
            build_training_snapshot, load_approved_delta_context, load_approved_delta_facts,
        },
    },
    presentation,
};

const MANIFEST_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 1_048_576;
const EXECUTION_LEASE_SCHEMA_VERSION: u32 = 1;
const MAX_EXECUTION_LEASE_BYTES: u64 = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionLeaseOwner {
    schema_version: u32,
    process_id: u32,
    process_started_at: u64,
    nonce: Uuid,
}

#[derive(Debug)]
struct OptimizationExecutionLease {
    directory: PathBuf,
    owner: ExecutionLeaseOwner,
}

impl OptimizationExecutionLease {
    fn for_command(
        command: &EncoderOptimizeCommand,
        database_url: &str,
    ) -> anyhow::Result<Option<Self>> {
        match command {
            EncoderOptimizeCommand::Resume(args) => {
                Self::acquire(database_url, args.run_id).map(Some)
            }
            _ => Ok(None),
        }
    }

    fn acquire(database_url: &str, run_id: Uuid) -> anyhow::Result<Self> {
        let database = database_file_path(database_url)?;
        let parent = database
            .parent()
            .context("experiment database path has no parent")?;
        let directory = parent.join(format!(".encoder-optimization-{run_id}.lease"));
        let process_id = std::process::id();
        let owner = ExecutionLeaseOwner {
            schema_version: EXECUTION_LEASE_SCHEMA_VERSION,
            process_id,
            process_started_at: process_started_at(process_id)
                .context("could not inspect the optimization runner process")?,
            nonce: Uuid::new_v4(),
        };

        for _ in 0..3 {
            let staged = parent.join(format!(
                ".encoder-optimization-{run_id}.lease-staged-{}",
                owner.nonce
            ));
            fs::create_dir(&staged).context("could not stage optimization execution lease")?;
            let owner_path = staged.join("owner.json");
            let bytes = serde_json::to_vec(&owner)?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&owner_path)
                .context("could not write optimization execution lease")?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            match fs::rename(&staged, &directory) {
                Ok(()) => return Ok(Self { directory, owner }),
                Err(_error) if directory.is_dir() => {
                    fs::remove_file(&owner_path)?;
                    fs::remove_dir(&staged)?;
                    let current = read_execution_lease(&directory)?;
                    if execution_owner_is_active(&current) {
                        anyhow::bail!(
                            "This optimization stage is already running in another local process. Reopen its status instead of starting it again."
                        );
                    }
                    let stale = parent.join(format!(
                        ".encoder-optimization-{run_id}.lease-stale-{}",
                        Uuid::new_v4()
                    ));
                    match fs::rename(&directory, &stale) {
                        Ok(()) => remove_execution_lease_directory(&stale)?,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(error)
                                .context("could not recover stale optimization execution lease");
                        }
                    }
                }
                Err(error) => {
                    fs::remove_file(&owner_path)?;
                    fs::remove_dir(&staged)?;
                    return Err(error).context("could not acquire optimization execution lease");
                }
            }
        }
        anyhow::bail!("optimization execution ownership changed repeatedly; inspect run status")
    }
}

impl Drop for OptimizationExecutionLease {
    fn drop(&mut self) {
        if matches!(read_execution_lease(&self.directory), Ok(ref owner) if owner == &self.owner) {
            let _ = remove_execution_lease_directory(&self.directory);
        }
    }
}

fn read_execution_lease(directory: &Path) -> anyhow::Result<ExecutionLeaseOwner> {
    let directory_metadata = fs::symlink_metadata(directory)
        .context("could not inspect optimization execution lease")?;
    anyhow::ensure!(
        directory_metadata.is_dir() && !directory_metadata.file_type().is_symlink(),
        "optimization execution lease is not a plain local directory"
    );
    let mut entries = fs::read_dir(directory)
        .context("could not inspect optimization execution lease")?
        .collect::<Result<Vec<_>, _>>()?;
    anyhow::ensure!(
        entries.len() == 1
            && entries
                .pop()
                .is_some_and(|entry| entry.file_name() == "owner.json"),
        "optimization execution lease has unexpected contents"
    );
    let owner_path = directory.join("owner.json");
    let metadata = fs::symlink_metadata(&owner_path)?;
    anyhow::ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= MAX_EXECUTION_LEASE_BYTES,
        "optimization execution lease is invalid"
    );
    let owner: ExecutionLeaseOwner = serde_json::from_slice(&fs::read(owner_path)?)?;
    anyhow::ensure!(
        owner.schema_version == EXECUTION_LEASE_SCHEMA_VERSION,
        "optimization execution lease uses an unsupported schema"
    );
    Ok(owner)
}

fn remove_execution_lease_directory(directory: &Path) -> anyhow::Result<()> {
    fs::remove_file(directory.join("owner.json"))?;
    fs::remove_dir(directory)?;
    Ok(())
}

fn process_started_at(process_id: u32) -> Option<u64> {
    System::new_all()
        .process(Pid::from_u32(process_id))
        .map(sysinfo::Process::start_time)
}

fn execution_owner_is_active(owner: &ExecutionLeaseOwner) -> bool {
    process_started_at(owner.process_id) == Some(owner.process_started_at)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OptimizeAdapter {
    Nomos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovalMode {
    ExplicitSealedUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinalDecisionPolicy {
    StrictMetricContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExistingExperimentManifest {
    protocol_id: Uuid,
    protocol_fingerprint: String,
    run_id: Uuid,
    run_head_fingerprint: String,
}

/// This intentionally begins after both human review boundaries. Earlier repair commands own
/// proposal and delta review; optimize consumes their immutable approved training snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptimizeManifest {
    schema_version: u32,
    name: String,
    adapter: OptimizeAdapter,
    project_source_revision: String,
    training_snapshot_id: Uuid,
    training_snapshot_fingerprint: String,
    training_snapshot_specification_fingerprint: String,
    benchmark_generation_id: Uuid,
    benchmark_generation_fingerprint: String,
    approval_mode: ApprovalMode,
    selection_policy: DevelopmentSelectionRule,
    final_decision_policy: FinalDecisionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    existing_experiment: Option<ExistingExperimentManifest>,
}

struct ResolvedLaunch {
    manifest: OptimizeManifest,
    manifest_fingerprint: String,
    definition: ProductionOptimizationDefinition,
    adopted_protocol_id: Option<Uuid>,
    adopted_run_id: Option<Uuid>,
}

struct LaunchContext {
    definition: ProductionOptimizationDefinition,
    run: ProductionOptimizationRun,
    view: OptimizationView,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedOptimizationReadiness {
    pub manifest_fingerprint: String,
    pub specification_fingerprint: String,
    pub project_id: Uuid,
    pub project_fingerprint: String,
    pub training_snapshot_id: Uuid,
    pub benchmark_generation_id: Uuid,
    pub candidate_count: usize,
    pub budget: CampaignBudget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_run: Option<ManagedExistingOptimization>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedExistingOptimization {
    pub run_id: Uuid,
    pub state: OptimizationRunState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedOptimizationAuthority {
    pub proposal_id: Uuid,
    pub selection_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub training_snapshot_id: Option<Uuid>,
    pub benchmark_generation_id: Uuid,
    pub hypotheses: Vec<String>,
    pub candidate_count: usize,
    pub base_training_inputs: usize,
    pub delta_rows: usize,
    pub budget: RepairBudget,
    pub valid_until: chrono::DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedOptimizationPreparation {
    pub manifest_path: PathBuf,
    pub manifest_name: String,
    pub authority: ManagedOptimizationAuthority,
    pub readiness: ManagedOptimizationReadiness,
    pub created_training_snapshot: bool,
    pub external_calls: u32,
}

/// Resolve the same immutable definition consumed by `start`, without writing
/// state or opening a native adapter. Managed-project readiness uses this
/// instead of reproducing optimization policy in the presentation layer.
pub(crate) async fn managed_readiness(
    store: &SqliteExperimentStore,
    manifest: &Path,
) -> anyhow::Result<ManagedOptimizationReadiness> {
    let resolved = resolve_manifest(store, manifest).await?;
    managed_readiness_from_resolved(store, &resolved).await
}

async fn managed_readiness_from_resolved(
    store: &SqliteExperimentStore,
    resolved: &ResolvedLaunch,
) -> anyhow::Result<ManagedOptimizationReadiness> {
    let existing_run = match store
        .find_optimization_by_manifest(resolved.manifest_fingerprint.clone())
        .await?
    {
        Some((definition, run)) => {
            if definition.specification_fingerprint != resolved.definition.specification_fingerprint
            {
                anyhow::bail!(
                    "manifest identity already belongs to a different resolved optimization"
                );
            }
            let context = load_launch(store, run.id).await?;
            Some(ManagedExistingOptimization {
                run_id: run.id,
                state: context.view.state,
            })
        }
        None => None,
    };
    Ok(ManagedOptimizationReadiness {
        manifest_fingerprint: resolved.manifest_fingerprint.clone(),
        specification_fingerprint: resolved.definition.specification_fingerprint.clone(),
        project_id: resolved.definition.project.id,
        project_fingerprint: resolved.definition.project.fingerprint.clone(),
        training_snapshot_id: resolved.definition.training_snapshot.id,
        benchmark_generation_id: resolved.definition.benchmark.generation_id,
        candidate_count: resolved.definition.candidates.len(),
        budget: resolved.definition.campaign_budget.clone(),
        existing_run,
    })
}

/// Reproduce a persisted run's exact launch summary without requiring its
/// source manifest file or an unexpired pre-launch approval. Once reserved,
/// the immutable optimization journal is the recovery authority.
pub(crate) async fn managed_run_readiness(
    store: &SqliteExperimentStore,
    run_id: Uuid,
) -> anyhow::Result<ManagedOptimizationReadiness> {
    let context = load_launch(store, run_id).await?;
    Ok(ManagedOptimizationReadiness {
        manifest_fingerprint: context.definition.manifest_fingerprint.clone(),
        specification_fingerprint: context.definition.specification_fingerprint.clone(),
        project_id: context.definition.project.id,
        project_fingerprint: context.definition.project.fingerprint.clone(),
        training_snapshot_id: context.definition.training_snapshot.id,
        benchmark_generation_id: context.definition.benchmark.generation_id,
        candidate_count: context.definition.candidates.len(),
        budget: context.definition.campaign_budget.clone(),
        existing_run: Some(ManagedExistingOptimization {
            run_id: context.run.id,
            state: context.view.state,
        }),
    })
}

/// Select recoverable project state without confusing recency with activity.
/// There may be many historical terminal runs, but more than one active run is
/// an invariant violation that must be resolved rather than hidden by the UI.
pub(crate) async fn managed_project_recovery(
    store: &SqliteExperimentStore,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
) -> anyhow::Result<Option<ManagedOptimizationReadiness>> {
    let mut active = Vec::new();
    let mut newest_terminal = None;
    for run_id in store.optimization_run_ids_for_project(project.id).await? {
        let preview = managed_run_readiness(store, run_id).await?;
        anyhow::ensure!(
            preview.project_id == project.id && preview.project_fingerprint == project.fingerprint,
            "optimization recovery crossed scientific project authority"
        );
        let state = preview
            .existing_run
            .as_ref()
            .context("persisted optimization recovery has no run state")?
            .state;
        if matches!(
            state,
            OptimizationRunState::Planned | OptimizationRunState::CampaignActive
        ) {
            active.push(preview);
        } else if newest_terminal.is_none() {
            newest_terminal = Some(preview);
        }
    }
    anyhow::ensure!(
        active.len() <= 1,
        "More than one active optimization exists for this scientific project. Inspect and resolve the run journals before continuing."
    );
    Ok(active.pop().or(newest_terminal))
}

/// Derive the one approved successor repair that still binds the current,
/// active, unused benchmark generation. Historical selections remain
/// inspectable but cannot become a new launch merely because their rows still
/// exist in the scientific store.
pub(crate) async fn managed_authority(
    store: &SqliteExperimentStore,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
) -> anyhow::Result<Option<ManagedOptimizationAuthority>> {
    let mut current = Vec::new();
    for selection_id in store
        .native_delta_selection_ids_for_project(project.id)
        .await?
    {
        let approved = load_approved_delta_facts(store, selection_id).await?;
        approved
            .proposal
            .context
            .execution_project
            .verify(project)?;
        if approved.proposal.expires_at <= Utc::now() {
            continue;
        }
        let Ok((generation, view)) =
            load_generation(store, approved.proposal.context.benchmark.generation_id).await
        else {
            continue;
        };
        if require_active_generation(&view, &generation).is_err() {
            continue;
        }
        let benchmark = &approved.proposal.context.benchmark;
        let observed_development = generation
            .development_suites
            .iter()
            .map(|value| {
                (
                    value.suite_key.clone(),
                    value.bundle.development_suite_fingerprint.clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        if generation.fingerprint != benchmark.generation_fingerprint
            || view.last_sequence != benchmark.journal_sequence
            || view.last_event_fingerprint != benchmark.journal_head_fingerprint
            || observed_development != benchmark.development_suite_fingerprints
            || generation.sealed_suite_id != benchmark.sealed_suite_id
            || generation.sealed_suite_fingerprint != benchmark.sealed_suite_fingerprint
        {
            continue;
        }
        let training_snapshot = store
            .get_native_repair_training_snapshot_for_selection(selection_id)
            .await?;
        current.push(ManagedOptimizationAuthority {
            proposal_id: approved.proposal.id,
            selection_id,
            training_snapshot_id: training_snapshot.map(|value| value.id),
            benchmark_generation_id: generation.id,
            hypotheses: approved
                .proposal
                .candidates
                .iter()
                .map(|value| value.hypothesis.clone())
                .collect(),
            candidate_count: approved.proposal.candidates.len(),
            base_training_inputs: approved.proposal.context.base_training_inputs.len(),
            delta_rows: approved.candidate_set.rows.len(),
            budget: approved.proposal.budget,
            valid_until: approved.proposal.expires_at,
        });
    }
    anyhow::ensure!(
        current.len() <= 1,
        "More than one approved repair selection claims current optimization authority. Resolve the scientific lineage before preparing a run."
    );
    Ok(current.pop())
}

/// Freeze the current approved native repair into its owner-managed logical
/// training snapshot, then write one content-addressed strict manifest below
/// the managed workspace. This performs no training, evaluation, provider
/// call, or sealed-evidence exposure.
pub(crate) async fn prepare_managed(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
    managed_root: &Path,
) -> anyhow::Result<ManagedOptimizationPreparation> {
    let mut authority = managed_authority(store, project)
        .await?
        .context("No current approved repair is available for optimization preparation")?;
    if let Some(recovered) = recover_managed(store, project, managed_root, &authority).await? {
        return Ok(recovered);
    }
    let created_training_snapshot = authority.training_snapshot_id.is_none();
    let prepared = build_training_snapshot(store, backend, authority.selection_id).await?;
    let snapshot = prepared.snapshot;
    let approved = prepared.context;
    anyhow::ensure!(
        approved.project.id == project.id && approved.project.fingerprint == project.fingerprint,
        "The current repair belongs to another scientific project snapshot."
    );
    authority.training_snapshot_id = Some(snapshot.id);

    let (name, manifest) = managed_manifest(project, &snapshot, &approved.proposal);
    let (manifest_path, readiness) =
        publish_managed_manifest(store, managed_root, &manifest, snapshot, &approved).await?;
    Ok(ManagedOptimizationPreparation {
        manifest_path,
        manifest_name: name,
        authority,
        readiness,
        created_training_snapshot,
        external_calls: approved.proposal.budget.maximum_external_calls,
    })
}

/// Recover the exact generated definition after a desktop restart without
/// replaying native artifacts or mutating scientific state. Persisted owner
/// objects are still fully reproduced and the on-disk definition must be the
/// exact content-addressed encoding implied by those facts.
pub(crate) async fn recover_managed(
    store: &SqliteExperimentStore,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
    managed_root: &Path,
    authority: &ManagedOptimizationAuthority,
) -> anyhow::Result<Option<ManagedOptimizationPreparation>> {
    let Some(snapshot_id) = authority.training_snapshot_id else {
        return Ok(None);
    };
    let approved = load_approved_delta_facts(store, authority.selection_id).await?;
    anyhow::ensure!(
        approved.project.id == project.id && approved.project.fingerprint == project.fingerprint,
        "The current repair belongs to another scientific project snapshot."
    );
    let snapshot = store
        .get_native_repair_training_snapshot(snapshot_id)
        .await?
        .context("The approved repair's training snapshot no longer exists")?;
    snapshot.validate_against(
        &approved.project,
        &approved.proposal,
        &approved.candidate_set,
        &approved.report,
        &approved.approval,
        approved.approval_predecessor.as_ref(),
        &approved.selection,
    )?;
    let (name, manifest) = managed_manifest(project, &snapshot, &approved.proposal);
    let (manifest_path, contents) = managed_manifest_file(managed_root, &manifest)?;
    if !manifest_path.exists() {
        return Ok(None);
    }
    verify_managed_definitions_directory(managed_root, false)?;
    anyhow::ensure!(
        fs::read(&manifest_path)? == contents,
        "The prepared optimization definition no longer matches its persisted scientific authority."
    );
    let resolved = resolve_loaded(
        store,
        manifest,
        snapshot,
        &approved.project,
        &approved.proposal,
    )
    .await?;
    let readiness = managed_readiness_from_resolved(store, &resolved).await?;
    Ok(Some(ManagedOptimizationPreparation {
        manifest_path,
        manifest_name: name,
        authority: authority.clone(),
        readiness,
        created_training_snapshot: false,
        external_calls: approved.proposal.budget.maximum_external_calls,
    }))
}

fn managed_manifest(
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
    snapshot: &encoder_repair_core::training::NativeRepairTrainingSnapshot,
    proposal: &RepairProposal,
) -> (String, OptimizeManifest) {
    let name = optimization_name(&project.name, &proposal.candidates);
    let manifest = OptimizeManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        name: name.clone(),
        adapter: OptimizeAdapter::Nomos,
        project_source_revision: project.source_revision.clone(),
        training_snapshot_id: snapshot.id,
        training_snapshot_fingerprint: snapshot.fingerprint.clone(),
        training_snapshot_specification_fingerprint: snapshot.specification_fingerprint.clone(),
        benchmark_generation_id: proposal.context.benchmark.generation_id,
        benchmark_generation_fingerprint: proposal.context.benchmark.generation_fingerprint.clone(),
        approval_mode: ApprovalMode::ExplicitSealedUse,
        selection_policy: DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
        final_decision_policy: FinalDecisionPolicy::StrictMetricContract,
        existing_experiment: None,
    };
    (name, manifest)
}

async fn publish_managed_manifest(
    store: &SqliteExperimentStore,
    managed_root: &Path,
    manifest: &OptimizeManifest,
    snapshot: encoder_repair_core::training::NativeRepairTrainingSnapshot,
    approved: &crate::commands::production_repair::ApprovedDeltaContext,
) -> anyhow::Result<(PathBuf, ManagedOptimizationReadiness)> {
    validate_manifest(manifest)?;
    let resolved = resolve_loaded(
        store,
        manifest.to_owned(),
        snapshot,
        &approved.project,
        &approved.proposal,
    )
    .await?;
    let readiness = managed_readiness_from_resolved(store, &resolved).await?;
    let (manifest_path, contents) = managed_manifest_file(managed_root, manifest)?;
    let definitions = manifest_path
        .parent()
        .context("managed optimization definition has no parent")?;
    verify_managed_definitions_directory(managed_root, true)?;
    anyhow::ensure!(
        definitions.is_dir(),
        "managed run definitions are unavailable"
    );
    write_content_addressed(&manifest_path, &contents)?;
    Ok((manifest_path, readiness))
}

fn managed_manifest_file(
    managed_root: &Path,
    manifest: &OptimizeManifest,
) -> anyhow::Result<(PathBuf, Vec<u8>)> {
    validate_manifest(manifest)?;
    let manifest_fingerprint = artifact_core::fingerprint(manifest)?;
    let manifest_name = format!(
        "optimization-{}.toml",
        manifest_fingerprint
            .strip_prefix("sha256:")
            .context("optimization manifest fingerprint is malformed")?
    );
    let contents = toml::to_string_pretty(manifest)
        .context("could not encode the managed optimization definition")?
        .into_bytes();
    Ok((
        managed_root
            .join("runs")
            .join("definitions")
            .join(manifest_name),
        contents,
    ))
}

fn verify_managed_definitions_directory(managed_root: &Path, create: bool) -> anyhow::Result<()> {
    let canonical_root = managed_root
        .canonicalize()
        .context("could not resolve managed project root")?;
    let runs = managed_root.join("runs");
    let definitions = runs.join("definitions");
    for directory in [&runs, &definitions] {
        if !directory.exists() {
            if !create {
                return Ok(());
            }
            fs::create_dir(directory).context("could not create managed run definitions")?;
        }
        let metadata = fs::symlink_metadata(directory)?;
        anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "managed run definitions must be real project directories"
        );
        anyhow::ensure!(
            directory.canonicalize()?.starts_with(&canonical_root),
            "managed run definitions escaped the project workspace"
        );
    }
    Ok(())
}

fn optimization_name(
    project_name: &str,
    hypotheses: &[encoder_repair_core::proposal::RepairCandidateHypothesis],
) -> String {
    let suffix = hypotheses
        .first()
        .map(|value| value.key.as_str())
        .unwrap_or("approved repair");
    let value = format!("{project_name} · {suffix}");
    value.chars().take(120).collect()
}

fn write_content_addressed(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::ensure!(
            fs::read(path)? == contents,
            "A managed optimization definition exists under the same identity with different bytes."
        );
        return Ok(());
    }
    let parent = path
        .parent()
        .context("managed optimization definition has no parent")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .context("could not stage managed optimization definition")?;
    staged.write_all(contents)?;
    staged.as_file().sync_all()?;
    match staged.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::ensure!(
                fs::read(path)? == contents,
                "A managed optimization definition exists under the same identity with different bytes."
            );
            Ok(())
        }
        Err(error) => Err(error.error).context("could not publish managed optimization definition"),
    }
}

pub async fn execute(command: EncoderOptimizeCommand, database_url: &str) -> anyhow::Result<()> {
    let args = backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &args.workspace)?;
    let workspace = args.workspace.clone();
    let python = args.python.clone();
    let _execution_lease = OptimizationExecutionLease::for_command(&command, database_url)?;
    let store = command.database_access().production(database_url).await?;
    match command {
        EncoderOptimizeCommand::Doctor(args) => {
            let backend = NomosBackend::open(&workspace, python)?;
            doctor(&store, &backend, args).await
        }
        command => {
            execute_lifecycle(command, &store, || {
                NomosBackend::open(&workspace, python).map_err(Into::into)
            })
            .await
        }
    }
}

/// Fixed managed-project composition. The caller supplies values reproduced
/// from an active scientific binding; arbitrary database roots remain invalid.
pub(crate) async fn execute_managed(
    command: EncoderOptimizeCommand,
    database_url: &str,
    managed_root: &Path,
    project_id: Uuid,
    project_fingerprint: &str,
) -> anyhow::Result<()> {
    ensure_database_belongs_to_managed_root(database_url, managed_root)?;
    let args = backend_args(&command);
    let workspace = args.workspace.clone();
    let python = args.python.clone();
    let store = command.database_access().production(database_url).await?;
    ensure_managed_command_scope(&command, &store, project_id, project_fingerprint).await?;
    let _execution_lease = OptimizationExecutionLease::for_command(&command, database_url)?;
    match command {
        EncoderOptimizeCommand::Doctor(args) => {
            let backend = NomosBackend::open(&workspace, python)?;
            doctor(&store, &backend, args).await
        }
        command => {
            execute_lifecycle(command, &store, || {
                NomosBackend::open(&workspace, python).map_err(Into::into)
            })
            .await
        }
    }
}

async fn ensure_managed_command_scope(
    command: &EncoderOptimizeCommand,
    store: &SqliteExperimentStore,
    project_id: Uuid,
    project_fingerprint: &str,
) -> anyhow::Result<()> {
    let project = match command {
        EncoderOptimizeCommand::Preview(args) | EncoderOptimizeCommand::Start(args) => {
            let preview = managed_readiness(store, &args.manifest).await?;
            (preview.project_id, preview.project_fingerprint)
        }
        EncoderOptimizeCommand::Status(args)
        | EncoderOptimizeCommand::Inspect(args)
        | EncoderOptimizeCommand::ReviewRepair(args)
        | EncoderOptimizeCommand::ReviewDelta(args)
        | EncoderOptimizeCommand::Resume(args)
        | EncoderOptimizeCommand::Doctor(args)
        | EncoderOptimizeCommand::Provenance(args)
        | EncoderOptimizeCommand::Report(args) => {
            let context = load_launch(store, args.run_id).await?;
            (
                context.definition.project.id,
                context.definition.project.fingerprint,
            )
        }
        EncoderOptimizeCommand::AuthorizeExternal(args)
        | EncoderOptimizeCommand::AuthorizeSealed(args) => {
            let context = load_launch(store, args.run_id).await?;
            (
                context.definition.project.id,
                context.definition.project.fingerprint,
            )
        }
        EncoderOptimizeCommand::Cancel(args) => {
            let context = load_launch(store, args.run_id).await?;
            (
                context.definition.project.id,
                context.definition.project.fingerprint,
            )
        }
    };
    anyhow::ensure!(
        project.0 == project_id && project.1 == project_fingerprint,
        "optimization belongs to another managed project's scientific binding"
    );
    Ok(())
}

/// The compiled composition root supplies a task adapter only for native stages.
/// Passive commands and launch validation consume the same persisted slice contracts.
pub(crate) async fn execute_lifecycle<B: EncoderTaskBackend>(
    command: EncoderOptimizeCommand,
    store: &SqliteExperimentStore,
    backend_factory: impl FnOnce() -> anyhow::Result<B>,
) -> anyhow::Result<()> {
    match command {
        EncoderOptimizeCommand::Preview(args) => preview(store, args).await,
        EncoderOptimizeCommand::Start(args) => start(store, args).await,
        EncoderOptimizeCommand::Status(args) => status(store, args).await,
        EncoderOptimizeCommand::Inspect(args) => inspect(store, args).await,
        EncoderOptimizeCommand::ReviewRepair(args) => review_facts(store, args, false).await,
        EncoderOptimizeCommand::ReviewDelta(args) => review_facts(store, args, true).await,
        EncoderOptimizeCommand::Resume(args) => resume(store, backend_factory, args).await,
        EncoderOptimizeCommand::AuthorizeExternal(args) => authorize_external(store, args).await,
        EncoderOptimizeCommand::AuthorizeSealed(args) => {
            let backend = backend_factory()?;
            authorize_sealed(store, &backend, args).await
        }
        EncoderOptimizeCommand::Cancel(args) => cancel(store, args).await,
        EncoderOptimizeCommand::Provenance(args) => {
            let backend = backend_factory()?;
            provenance(store, &backend, args).await
        }
        EncoderOptimizeCommand::Report(args) => report::execute(store, args).await,
        EncoderOptimizeCommand::Doctor(_) => {
            anyhow::bail!("native Doctor is handled by the adapter composition root")
        }
    }
}

async fn preview(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeManifestArgs,
) -> anyhow::Result<()> {
    let resolved = resolve_manifest(store, &args.manifest).await?;
    presentation::print(&serde_json::json!({
        "persisted": false,
        "external_calls": 0,
        "manifest_fingerprint": resolved.manifest_fingerprint,
        "specification_fingerprint": resolved.definition.specification_fingerprint,
        "project": resolved.definition.project,
        "training_snapshot": resolved.definition.training_snapshot,
        "benchmark": resolved.definition.benchmark,
        "candidate_count": resolved.definition.candidates.len(),
        "budget": resolved.definition.campaign_budget,
        "review_boundaries": {
            "repair_proposal": "already_frozen",
            "native_delta": "already_frozen",
            "sealed_use": "explicit_runtime_authorization",
        },
        "adopts_existing_experiment": resolved.manifest.existing_experiment.is_some(),
        "stages": [
            "attach_campaign",
            "prepare_protocol",
            "start_or_adopt_run",
            "train_and_evaluate_development",
            "finalize_or_await_sealed_authorization",
            "complete",
        ],
    }))
}

async fn start(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeManifestArgs,
) -> anyhow::Result<()> {
    let manifest = read_manifest(&args.manifest)?;
    validate_manifest(&manifest)?;
    let manifest_fingerprint = artifact_core::fingerprint(&manifest)?;
    if let Some((_, run)) = store
        .find_optimization_by_manifest(manifest_fingerprint)
        .await?
    {
        let context = load_launch(store, run.id).await?;
        return print_current_status(store, &context, true).await;
    }
    let resolved = resolve_manifest(store, &args.manifest).await?;
    if let Some((definition, run)) = store
        .find_optimization_by_manifest(resolved.manifest_fingerprint.clone())
        .await?
    {
        if definition.specification_fingerprint != resolved.definition.specification_fingerprint {
            anyhow::bail!("manifest identity already belongs to a different resolved optimization");
        }
        let context = load_launch(store, run.id).await?;
        return print_current_status(store, &context, true).await;
    }
    let now = Utc::now();
    let run = ProductionOptimizationRun::create_with_reservations(
        &resolved.definition,
        Uuid::new_v4(),
        resolved.adopted_protocol_id.unwrap_or_else(Uuid::new_v4),
        resolved.adopted_run_id.unwrap_or_else(Uuid::new_v4),
        now,
    )?;
    let first = first_optimization_event(&run, now)?;
    store
        .create_optimization(resolved.definition, run.clone(), first)
        .await?;
    let context = load_launch(store, run.id).await?;
    print_current_status(store, &context, false).await
}

async fn status(store: &SqliteExperimentStore, args: EncoderOptimizeRunArgs) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    print_current_status(store, &context, true).await
}

async fn inspect(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let campaign = load_optional_campaign(store, &context).await?;
    presentation::print(&serde_json::json!({
        "definition": context.definition,
        "run": context.run,
        "lifecycle": context.view,
        "campaign": campaign.as_ref().map(|value| &value.view),
        "experiment": campaign.as_ref().and_then(|value| value.experiment.as_ref()),
        "next_command": next_command(&context, campaign.as_ref()),
    }))
}

async fn review_facts(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeRunArgs,
    delta: bool,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    let value = if delta {
        serde_json::json!({
            "boundary": "native_delta_review",
            "status": "already_frozen",
            "selection": approved.selection,
            "approval": approved.approval,
            "predecessor": approved.approval_predecessor,
            "note": "Use production-repair before optimize to create a different review; this run cannot rewrite frozen authority.",
        })
    } else {
        serde_json::json!({
            "boundary": "repair_proposal_review",
            "status": "already_frozen",
            "proposal": {
                "id": approved.proposal.id,
                "fingerprint": approved.proposal.fingerprint,
                "specification_fingerprint": approved.proposal.specification_fingerprint,
            },
            "application": approved.selection.application,
            "note": "Use production-repair before optimize to create a different review; this run cannot rewrite frozen authority.",
        })
    };
    presentation::print(&value)
}

async fn resume<B: EncoderTaskBackend>(
    store: &SqliteExperimentStore,
    backend_factory: impl FnOnce() -> anyhow::Result<B>,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if context.view.state != OptimizationRunState::Planned
        && context.view.state != OptimizationRunState::CampaignActive
    {
        return print_current_status(store, &context, true).await;
    }
    if context.view.state == OptimizationRunState::Planned {
        attach_campaign(store, &context).await?;
        return status(
            store,
            EncoderOptimizeRunArgs {
                run_id: args.run_id,
                backend: args.backend,
            },
        )
        .await;
    }

    let campaign = load_optimization_campaign(store, context.run.reserved_campaign_id).await?;
    match campaign.view.state {
        CampaignState::AwaitingGeneration => {
            ensure_generation_current(store, &context.definition).await?;
            let event = bind_generation_event(
                &campaign.campaign,
                &campaign.view,
                context.definition.benchmark.clone(),
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::ReadyToPrepare => {
            ensure_generation_current(store, &context.definition).await?;
            let backend = backend_factory()?;
            let runner = ExperimentRunner::new(store, &backend);
            let protocol = runner
                .prepare_multi_protocol_identified(
                    context.run.reserved_protocol_id,
                    context.definition.project.id,
                    context.definition.metric_contract.clone(),
                    protocol_budget(&context.definition)?,
                    context.definition.maximum_evaluation_seconds,
                    context.definition.development_suite_keys.clone(),
                    context.definition.sealed_suite_key.clone(),
                    context.definition.candidates.clone(),
                )
                .await?;
            let event =
                prepare_iteration_event(&campaign.campaign, &campaign.view, &protocol, Utc::now())?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::ReadyToStart => {
            ensure_generation_current(store, &context.definition).await?;
            let backend = backend_factory()?;
            let runner = ExperimentRunner::new(store, &backend);
            let experiment = runner
                .create_run_identified(
                    context.run.reserved_protocol_id,
                    context.run.reserved_experiment_run_id,
                )
                .await?;
            let event = if experiment.state == ExperimentRunState::Ready {
                start_run_event(&campaign.campaign, &campaign.view, &experiment, Utc::now())?
            } else {
                adopt_run_event(&campaign.campaign, &campaign.view, &experiment, Utc::now())?
            };
            store.append_campaign_event(&event).await?;
        }
        CampaignState::RunningDevelopment => {
            ensure_generation_current(store, &context.definition).await?;
            let current = campaign
                .experiment
                .context("campaign experiment is missing")?;
            let experiment = if matches!(
                current.state,
                ExperimentRunState::AwaitingSealedAuthorization | ExperimentRunState::Completed
            ) {
                current
            } else {
                let backend = backend_factory()?;
                ExperimentRunner::new(store, &backend)
                    .run_development(context.run.reserved_experiment_run_id)
                    .await?
            };
            let event = record_development_event(
                &campaign.campaign,
                &campaign.view,
                &experiment,
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::AwaitingFinalization => {
            let experiment = campaign
                .experiment
                .as_ref()
                .context("campaign experiment is missing")?;
            finalize_campaign_iteration(store, &campaign, experiment).await?;
        }
        CampaignState::AwaitingSealedAuthorization => {
            return print_status(&context, true, Some(&campaign));
        }
        CampaignState::SealedAuthorized => {
            ensure_generation_current(store, &context.definition).await?;
            let experiment = match campaign.experiment.as_ref() {
                Some(value) if value.state == ExperimentRunState::Completed => value.clone(),
                _ => {
                    let backend = backend_factory()?;
                    ExperimentRunner::new(store, &backend)
                        .run_sealed(context.run.reserved_experiment_run_id)
                        .await?
                }
            };
            finalize_campaign_iteration(store, &campaign, &experiment).await?;
        }
        CampaignState::RenewalRequired => {
            let event = complete_campaign_event(
                &campaign.campaign,
                &campaign.view,
                "finite optimize run completed its one authorized iteration",
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::Completed => {
            let decision = campaign
                .experiment
                .as_ref()
                .and_then(|value| value.final_decision)
                .context("completed campaign has no deterministic experiment decision")?;
            let event = context.view.next_event(
                &context.run,
                OptimizationEventKind::Completed {
                    decision,
                    campaign_head_fingerprint: campaign.view.last_event_fingerprint,
                },
                Utc::now(),
            )?;
            store.append_optimization_event(event).await?;
        }
    }
    status(
        store,
        EncoderOptimizeRunArgs {
            run_id: args.run_id,
            backend: args.backend,
        },
    )
    .await
}

async fn authorize_sealed(
    store: &SqliteExperimentStore,
    backend: &impl EncoderTaskBackend,
    args: EncoderOptimizeAuthorizeArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if context.view.state != OptimizationRunState::CampaignActive {
        anyhow::bail!("optimization is not active");
    }
    let campaign = load_optimization_campaign(store, context.run.reserved_campaign_id).await?;
    if !matches!(
        campaign.view.state,
        CampaignState::AwaitingSealedAuthorization | CampaignState::SealedAuthorized
    ) {
        anyhow::bail!("optimization is not waiting for sealed-use authorization");
    }
    ensure_generation_current(store, &context.definition).await?;
    let runner = ExperimentRunner::new(store, backend);
    let experiment = runner
        .authorize_sealed(context.run.reserved_experiment_run_id, &args.authorized_by)
        .await?;
    if campaign.view.state == CampaignState::SealedAuthorized {
        return print_current_status(store, &context, true).await;
    }
    let event = record_sealed_authorization_event(
        &campaign.campaign,
        &campaign.view,
        &experiment,
        args.authorized_by,
        Utc::now(),
    )?;
    store.append_campaign_event(&event).await?;
    let updated = load_launch(store, args.run_id).await?;
    print_current_status(store, &updated, true).await
}

async fn authorize_external(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeAuthorizeArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    if approved.proposal.budget.maximum_external_calls == 0 {
        return presentation::print(&serde_json::json!({
            "run_id": context.run.id,
            "status": "not_required",
            "authorized": false,
            "authorized_by": args.authorized_by,
            "maximum_external_calls": 0,
            "reason": "The frozen repair proposal authorizes no network or paid provider calls.",
            "next_command": next_command(&context, None),
        }));
    }
    anyhow::bail!(
        "this optimize run has no persisted external-call reservation; create and authorize the exact request through production-repair before starting optimize"
    )
}

async fn cancel(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeCancelArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if !matches!(
        context.view.state,
        OptimizationRunState::Planned | OptimizationRunState::CampaignActive
    ) {
        return print_current_status(store, &context, true).await;
    }
    let event = context.view.next_event(
        &context.run,
        OptimizationEventKind::Cancelled {
            reason: args.reason,
        },
        Utc::now(),
    )?;
    store.append_optimization_event(event).await?;
    let updated = load_launch(store, args.run_id).await?;
    print_current_status(store, &updated, true).await
}

async fn doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_context(store, backend, snapshot.selection.id, true).await?;
    snapshot.validate_against(
        &approved.project,
        &approved.proposal,
        &approved.candidate_set,
        &approved.report,
        &approved.approval,
        approved.approval_predecessor.as_ref(),
        &approved.selection,
    )?;
    backend.inspect(approved.project.clone()).await?;
    ensure_generation_matches(store, &context.definition, false).await?;
    let campaign_fingerprint = if store
        .get_campaign(context.run.reserved_campaign_id)
        .await?
        .is_some()
    {
        Some(artifact_core::fingerprint(
            &campaign_provenance(store, backend, context.run.reserved_campaign_id).await?,
        )?)
    } else {
        None
    };
    presentation::print(&serde_json::json!({
        "verified": true,
        "run_id": context.run.id,
        "state": context.view.state,
        "optimization_head_fingerprint": context.view.last_event_fingerprint,
        "campaign_provenance_fingerprint": campaign_fingerprint,
        "native_replay_verified": true,
    }))
}

async fn provenance(
    store: &SqliteExperimentStore,
    backend: &impl EncoderTaskBackend,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let campaign = if store
        .get_campaign(context.run.reserved_campaign_id)
        .await?
        .is_some()
    {
        Some(campaign_provenance(store, backend, context.run.reserved_campaign_id).await?)
    } else {
        None
    };
    let bundle = serde_json::json!({
        "schema_version": 1,
        "definition": context.definition,
        "run": context.run,
        "lifecycle": context.view,
        "campaign": campaign,
    });
    presentation::print(&serde_json::json!({
        "bundle": bundle,
        "bundle_fingerprint": artifact_core::fingerprint(&bundle)?,
    }))
}

async fn resolve_manifest(
    store: &SqliteExperimentStore,
    path: &Path,
) -> anyhow::Result<ResolvedLaunch> {
    let manifest = read_manifest(path)?;
    validate_manifest(&manifest)?;
    let snapshot = store
        .get_native_repair_training_snapshot(manifest.training_snapshot_id)
        .await?
        .with_context(|| {
            format!(
                "native repair training snapshot {} does not exist",
                manifest.training_snapshot_id
            )
        })?;
    if snapshot.fingerprint != manifest.training_snapshot_fingerprint
        || snapshot.specification_fingerprint
            != manifest.training_snapshot_specification_fingerprint
    {
        anyhow::bail!("optimization manifest training snapshot binding changed");
    }
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    snapshot.validate_against(
        &approved.project,
        &approved.proposal,
        &approved.candidate_set,
        &approved.report,
        &approved.approval,
        approved.approval_predecessor.as_ref(),
        &approved.selection,
    )?;
    if approved.project.source_revision != manifest.project_source_revision {
        anyhow::bail!("optimization manifest project revision differs from the immutable project");
    }

    resolve_loaded(
        store,
        manifest,
        snapshot,
        &approved.project,
        &approved.proposal,
    )
    .await
}

async fn resolve_loaded(
    store: &SqliteExperimentStore,
    manifest: OptimizeManifest,
    snapshot: encoder_repair_core::training::NativeRepairTrainingSnapshot,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
    proposal: &RepairProposal,
) -> anyhow::Result<ResolvedLaunch> {
    validate_manifest(&manifest)?;
    let manifest_fingerprint = artifact_core::fingerprint(&manifest)?;
    anyhow::ensure!(
        snapshot.id == manifest.training_snapshot_id
            && snapshot.fingerprint == manifest.training_snapshot_fingerprint
            && snapshot.specification_fingerprint
                == manifest.training_snapshot_specification_fingerprint
            && project.source_revision == manifest.project_source_revision,
        "managed optimization definition no longer matches its verified repair authority"
    );
    let source_view = load_persisted_view(proposal.context.source_experiment_run_id, store).await?;
    let source_protocol = store
        .get_protocol(source_view.protocol_id)
        .await?
        .context("repair metric-source protocol does not exist")?;
    let source_project = store
        .get_project(source_protocol.project_snapshot_id)
        .await?
        .context("repair metric-source project does not exist")?;
    source_protocol.validate_integrity(&source_project)?;

    let (generation, generation_view) =
        load_generation(store, manifest.benchmark_generation_id).await?;
    if generation.fingerprint != manifest.benchmark_generation_fingerprint {
        anyhow::bail!("optimization manifest benchmark generation binding changed");
    }
    require_active_generation(&generation_view, &generation)?;
    let benchmark = CampaignBenchmarkBinding::from_active_generation(
        &generation,
        &generation_view,
        source_protocol.sealed_suite_key.clone(),
    )?;
    let repair_benchmark = &proposal.context.benchmark;
    if repair_benchmark.generation_id != generation.id
        || repair_benchmark.generation_fingerprint != generation.fingerprint
        || repair_benchmark.journal_sequence != generation_view.last_sequence
        || repair_benchmark.journal_head_fingerprint != generation_view.last_event_fingerprint
        || repair_benchmark.development_suite_fingerprints
            != benchmark.development_suite_fingerprints
        || repair_benchmark.sealed_suite_id != benchmark.sealed_suite_id
        || repair_benchmark.sealed_suite_fingerprint != benchmark.sealed_suite_fingerprint
    {
        anyhow::bail!("approved repair and optimization benchmark authorities differ");
    }

    let compiled = snapshot.compile_training_candidates(project, proposal)?;
    let deterministic = compiled
        .into_iter()
        .map(|candidate| {
            TrainingCandidate::create_identified(
                deterministic_uuid(
                    &manifest_fingerprint,
                    "candidate",
                    u64::from(candidate.sequence),
                ),
                project,
                candidate.sequence,
                candidate.maximum_training_seconds,
                candidate.parameters,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let budget = protocol_budget_from(
        proposal,
        deterministic.len(),
        source_protocol.development_suite_keys().len(),
    )?;

    let (candidates, adopted_protocol_id, adopted_run_id) = match &manifest.existing_experiment {
        Some(existing) => {
            let protocol = store
                .get_protocol(existing.protocol_id)
                .await?
                .context("manifest adoption protocol does not exist")?;
            protocol.validate_integrity(project)?;
            let view = load_persisted_view(existing.run_id, store).await?;
            if protocol.fingerprint != existing.protocol_fingerprint
                || view.protocol_id != protocol.id
                || view.last_event_fingerprint != existing.run_head_fingerprint
                || protocol.project_snapshot_id != project.id
                || protocol.metric_contract != source_protocol.metric_contract
                || protocol.budget != budget
                || protocol.maximum_evaluation_seconds != proposal.budget.maximum_evaluation_seconds
                || protocol.development_suite_keys() != source_protocol.development_suite_keys()
                || protocol.sealed_suite_key != source_protocol.sealed_suite_key
                || protocol.development_selection_rule
                    != Some(DevelopmentSelectionRule::MaximizeWorstSuiteThenMean)
                || !candidate_shapes_match(&protocol.candidates, &deterministic)
                || !protocol_matches_benchmark(&protocol, &benchmark)
            {
                anyhow::bail!("existing experiment is not the exact approved optimization request");
            }
            (
                protocol.candidates,
                Some(existing.protocol_id),
                Some(existing.run_id),
            )
        }
        None => (deterministic, None, None),
    };
    let campaign_budget = campaign_budget_from(
        proposal,
        candidates.len(),
        source_protocol.development_suite_keys().len(),
    )?;
    let definition = ProductionOptimizationDefinition::create(
        manifest.name.clone(),
        manifest_fingerprint.clone(),
        project,
        OptimizationArtifactBinding::new(
            snapshot.proposal.id,
            snapshot.proposal.fingerprint.clone(),
        )?,
        OptimizationArtifactBinding::new(
            snapshot.selection.id,
            snapshot.selection.fingerprint.clone(),
        )?,
        OptimizationArtifactBinding::new(snapshot.id, snapshot.fingerprint.clone())?,
        snapshot.specification_fingerprint.clone(),
        benchmark,
        &source_protocol,
        candidates,
        campaign_budget,
        proposal.budget.maximum_evaluation_seconds,
        Utc::now(),
    )?;
    Ok(ResolvedLaunch {
        manifest,
        manifest_fingerprint,
        definition,
        adopted_protocol_id,
        adopted_run_id,
    })
}

async fn attach_campaign(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
) -> anyhow::Result<()> {
    ensure_generation_current(store, &context.definition).await?;
    let expected = ProductionCampaign::create_identified(
        context.run.reserved_campaign_id,
        format!("{} optimization", context.definition.name),
        context.definition.project.id,
        context.definition.project.fingerprint.clone(),
        context.definition.campaign_budget.clone(),
        context.run.created_at,
    )?;
    let campaign = match store.get_campaign(expected.id).await? {
        Some(existing) => {
            if existing != expected {
                anyhow::bail!("reserved campaign identity belongs to a different optimization");
            }
            existing
        }
        None => {
            let first = first_campaign_event(&expected, expected.created_at)?;
            store.create_campaign(&expected, &first).await?;
            expected
        }
    };
    let mut campaign_context = load_optimization_campaign(store, campaign.id).await?;
    if campaign_context.view.state == CampaignState::AwaitingGeneration {
        let event = bind_generation_event(
            &campaign_context.campaign,
            &campaign_context.view,
            context.definition.benchmark.clone(),
            Utc::now(),
        )?;
        store.append_campaign_event(&event).await?;
        campaign_context = load_optimization_campaign(store, campaign.id).await?;
    }
    let event = context.view.next_event(
        &context.run,
        OptimizationEventKind::CampaignAttached {
            campaign_fingerprint: campaign_context.campaign.fingerprint,
        },
        Utc::now(),
    )?;
    store.append_optimization_event(event).await?;
    Ok(())
}

async fn load_launch(store: &SqliteExperimentStore, run_id: Uuid) -> anyhow::Result<LaunchContext> {
    let (definition, run) = store
        .get_optimization_run(run_id)
        .await?
        .with_context(|| format!("production optimization run {run_id} does not exist"))?;
    let project = store
        .get_project(definition.project.id)
        .await?
        .context("optimization project does not exist")?;
    let metric_source = store
        .get_protocol(definition.metric_source_protocol.id)
        .await?
        .context("optimization metric source protocol does not exist")?;
    definition.validate_integrity(&project, &metric_source)?;
    run.validate_integrity(&definition)?;
    let events = store.list_optimization_events(run.id).await?;
    let view = replay_optimization(&run, &events)?;
    Ok(LaunchContext {
        definition,
        run,
        view,
    })
}

/// Cheap, fully deterministic campaign replay used by routine operator commands. Native project
/// inspection belongs to side-effecting runner stages and to `doctor`, not to every status poll.
async fn load_optimization_campaign(
    store: &SqliteExperimentStore,
    campaign_id: Uuid,
) -> anyhow::Result<CampaignContext> {
    let campaign = store
        .get_campaign(campaign_id)
        .await?
        .with_context(|| format!("production campaign {campaign_id} does not exist"))?;
    let project = store
        .get_project(campaign.project_snapshot_id)
        .await?
        .context("campaign project does not exist")?;
    if project.fingerprint != campaign.project_snapshot_fingerprint {
        anyhow::bail!("campaign project fingerprint changed");
    }
    let events = store.list_campaign_events(campaign.id).await?;
    let view = replay_campaign(&campaign, &events)?;
    let generation = match &view.current_generation {
        Some(binding) => {
            let value = load_generation(store, binding.generation_id).await?;
            if value.0.fingerprint != binding.generation_fingerprint {
                anyhow::bail!("campaign benchmark-generation binding changed");
            }
            Some(value)
        }
        None => None,
    };
    let experiment = match view.run_id {
        Some(run_id) => Some(load_persisted_view(run_id, store).await?),
        None => None,
    };
    Ok(CampaignContext {
        campaign,
        view,
        generation,
        experiment,
    })
}

async fn load_optional_campaign(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
) -> anyhow::Result<Option<crate::commands::production_campaign::CampaignContext>> {
    if store
        .get_campaign(context.run.reserved_campaign_id)
        .await?
        .is_some()
    {
        let campaign = load_optimization_campaign(store, context.run.reserved_campaign_id).await?;
        if context
            .view
            .campaign_fingerprint
            .as_deref()
            .is_some_and(|value| value != campaign.campaign.fingerprint)
        {
            anyhow::bail!("optimization campaign binding changed");
        }
        Ok(Some(campaign))
    } else {
        Ok(None)
    }
}

async fn ensure_generation_current(
    store: &SqliteExperimentStore,
    definition: &ProductionOptimizationDefinition,
) -> anyhow::Result<()> {
    ensure_generation_matches(store, definition, true).await
}

async fn ensure_generation_matches(
    store: &SqliteExperimentStore,
    definition: &ProductionOptimizationDefinition,
    require_active: bool,
) -> anyhow::Result<()> {
    let (generation, view) = load_generation(store, definition.benchmark.generation_id).await?;
    if generation.fingerprint != definition.benchmark.generation_fingerprint {
        anyhow::bail!("optimization benchmark generation fingerprint changed");
    }
    if require_active {
        require_active_generation(&view, &generation)?;
    }
    Ok(())
}

fn require_active_generation(
    view: &BenchmarkGenerationView,
    generation: &workflow_core::benchmark_generation::BenchmarkGeneration,
) -> anyhow::Result<()> {
    if view.state != BenchmarkGenerationState::Active || !view.is_adaptive_eligible() {
        anyhow::bail!("optimization benchmark generation is not active and unused");
    }
    generation
        .freshness
        .as_ref()
        .context("optimization benchmark generation has no renewable freshness authority")?
        .validate_at(Utc::now())?;
    Ok(())
}

fn protocol_budget(
    definition: &ProductionOptimizationDefinition,
) -> anyhow::Result<OptimizationBudget> {
    Ok(OptimizationBudget {
        maximum_candidates: definition.campaign_budget.maximum_candidates,
        maximum_training_seconds: definition.campaign_budget.maximum_training_seconds,
        maximum_development_evaluations: definition.campaign_budget.maximum_development_evaluations,
        maximum_sealed_evaluations: definition.campaign_budget.maximum_sealed_evaluations,
    })
}

fn protocol_budget_from(
    proposal: &RepairProposal,
    candidate_count: usize,
    suite_count: usize,
) -> anyhow::Result<OptimizationBudget> {
    let candidate_count = u32::try_from(candidate_count)?;
    let suite_count = u32::try_from(suite_count)?;
    let development = candidate_count
        .checked_mul(suite_count)
        .context("development evaluation budget overflowed")?;
    if development > proposal.budget.maximum_development_evaluations
        || proposal.budget.maximum_sealed_uses != 1
    {
        anyhow::bail!("repair budget cannot cover the exact optimization protocol");
    }
    Ok(OptimizationBudget {
        maximum_candidates: candidate_count,
        maximum_training_seconds: proposal.budget.maximum_training_seconds,
        maximum_development_evaluations: development,
        maximum_sealed_evaluations: 1,
    })
}

fn campaign_budget_from(
    proposal: &RepairProposal,
    candidate_count: usize,
    suite_count: usize,
) -> anyhow::Result<CampaignBudget> {
    let candidate_count = u32::try_from(candidate_count)?;
    let suite_count = u32::try_from(suite_count)?;
    let development = candidate_count
        .checked_mul(suite_count)
        .context("development evaluation budget overflowed")?;
    let backend_operations = suite_count
        .checked_add(1)
        .and_then(|value| value.checked_add(candidate_count))
        .and_then(|value| value.checked_add(development))
        .and_then(|value| value.checked_add(1))
        .context("backend-operation budget overflowed")?;
    Ok(CampaignBudget {
        maximum_iterations: 1,
        maximum_candidates: candidate_count,
        maximum_training_seconds: proposal.budget.maximum_training_seconds,
        maximum_development_evaluations: development,
        maximum_sealed_evaluations: 1,
        maximum_backend_operations: backend_operations,
    })
}

fn candidate_shapes_match(existing: &[TrainingCandidate], expected: &[TrainingCandidate]) -> bool {
    existing.len() == expected.len()
        && existing.iter().zip(expected).all(|(left, right)| {
            left.project_snapshot_id == right.project_snapshot_id
                && left.project_snapshot_fingerprint == right.project_snapshot_fingerprint
                && left.sequence == right.sequence
                && left.maximum_training_seconds == right.maximum_training_seconds
                && left.parameters == right.parameters
        })
}

fn protocol_matches_benchmark(
    protocol: &ExperimentProtocol,
    benchmark: &CampaignBenchmarkBinding,
) -> bool {
    protocol
        .baseline_development_reports()
        .into_iter()
        .map(|report| (report.suite_key.clone(), report.suite_fingerprint.clone()))
        .collect::<std::collections::BTreeMap<_, _>>()
        == benchmark.development_suite_fingerprints
        && protocol.sealed_suite_key == benchmark.sealed_suite_key
        && protocol.baseline_sealed_report.suite_fingerprint == benchmark.sealed_suite_fingerprint
}

fn deterministic_uuid(manifest_fingerprint: &str, kind: &str, sequence: u64) -> Uuid {
    let digest = Sha256::digest(format!("{manifest_fingerprint}:{kind}:{sequence}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn read_manifest(path: &Path) -> anyhow::Result<OptimizeManifest> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("could not inspect optimization manifest {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("optimization manifest must be a regular file no larger than 1 MiB");
    }
    let bytes = fs::read(path)
        .with_context(|| format!("could not read optimization manifest {}", path.display()))?;
    let text = std::str::from_utf8(&bytes).context("optimization manifest is not UTF-8")?;
    toml::from_str(text).context("optimization manifest is not valid strict TOML")
}

fn validate_manifest(manifest: &OptimizeManifest) -> anyhow::Result<()> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.name.trim() != manifest.name
        || manifest.name.is_empty()
        || manifest.name.chars().count() > 120
        || manifest.project_source_revision.trim() != manifest.project_source_revision
        || manifest.project_source_revision.is_empty()
        || manifest.training_snapshot_id.is_nil()
        || manifest.benchmark_generation_id.is_nil()
        || !canonical_fingerprint(&manifest.training_snapshot_fingerprint)
        || !canonical_fingerprint(&manifest.training_snapshot_specification_fingerprint)
        || !canonical_fingerprint(&manifest.benchmark_generation_fingerprint)
        || manifest.selection_policy != DevelopmentSelectionRule::MaximizeWorstSuiteThenMean
    {
        anyhow::bail!("optimization manifest is incomplete or uses unsupported policy values");
    }
    if let Some(existing) = &manifest.existing_experiment {
        if existing.protocol_id.is_nil()
            || existing.run_id.is_nil()
            || !canonical_fingerprint(&existing.protocol_fingerprint)
            || !canonical_fingerprint(&existing.run_head_fingerprint)
        {
            anyhow::bail!("optimization manifest existing experiment binding is incomplete");
        }
    }
    Ok(())
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    })
}

async fn print_current_status(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
    existing: bool,
) -> anyhow::Result<()> {
    let campaign = load_optional_campaign(store, context).await?;
    print_status(context, existing, campaign.as_ref())
}

fn print_status(
    context: &LaunchContext,
    existing: bool,
    campaign: Option<&crate::commands::production_campaign::CampaignContext>,
) -> anyhow::Result<()> {
    let campaign_view = campaign.map(|value| &value.view);
    let experiment = campaign.and_then(|value| value.experiment.as_ref());
    let mut completed = Vec::new();
    if campaign.is_some() {
        completed.push("campaign_reserved_and_attached");
    }
    if campaign_view.and_then(|value| value.protocol_id).is_some() {
        completed.push("protocol_prepared");
    }
    if campaign_view.and_then(|value| value.run_id).is_some() {
        completed.push("experiment_run_started_or_adopted");
    }
    if campaign_view.is_some_and(|value| {
        matches!(
            value.state,
            CampaignState::AwaitingSealedAuthorization
                | CampaignState::SealedAuthorized
                | CampaignState::AwaitingFinalization
                | CampaignState::RenewalRequired
                | CampaignState::Completed
        )
    }) {
        completed.push("development_evaluation");
    }
    if experiment.is_some_and(|value| value.sealed_report.is_some()) {
        completed.push("sealed_evaluation");
    }
    if context.view.state == OptimizationRunState::Completed {
        completed.push("final_decision_persisted");
    }
    let requires_human_authorization = campaign_view
        .is_some_and(|value| value.state == CampaignState::AwaitingSealedAuthorization);
    let stopped_reason = match context.view.state {
        OptimizationRunState::Cancelled => "cancelled_by_operator",
        OptimizationRunState::Failed => "failed",
        OptimizationRunState::Completed => "finite_run_completed",
        OptimizationRunState::Planned => "waiting_for_operator_resume",
        OptimizationRunState::CampaignActive if requires_human_authorization => {
            "explicit_sealed_use_authorization_required"
        }
        OptimizationRunState::CampaignActive => "stage_boundary_waiting_for_operator_resume",
    };
    let reserved = campaign_view.map(|value| value.reserved_usage);
    let budget = &context.definition.campaign_budget;
    let remaining = reserved.map(|value| serde_json::json!({
        "iterations": budget.maximum_iterations.saturating_sub(value.iterations),
        "candidates": budget.maximum_candidates.saturating_sub(value.candidates),
        "training_seconds": budget.maximum_training_seconds.saturating_sub(value.training_seconds),
        "development_evaluations": budget.maximum_development_evaluations.saturating_sub(value.development_evaluations),
        "sealed_evaluations": budget.maximum_sealed_evaluations.saturating_sub(value.sealed_evaluations),
        "backend_operations": budget.maximum_backend_operations.saturating_sub(value.backend_operations),
    }));
    presentation::print(&serde_json::json!({
        "run_id": context.run.id,
        "existing": existing,
        "state": context.view.state,
        "campaign_state": campaign_view.map(|value| value.state),
        "experiment_state": experiment.map(|value| value.state),
        "decision": context.view.decision,
        "selected_candidate_id": experiment.and_then(|value| value.selected_candidate_id),
        "completed": completed,
        "failed_or_uncertain": context.view.failure_reason,
        "stopped_reason": stopped_reason,
        "human_authorization_required": requires_human_authorization,
        "last_sequence": context.view.last_sequence,
        "head_fingerprint": context.view.last_event_fingerprint,
        "artifacts": {
            "definition_id": context.definition.id,
            "training_snapshot_id": context.definition.training_snapshot.id,
            "campaign_id": context.run.reserved_campaign_id,
            "protocol_id": context.run.reserved_protocol_id,
            "experiment_run_id": context.run.reserved_experiment_run_id,
        },
        "budgets": {
            "maximum": budget,
            "reserved": reserved,
            "remaining_unreserved": remaining,
        },
        "stage": stage_status(context, campaign),
        "next_command": next_command(context, campaign),
    }))
}

fn stage_status(
    context: &LaunchContext,
    campaign: Option<&crate::commands::production_campaign::CampaignContext>,
) -> serde_json::Value {
    let candidate_count = context.definition.candidates.len();
    let development_suite_count = context.definition.development_suite_keys.len();
    let campaign_state = campaign.map(|value| value.view.state);
    let experiment = campaign.and_then(|value| value.experiment.as_ref());
    let (key, label, detail, execution) = match (context.view.state, campaign_state) {
        (OptimizationRunState::Planned, _) => (
            "attach_campaign",
            "Set up the controlled run",
            "Attach the already-reserved campaign. This writes identities only; it does not train or evaluate.",
            "quick",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::AwaitingGeneration)) => (
            "bind_evaluations",
            "Bind the approved evaluations",
            "Confirm that the frozen development and sealed suites still match this run definition.",
            "quick",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::ReadyToPrepare)) => (
            "prepare_protocol",
            "Prepare the experiment contract",
            "Resolve the baseline reports, candidate recipes, metric gates, and finite execution budget.",
            "native",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::ReadyToStart)) => (
            "create_experiment",
            "Create the experiment journal",
            "Persist the candidate execution record before any model build begins.",
            "quick",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::RunningDevelopment)) => (
            "development",
            "Train and evaluate the candidates",
            "Build each candidate from the frozen training snapshot, then run every development suite. Sealed evidence remains unavailable.",
            "native",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::AwaitingFinalization)) => (
            "record_development",
            "Record the development decision",
            "Persist the deterministic development selection and reconcile the campaign budget.",
            "quick",
        ),
        (
            OptimizationRunState::CampaignActive,
            Some(CampaignState::AwaitingSealedAuthorization),
        ) => (
            "await_sealed_authorization",
            "Review final acceptance",
            "Development is complete. Sealed evidence stays closed until you explicitly authorize its one permitted use.",
            "authorization",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::SealedAuthorized)) => (
            "sealed_evaluation",
            "Run final acceptance",
            "Evaluate the selected candidate once against sealed evidence and persist the strict gate decision.",
            "native",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::RenewalRequired)) => (
            "complete_campaign",
            "Close the finite campaign",
            "Record that the single authorized iteration has exhausted its scope.",
            "quick",
        ),
        (OptimizationRunState::CampaignActive, Some(CampaignState::Completed)) => (
            "record_final_decision",
            "Record the final run decision",
            "Link the completed campaign decision into the optimization journal.",
            "quick",
        ),
        (OptimizationRunState::Completed, _) => (
            "completed",
            "Optimization complete",
            "The finite run and its final decision are durably recorded.",
            "terminal",
        ),
        (OptimizationRunState::Cancelled, _) => (
            "cancelled",
            "Run cancelled",
            "No further stage can execute for this immutable run.",
            "terminal",
        ),
        (OptimizationRunState::Failed, _) => (
            "failed",
            "Run failed",
            "The failure is recorded. Inspect the persisted reason before starting another run.",
            "terminal",
        ),
        (OptimizationRunState::CampaignActive, None) => (
            "recover_campaign",
            "Recover the campaign record",
            "The optimization journal is active, but its reserved campaign could not be loaded.",
            "blocked",
        ),
    };

    let development = experiment.map(|value| {
        let completed_units = value
            .candidates
            .values()
            .map(|candidate| {
                let completed_build = usize::from(candidate.train_output.is_some());
                let recorded_failure = usize::from(
                    candidate.state == CandidateExecutionState::Failed
                        && (candidate.train_output.is_none()
                            || candidate.development_reports.len() < development_suite_count),
                );
                completed_build + candidate.development_reports.len() + recorded_failure
            })
            .sum::<usize>();
        let active_candidate_id = value.candidates.iter().find_map(|(id, candidate)| {
            matches!(
                candidate.state,
                CandidateExecutionState::Training | CandidateExecutionState::Trained
            )
            .then_some(*id)
        });
        serde_json::json!({
            "completed_units": completed_units,
            "total_units": candidate_count.saturating_mul(development_suite_count.saturating_add(1)),
            "active_candidate_id": active_candidate_id,
        })
    });
    serde_json::json!({
        "key": key,
        "label": label,
        "detail": detail,
        "execution": execution,
        "development": development,
    })
}

fn next_command(
    context: &LaunchContext,
    campaign: Option<&crate::commands::production_campaign::CampaignContext>,
) -> &'static str {
    match context.view.state {
        OptimizationRunState::Planned => "resume",
        OptimizationRunState::Completed
        | OptimizationRunState::Cancelled
        | OptimizationRunState::Failed => "none",
        OptimizationRunState::CampaignActive => match campaign.map(|value| value.view.state) {
            Some(CampaignState::AwaitingSealedAuthorization) => "authorize-sealed",
            _ => "resume",
        },
    }
}

pub(crate) fn backend_args(command: &EncoderOptimizeCommand) -> &NomosWorkspaceArgs {
    match command {
        EncoderOptimizeCommand::Preview(args) | EncoderOptimizeCommand::Start(args) => {
            &args.backend
        }
        EncoderOptimizeCommand::Status(args)
        | EncoderOptimizeCommand::Inspect(args)
        | EncoderOptimizeCommand::ReviewRepair(args)
        | EncoderOptimizeCommand::ReviewDelta(args)
        | EncoderOptimizeCommand::Resume(args)
        | EncoderOptimizeCommand::Doctor(args)
        | EncoderOptimizeCommand::Provenance(args)
        | EncoderOptimizeCommand::Report(args) => &args.backend,
        EncoderOptimizeCommand::AuthorizeSealed(args) => &args.backend,
        EncoderOptimizeCommand::AuthorizeExternal(args) => &args.backend,
        EncoderOptimizeCommand::Cancel(args) => &args.backend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_manifest_rejects_unknown_fields() {
        let text = r#"
schema_version = 1
name = "bounded repair"
adapter = "nomos"
project_source_revision = "abc"
training_snapshot_id = "00000000-0000-4000-8000-000000000001"
training_snapshot_fingerprint = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
training_snapshot_specification_fingerprint = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
benchmark_generation_id = "00000000-0000-4000-8000-000000000002"
benchmark_generation_fingerprint = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
approval_mode = "explicit_sealed_use"
selection_policy = "maximize_worst_suite_then_mean"
final_decision_policy = "strict_metric_contract"
surprise = true
"#;
        assert!(toml::from_str::<OptimizeManifest>(text).is_err());
    }

    #[test]
    fn reserved_candidate_ids_are_stable_and_distinct() {
        let first = deterministic_uuid("sha256:test", "candidate", 1);
        assert_eq!(first, deterministic_uuid("sha256:test", "candidate", 1));
        assert_ne!(first, deterministic_uuid("sha256:test", "candidate", 2));
        assert!(!first.is_nil());
    }

    #[test]
    fn content_addressed_definition_is_idempotent_and_never_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("definition.toml");
        write_content_addressed(&path, b"first").unwrap();
        write_content_addressed(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert!(write_content_addressed(&path, b"second").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"first");
    }

    #[test]
    fn managed_definition_directory_rejects_non_directory_ancestor() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("runs"), b"not a directory").unwrap();
        assert!(verify_managed_definitions_directory(project.path(), true).is_err());
    }

    #[test]
    fn optimization_execution_lease_blocks_a_live_duplicate_and_releases_cleanly() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("scientific.sqlite");
        let url = format!("sqlite://{}", database.to_string_lossy().replace('\\', "/"));
        let run_id = Uuid::new_v4();
        let lease = OptimizationExecutionLease::acquire(&url, run_id).unwrap();
        let duplicate = OptimizationExecutionLease::acquire(&url, run_id).unwrap_err();
        assert!(duplicate.to_string().contains("already running"));
        drop(lease);
        assert!(OptimizationExecutionLease::acquire(&url, run_id).is_ok());
    }

    #[test]
    fn optimization_execution_lease_recovers_an_atomically_published_stale_owner() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("scientific.sqlite");
        let url = format!("sqlite://{}", database.to_string_lossy().replace('\\', "/"));
        let run_id = Uuid::new_v4();
        let lease_directory = directory
            .path()
            .join(format!(".encoder-optimization-{run_id}.lease"));
        fs::create_dir(&lease_directory).unwrap();
        let mut absent_process_id = u32::MAX;
        while process_started_at(absent_process_id).is_some() {
            absent_process_id -= 1;
        }
        let stale = ExecutionLeaseOwner {
            schema_version: EXECUTION_LEASE_SCHEMA_VERSION,
            process_id: absent_process_id,
            process_started_at: 0,
            nonce: Uuid::new_v4(),
        };
        fs::write(
            lease_directory.join("owner.json"),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();

        let lease = OptimizationExecutionLease::acquire(&url, run_id).unwrap();
        assert_ne!(lease.owner, stale);
        drop(lease);
        assert!(!lease_directory.exists());
    }
}
