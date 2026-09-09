use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Subcommand)]
pub enum ManagedOptimizeCommand {
    /// Resolve the exact reviewed request without persisting a run.
    Preview {
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Idempotently reserve one run for the exact reviewed request.
    Start {
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Show concise persisted lifecycle state.
    Status { run_id: Uuid },
    /// Inspect the immutable definition and linked lifecycle state.
    Inspect { run_id: Uuid },
    /// Verify the frozen repair review used by this run.
    ReviewRepair { run_id: Uuid },
    /// Verify the frozen native-delta review used by this run.
    ReviewDelta { run_id: Uuid },
    /// Execute at most one durable stage.
    Resume { run_id: Uuid },
    /// Inspect a reserved external-call boundary.
    AuthorizeExternal {
        run_id: Uuid,
        #[arg(long, default_value = "local-operator")]
        authorized_by: String,
    },
    /// Authorize the selected candidate's one sealed evaluation.
    AuthorizeSealed {
        run_id: Uuid,
        #[arg(long, default_value = "local-operator")]
        authorized_by: String,
    },
    /// Persist cancellation before another stage starts.
    Cancel {
        run_id: Uuid,
        #[arg(long)]
        reason: String,
    },
    /// Deeply verify the complete optimization and native evidence chain.
    Doctor { run_id: Uuid },
    /// Print a row-free, machine-verifiable provenance bundle.
    Provenance { run_id: Uuid },
    /// Print the deterministic operator report.
    Report { run_id: Uuid },
}

#[derive(Debug, Subcommand)]
pub enum ManagedProviderCommand {
    /// Show non-secret settings and availability-only credential status.
    Show,
    /// Append one strict non-secret settings revision from JSON.
    Configure {
        #[arg(long)]
        file: PathBuf,
        /// Active revision observed before editing; omit only for first setup.
        #[arg(long)]
        expected_revision_id: Option<Uuid>,
        #[arg(long, default_value = "local-operator")]
        actor: String,
        #[arg(long, default_value = "Configure project providers")]
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum WorkspaceDatasetPurpose {
    Unassigned,
    Training,
    Development,
    Sealed,
}

impl From<WorkspaceDatasetPurpose> for project_workspace_core::DatasetPurpose {
    fn from(value: WorkspaceDatasetPurpose) -> Self {
        match value {
            WorkspaceDatasetPurpose::Unassigned => Self::Unassigned,
            WorkspaceDatasetPurpose::Training => Self::Training,
            WorkspaceDatasetPurpose::Development => Self::Development,
            WorkspaceDatasetPurpose::Sealed => Self::Sealed,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Inspect and fingerprint a local safetensors encoder checkpoint.
    InspectModel { source: PathBuf },
    /// Copy the previewed checkpoint into a NEW managed project folder.
    Create {
        destination: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        model: PathBuf,
        #[arg(long)]
        expected_fingerprint: String,
        #[arg(long)]
        task: Option<String>,
    },
    /// Read managed project metadata and validate project/database binding.
    Open { folder: PathBuf },
    /// Rehash every baseline and dataset artifact and validate dataset counts.
    Verify { folder: PathBuf },
    /// Upgrade the project registry and initialize missing baseline history.
    Upgrade { folder: PathBuf },
    /// Derive launch readiness from current managed and scientific facts.
    Readiness {
        folder: PathBuf,
        /// Exact reviewed optimization manifest to resolve without persisting it.
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Operate the bound finite optimizer through fixed project-scoped intents.
    Optimize {
        folder: PathBuf,
        #[command(subcommand)]
        command: Box<ManagedOptimizeCommand>,
    },
    /// Configure separate project providers without storing secret values.
    Providers {
        folder: PathBuf,
        #[command(subcommand)]
        command: ManagedProviderCommand,
    },
    /// Passively verify a Nomos runtime before offering to bind it.
    PreviewNomosBinding {
        folder: PathBuf,
        #[arg(long)]
        runtime: PathBuf,
        #[arg(long, default_value = "python")]
        python: PathBuf,
        /// Existing Encoder Gym scientific database to verify and copy into this project.
        #[arg(long)]
        history_database: Option<PathBuf>,
    },
    /// Verify and bind the compiled Nomos runtime to a contained scientific store.
    BindNomos {
        folder: PathBuf,
        #[arg(long)]
        runtime: PathBuf,
        #[arg(long, default_value = "python")]
        python: PathBuf,
        /// Existing Encoder Gym scientific database previously accepted by preview.
        #[arg(long)]
        history_database: Option<PathBuf>,
        #[arg(long, default_value = "local-operator")]
        actor: String,
        #[arg(long, default_value = "Configure verified Nomos runtime")]
        reason: String,
    },
    /// Preview local JSONL data without altering its native structure.
    InspectDataset {
        source: PathBuf,
        #[arg(long, value_enum, default_value_t = WorkspaceDatasetPurpose::Unassigned)]
        purpose: WorkspaceDatasetPurpose,
    },
    /// Copy a previewed dataset and record immutable custody metadata.
    ImportDataset {
        folder: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long, value_enum)]
        purpose: WorkspaceDatasetPurpose,
        #[arg(long)]
        expected_fingerprint: String,
    },
    /// Backfill exact final-stage Nomos training inputs, never held-out data.
    BackfillNomos {
        folder: PathBuf,
        #[arg(long)]
        source_root: PathBuf,
    },
}
