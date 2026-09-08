use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

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
    /// Verify and bind the compiled Nomos runtime to a contained scientific store.
    BindNomos {
        folder: PathBuf,
        #[arg(long)]
        runtime: PathBuf,
        #[arg(long, default_value = "python")]
        python: PathBuf,
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
