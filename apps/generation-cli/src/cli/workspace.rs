use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Subcommand)]
pub enum WorkspaceOptimizationSetupCommand {
    /// Read-only preview of the exact model, training version and benchmark.
    Preview {
        #[arg(long)]
        model: Uuid,
        #[arg(long)]
        dataset_version: Uuid,
        #[arg(long)]
        benchmark_version: Uuid,
    },
    /// Save the reviewed identity-only setup; does not start a run.
    Save {
        #[arg(long)]
        file: PathBuf,
    },
    /// Read immutable setup history; the last entry is the current selection.
    List,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceOptimizationLaunchCommand {
    /// Read-only preview of the exact one-click optimization authority.
    Preview {
        #[arg(long)]
        setup: Uuid,
        /// Preview bounded agent settings from strict JSON; does not execute.
        #[arg(long, conflicts_with = "quick_test")]
        settings_file: Option<PathBuf>,
        /// Preview a single small diagnostic iteration with no final holdout.
        #[arg(long)]
        quick_test: bool,
        /// Preview the standard bounded Agent loop using core-owned defaults.
        #[arg(long, conflicts_with_all = ["settings_file", "quick_test"])]
        agentic: bool,
    },
    /// Authorize the exact previewed inputs and finite execution envelope.
    Authorize {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "local-operator")]
        authorized_by: String,
    },
    /// Read immutable one-click authorization history.
    List,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceOptimizationRunCommand {
    /// Run/resume the authorized finite Agent loop through development; never uses final holdout.
    DriveAgent {
        run_id: Uuid,
        /// Resume only the stopped execution head returned by run show.
        #[arg(long, value_name = "EXECUTION_HEAD")]
        resume: Option<String>,
        #[command(flatten)]
        runtime: super::ResearchRuntimeArgs,
    },
    /// Durably stop the Agent run; preserve its identity and completed artifacts.
    StopAgent {
        run_id: Uuid,
        /// Reuse this UUID when retrying the same Stop command.
        #[arg(long)]
        request_id: Option<Uuid>,
    },
    /// Reconcile an exited Agent worker without starting or resuming work.
    ReconcileAgent { run_id: Uuid },
    /// Inspect durable native-training attempts and time charges without executing work.
    TrainingTime { run_id: Uuid },
    /// Read immutable iteration inputs and development report references.
    Iterations { run_id: Uuid },
    /// Read iteration custody and original development comparisons; never executes work.
    History { run_id: Uuid },
    /// Pin the first Agent iteration from already-verified inputs and saved development evidence.
    BindIteration { run_id: Uuid },
    /// Execute the pinned Agent and generator, publishing its dataset edits without training.
    EditDataset {
        run_id: Uuid,
        #[command(flatten)]
        runtime: super::ResearchRuntimeArgs,
    },
    /// Execute Agent edits, render and qualify the derived dataset without training.
    PrepareCandidate {
        run_id: Uuid,
        #[command(flatten)]
        runtime: super::ResearchRuntimeArgs,
    },
    /// Compose Agent edits, qualified data, bounded training and development.
    CompleteIteration {
        run_id: Uuid,
        #[command(flatten)]
        runtime: super::ResearchRuntimeArgs,
    },
    /// Read this run's pinned non-secret provider connections and models.
    Providers { run_id: Uuid },
    /// Authorize exact inputs and idempotently reserve their project run.
    Start {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "local-operator")]
        authorized_by: String,
    },
    /// List project-owned input-first optimization runs.
    List,
    /// Show one verified project run and its journal head.
    Show { run_id: Uuid },
    /// Permanently cancel unfinished work for this project run.
    Cancel { run_id: Uuid },
    /// Verify selected inputs and native runtime without executing work.
    Prepare { run_id: Uuid },
    /// Render the selected dataset into the task adapter's immutable format.
    Materialize { run_id: Uuid },
    /// Attach the first finite candidate experiment without executing it.
    Attach { run_id: Uuid },
    /// Train one candidate and compare it on the shared benchmark.
    Execute { run_id: Uuid },
    /// Add the trained candidate and its exact dataset to project custody.
    Register { run_id: Uuid },
    /// Run the one authorized final evaluation for the selected candidate.
    Finalize { run_id: Uuid },
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceBenchmarkCommand {
    /// List immutable versions of the project's shared evaluation benchmark.
    List,
    /// Evaluate the baseline and create the first benchmark from the bound task runtime.
    Initialize {
        #[arg(long)]
        expected_parent: Option<Uuid>,
    },
    /// Preview the benchmark pinned by a recorded experiment without writing.
    PreviewRun { run_id: Uuid },
    /// Adopt a recorded experiment's benchmark; never executes evaluation.
    AdoptRun {
        run_id: Uuid,
        #[arg(long)]
        expected_parent: Option<Uuid>,
        /// Reject a definition changed since the user previewed it.
        #[arg(long)]
        expected_definition: Option<String>,
    },
    /// Reverify a catalog version against its original scientific protocol.
    Inspect { version_id: Uuid },
    /// Compare all project models using only this version's development reports.
    Results { version_id: Uuid },
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceDatasetCommand {
    /// List the project's base dataset, variants, and version summaries.
    List,
    /// Reconstruct the imported model's exact recorded final-stage dataset.
    AdoptBaseline,
    /// Link already-registered run outputs to their verified training datasets.
    AdoptRun { run_id: uuid::Uuid },
    /// Create the base dataset from verified training imports.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "source", required = true)]
        sources: Vec<Uuid>,
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[arg(long)]
        version_id: Option<Uuid>,
    },
    /// Create a named variant from one exact existing version.
    Fork {
        parent_id: Uuid,
        #[arg(long)]
        name: String,
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[arg(long)]
        version_id: Option<Uuid>,
    },
    /// Append an immutable version using an exact-parent JSON change request.
    Revise {
        #[arg(long)]
        file: PathBuf,
    },
    /// Inspect verified membership and ancestry references without row payloads.
    Inspect { version_id: Uuid },
    /// Inspect a bounded page of native training rows.
    Rows {
        version_id: Uuid,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Inspect row-level before/after changes for one version.
    Changes {
        version_id: Uuid,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u32).range(1..=50))]
        limit: u32,
    },
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceActivityCommand {
    /// Initialize the activity schema after verifying the project binding.
    Init,
    /// List recent project actions with their complete immutable event chains.
    List {
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=10_000))]
        limit: u32,
    },
    /// Show one action and every event recorded for it.
    Show { action_id: Uuid },
    /// Show every action and retry linked to a run, without a recent-action cutoff.
    Run { run_id: Uuid },
    /// Append one validated event from a JSON request file.
    Append {
        #[arg(long)]
        file: PathBuf,
    },
    /// Export the complete verified event stream as JSON Lines.
    Export {
        #[arg(long = "destination")]
        output: PathBuf,
    },
}

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
    /// Select the baseline, starting dataset and shared evaluation version.
    OptimizationSetup {
        folder: PathBuf,
        #[command(subcommand)]
        command: WorkspaceOptimizationSetupCommand,
    },
    /// Preview and authorize one exact input-first Optimize action.
    OptimizationLaunch {
        folder: PathBuf,
        #[command(subcommand)]
        command: WorkspaceOptimizationLaunchCommand,
    },
    /// Start and inspect project-owned input-first optimization runs.
    OptimizationRun {
        folder: PathBuf,
        #[command(subcommand)]
        command: WorkspaceOptimizationRunCommand,
    },
    /// Manage native training datasets, variants, immutable versions, and diffs.
    Dataset {
        folder: PathBuf,
        #[command(subcommand)]
        command: WorkspaceDatasetCommand,
    },
    /// Manage one shared, versioned evaluation benchmark for this project.
    Benchmark {
        folder: PathBuf,
        #[command(subcommand)]
        command: WorkspaceBenchmarkCommand,
    },
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
    /// Inspect, append, or export the project-wide activity trail.
    Activity {
        folder: PathBuf,
        #[command(subcommand)]
        command: WorkspaceActivityCommand,
    },
    /// Derive launch readiness from current managed and scientific facts.
    Readiness {
        folder: PathBuf,
        /// Exact reviewed optimization manifest to resolve without persisting it.
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
    /// Freeze the one current approved repair and create its managed run definition.
    PrepareOptimization { folder: PathBuf },
    /// Bring completed optimization checkpoints into project custody, including rejected models.
    RegisterRunModels {
        folder: PathBuf,
        #[arg(long)]
        run_id: Uuid,
    },
    /// Operate the bound finite optimizer through fixed project-scoped intents.
    Optimize {
        folder: PathBuf,
        #[command(subcommand)]
        command: Box<ManagedOptimizeCommand>,
    },
    /// Promote a checkpoint already accepted by the bound finite optimizer.
    Promote {
        folder: PathBuf,
        #[arg(long)]
        run_id: Uuid,
        /// Active baseline revision observed when the operator accepted promotion.
        #[arg(long)]
        expected_baseline_revision_id: Uuid,
        #[arg(long, default_value = "local-operator")]
        actor: String,
        #[arg(long, default_value = "Promote sealed-accepted optimization candidate")]
        reason: String,
    },
    /// Reactivate a model from immutable baseline history.
    RestoreBaseline {
        folder: PathBuf,
        /// Stable identity for retrying this exact baseline revision.
        #[arg(long)]
        revision_id: Uuid,
        /// Earlier baseline revision whose model should become active again.
        #[arg(long)]
        target_revision_id: Uuid,
        /// Active revision observed before the user confirmed restoration.
        #[arg(long)]
        expected_baseline_revision_id: Uuid,
        #[arg(long, default_value = "local-operator")]
        actor: String,
        #[arg(long, default_value = "Restore previous baseline")]
        reason: String,
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
    /// Install only the compiled Nomos adapter's missing Python packages.
    PrepareNomosPython {
        folder: PathBuf,
        #[arg(long)]
        runtime: PathBuf,
        #[arg(long)]
        python: PathBuf,
        /// Explicitly authorize a networked package install into this interpreter.
        #[arg(long)]
        allow_network_install: bool,
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
