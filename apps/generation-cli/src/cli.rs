use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use uuid::Uuid;
#[path = "cli/workspace.rs"]
mod workspace;
pub use workspace::{ManagedOptimizeCommand, ManagedProviderCommand, WorkspaceCommand};

#[derive(Debug, Parser)]
#[command(
    name = "synth",
    version,
    about = "Local, explicit-coverage synthetic-data generation",
    arg_required_else_help = true
)]
pub struct Cli {
    /// SQLite URL. Defaults to SYNTH_DATABASE_URL or the local data directory.
    #[arg(long, global = true)]
    pub database_url: Option<String>,
    /// Output mode for command results. Progress and diagnostics remain on stderr.
    #[arg(
        id = "output_format",
        long = "output",
        global = true,
        value_enum,
        default_value_t = OutputFormat::Human
    )]
    pub output: OutputFormat,
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn database_url(&self) -> String {
        self.database_url
            .clone()
            .or_else(|| std::env::var("SYNTH_DATABASE_URL").ok())
            .unwrap_or_else(|| "sqlite://data/synthetic-data.db?mode=rwc".into())
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create and open Gym-owned local-model workspaces; never starts training.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// Explicitly initialize or upgrade one database schema.
    Database {
        #[command(subcommand)]
        command: DatabaseCommand,
    },
    /// Check local database, configuration, artifacts, and optional backend connectivity.
    Doctor(DoctorArgs),
    /// Validate, resolve, and initialize declarative project configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Preview and atomically prepare a complete finite encoder project.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Create and inspect dataset definitions.
    Dataset {
        #[command(subcommand)]
        command: DatasetCommand,
    },
    /// Define reusable semantics and explicitly bind them to dataset concepts.
    Semantic {
        #[command(subcommand)]
        command: SemanticCommand,
    },
    /// Run bounded, evidence-backed authenticity research for a dataset.
    Research {
        #[command(subcommand)]
        command: ResearchCommand,
    },
    /// Design explicit generation allocations and strategies with a bounded Pi agent.
    Architect {
        #[command(subcommand)]
        command: ArchitectCommand,
    },
    /// Design renewable evaluation evidence with a bounded, row-free Pi agent.
    BenchmarkArchitect {
        #[command(subcommand)]
        command: BenchmarkArchitectCommand,
    },
    /// Create and inspect immutable dataset snapshots.
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    /// Audit semantic row quality and curate explicit training membership.
    Quality {
        #[command(subcommand)]
        command: QualityCommand,
    },
    /// Run bounded generation quality supervision and governed prompt repair.
    Supervisor {
        #[command(subcommand)]
        command: SupervisorCommand,
    },
    /// Register and verify local pretrained encoder bundles.
    Encoder {
        #[command(subcommand)]
        command: EncoderCommand,
    },
    /// Train and inspect text-classification encoder runs.
    Training {
        #[command(subcommand)]
        command: TrainingCommand,
    },
    /// Evaluate checkpoints against an immutable snapshot split.
    Evaluation {
        #[command(subcommand)]
        command: EvaluationCommand,
    },
    /// Run bounded production encoder experiments through compiled task adapters.
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
    },
    /// Govern renewable, one-use benchmark generations for production experiments.
    BenchmarkGeneration {
        #[command(subcommand)]
        command: Box<BenchmarkGenerationCommand>,
    },
    /// Operate a finite, renewable production optimization campaign.
    ProductionCampaign {
        #[command(subcommand)]
        command: Box<ProductionCampaignCommand>,
    },
    /// Diagnose and inspect development-only production encoder repair evidence.
    ProductionRepair {
        #[command(subcommand)]
        command: Box<ProductionRepairCommand>,
    },
    /// Aggregate and inspect persisted classification errors.
    Analysis {
        #[command(subcommand)]
        command: AnalysisCommand,
    },
    /// Inspect persisted bounded LLM advisory assessments.
    Advisor {
        #[command(subcommand)]
        command: AdvisorCommand,
    },
    /// Build and explicitly apply bounded data recommendations.
    Optimize {
        #[command(subcommand)]
        command: OptimizeCommand,
    },
    /// Record and assess a human-governed optimization experiment lineage.
    Campaign {
        #[command(subcommand)]
        command: CampaignCommand,
    },
    /// Preview and persist generation plans.
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    /// Preview and persist exact initial dataset-budget allocations.
    Allocation {
        #[command(subcommand)]
        command: AllocationCommand,
    },
    /// Define immutable evaluation cohorts and append-only role decisions.
    Cohort {
        #[command(subcommand)]
        command: CohortCommand,
    },
    /// Record and inspect scientific evidence disclosures.
    Exposure {
        #[command(subcommand)]
        command: ExposureCommand,
    },
    /// Detect and govern leakage across evaluation cohorts.
    Contamination {
        #[command(subcommand)]
        command: ContaminationCommand,
    },
    /// Define immutable benchmark suites and deterministic acceptance contracts.
    Benchmark {
        #[command(subcommand)]
        command: BenchmarkCommand,
    },
    /// Define and operate finite local encoder-development workflows.
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    /// Configure non-secret generation backend settings.
    Backend {
        #[command(subcommand)]
        command: BackendCommand,
    },
    /// Inspect and recover work interrupted by a terminated local process.
    Recovery {
        #[command(subcommand)]
        command: RecoveryCommand,
    },
    /// Trace an artifact through its complete persisted dependency chain.
    Provenance {
        #[arg(value_enum)]
        kind: ArtifactKindArg,
        id: Uuid,
    },
    /// Run a persisted plan in the foreground.
    Generate(GenerateArgs),
    /// Inspect or cancel a generation job.
    Job {
        #[command(subcommand)]
        command: JobCommand,
    },
    /// Show persisted per-cell coverage.
    Coverage { plan_id: Uuid },
    /// List persisted generated rows.
    Rows(RowsArgs),
    /// Export accepted rows.
    Export(ExportArgs),
}

#[derive(Debug, Subcommand)]
pub enum DatabaseCommand {
    /// Create a missing database or apply pending migrations. Does not run recovery.
    Migrate {
        #[arg(long, value_enum, default_value_t = DatabaseKind::Classification)]
        kind: DatabaseKind,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DatabaseKind {
    Classification,
    Production,
}

#[derive(Debug, Subcommand)]
pub enum ExperimentCommand {
    /// Verify the isolated Nomos copy and every immutable pilot artifact.
    NomosVerify(NomosWorkspaceArgs),
    /// Register or reuse the exact verified Nomos project snapshot.
    NomosRegister(NomosWorkspaceArgs),
    /// Freeze baseline reports, gates, budgets, and a finite candidate set.
    Prepare(ExperimentPrepareArgs),
    /// Create a new append-only run journal for a prepared protocol.
    Start(ExperimentProtocolArgs),
    /// Train and development-evaluate every finite candidate.
    RunDevelopment(ExperimentRunArgs),
    /// Show a deeply verified run view without advancing it.
    Status(ExperimentRunArgs),
    /// Explicitly authorize the development-selected candidate's sealed report.
    AuthorizeSealed(ExperimentAuthorizeArgs),
    /// Execute the one authorized candidate sealed report and finalize acceptance.
    RunSealed(ExperimentRunArgs),
}

#[derive(Debug, Subcommand)]
pub enum BenchmarkGenerationCommand {
    /// Build a strict row-free Nomos authority envelope from a qualification audit.
    NomosBuildAuthority(BenchmarkGenerationNomosAuthorityArgs),
    /// Import a completed pre-journal sealed run as an exhausted historical anchor.
    MigrateConsumed(BenchmarkGenerationHistoricalArgs),
    /// Compile the current verified Nomos authority evidence into a draft generation.
    NomosCreate(BenchmarkGenerationNomosCreateArgs),
    /// Import one integrity-checked generation authority artifact in draft state.
    Import(BenchmarkGenerationFileArgs),
    /// Show the immutable authority and deeply replayed lifecycle state.
    Show(BenchmarkGenerationIdArgs),
    /// Confirm a fresh draft generation is ready for activation.
    MarkReady(BenchmarkGenerationActorArgs),
    /// Activate the first generation, which must not declare a predecessor.
    ActivateInitial(BenchmarkGenerationActorArgs),
    /// Atomically supersede an exhausted predecessor and activate its ready successor.
    ActivateSuccessor(BenchmarkGenerationSuccessorArgs),
    /// Retire a ready or active generation without consuming sealed evidence.
    Exhaust(BenchmarkGenerationExhaustArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationHistoricalArgs {
    pub experiment_run_id: Uuid,
    #[arg(long, default_value = "local-operator")]
    pub recorded_by: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationNomosAuthorityArgs {
    /// Qualification audit path, relative to the isolated workspace or absolute within it.
    pub qualification_report: PathBuf,
    /// New authority JSON path, relative to the isolated workspace.
    pub output: PathBuf,
    #[arg(long, default_value = "nomos-successor-acquisition")]
    pub acquired_by: String,
    #[arg(long, default_value = "local-operator")]
    pub reviewed_by: String,
    #[arg(
        long,
        default_value = "Frozen successor cohort independently reviewed for one bounded production campaign"
    )]
    pub rationale: String,
    #[arg(long, default_value_t = 90)]
    pub valid_days: i64,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationNomosCreateArgs {
    #[arg(long)]
    pub predecessor_generation_id: Option<Uuid>,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationFileArgs {
    /// Strict JSON BenchmarkGeneration artifact.
    pub file: PathBuf,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationIdArgs {
    pub generation_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationActorArgs {
    pub generation_id: Uuid,
    #[arg(long, default_value = "local-operator")]
    pub actor: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationSuccessorArgs {
    pub predecessor_generation_id: Uuid,
    pub successor_generation_id: Uuid,
    #[arg(long, default_value = "local-operator")]
    pub actor: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct BenchmarkGenerationExhaustArgs {
    pub generation_id: Uuid,
    #[arg(long)]
    pub reason: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Subcommand)]
pub enum ProductionCampaignCommand {
    /// Create an append-only campaign from a strict finite-budget input file.
    Create(ProductionCampaignCreateArgs),
    /// Show the deeply replayed campaign and linked experiment/generation facts.
    Show(ProductionCampaignIdArgs),
    /// Deeply verify the complete project, generation, experiment, and exposure chain.
    Doctor(ProductionCampaignIdArgs),
    /// Print a row-free fingerprinted provenance chain for audit and handoff.
    Provenance(ProductionCampaignIdArgs),
    /// Explain the exact next action or blocking approval boundary.
    Readiness(ProductionCampaignIdArgs),
    /// Bind one currently active, unused benchmark generation.
    BindGeneration(ProductionCampaignBindArgs),
    /// Prepare the next finite experiment protocol from strict JSON.
    Prepare(ProductionCampaignPrepareArgs),
    /// Start the exact prepared protocol as a recoverable run.
    Start(ProductionCampaignIdArgs),
    /// Resume deterministic authorized work and stop at the next authority boundary.
    Advance(ProductionCampaignIdArgs),
    /// Explicitly authorize the sole selected candidate to consume sealed evidence.
    AuthorizeSealed(ProductionCampaignAuthorizeArgs),
    /// Link the immutable successor-acquisition handoff required for renewal.
    LinkRenewalHandoff(ProductionCampaignHandoffArgs),
    /// End a campaign that has reached a renewal boundary.
    Complete(ProductionCampaignCompleteArgs),
}

#[derive(Debug, Subcommand)]
pub enum ProductionRepairCommand {
    /// Collect complete development observations and persist a comparative diagnosis.
    Diagnose(ProductionRepairDiagnoseArgs),
    /// Show one deeply verified immutable comparative diagnosis.
    Show(ProductionRepairIdArgs),
    /// Deeply verify the diagnosis, all source reports, and every observation binding.
    Doctor(ProductionRepairIdArgs),
    /// List row-free repair evidence summaries for one campaign.
    Evidence(ProductionRepairCampaignArgs),
    /// Compile and persist one finite repair proposal from strict JSON or TOML.
    Propose(ProductionRepairProposeArgs),
    /// Show one deeply verified immutable repair proposal and review chain.
    ProposalShow(ProductionRepairProposalIdArgs),
    /// Verify proposal evidence, current project revision, and benchmark authority.
    ProposalDoctor(ProductionRepairProposalIdArgs),
    /// Append an immutable operator review to a current repair proposal.
    Review(ProductionRepairReviewArgs),
    /// Idempotently reserve application of an exactly approved repair proposal.
    Apply(ProductionRepairProposalIdArgs),
    /// Materialize, audit, persist, and quality-score the approved native delta.
    DeltaBuild(ProductionRepairProposalIdArgs),
    /// Show one native candidate set, quality report, reviews, and selection.
    DeltaShow(ProductionRepairDeltaIdArgs),
    /// Re-run native artifact verification and deeply replay persisted quality evidence.
    DeltaDoctor(ProductionRepairDeltaIdArgs),
    /// Append an immutable operator review to one native delta report.
    DeltaReview(ProductionRepairDeltaReviewArgs),
    /// Freeze the latest exact approved delta as the proposal's immutable selection.
    DeltaSelect(ProductionRepairDeltaReportIdArgs),
    /// Build one immutable logical base-plus-approved-delta training snapshot.
    TrainingSnapshotBuild(ProductionRepairSelectionIdArgs),
    /// Show one deeply verified combined repair training snapshot.
    TrainingSnapshotShow(ProductionRepairTrainingSnapshotIdArgs),
    /// Verify the snapshot, current project, and reproduced native delta artifact.
    TrainingSnapshotDoctor(ProductionRepairTrainingSnapshotIdArgs),
    /// Prepare one bounded multi-suite experiment from the exact repair snapshot.
    TrainingExperimentPrepare(ProductionRepairTrainingSnapshotIdArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairDiagnoseArgs {
    pub campaign_id: Uuid,
    /// Historical campaign run to diagnose. Defaults to the latest linked run.
    #[arg(long)]
    pub run_id: Option<Uuid>,
    /// Native categorical slice to include. Repeat for multiple dimensions.
    #[arg(long = "dimension", required = true)]
    pub dimensions: Vec<String>,
    /// Minimum eligible support required before a slice weakness is actionable.
    #[arg(long, default_value_t = 12)]
    pub minimum_support: u64,
    /// Finite time limit for each model/suite observation collection.
    #[arg(long, default_value_t = 900)]
    pub maximum_seconds: u64,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairIdArgs {
    pub diagnosis_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairCampaignArgs {
    pub campaign_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairProposeArgs {
    pub diagnosis_id: Uuid,
    pub benchmark_generation_id: Uuid,
    /// Strict JSON or TOML containing targets, actions, policies, budgets, and hypotheses.
    pub file: PathBuf,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairProposalIdArgs {
    pub proposal_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairDeltaIdArgs {
    pub candidate_set_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairDeltaReportIdArgs {
    pub report_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairSelectionIdArgs {
    pub selection_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairTrainingSnapshotIdArgs {
    pub training_snapshot_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProductionRepairDeltaReviewDecisionArg {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairDeltaReviewArgs {
    pub report_id: Uuid,
    #[arg(long, value_enum)]
    pub decision: ProductionRepairDeltaReviewDecisionArg,
    #[arg(long)]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProductionRepairReviewDecisionArg {
    Approve,
    Reject,
    RequestRevision,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionRepairReviewArgs {
    pub proposal_id: Uuid,
    #[arg(long, value_enum)]
    pub decision: ProductionRepairReviewDecisionArg,
    #[arg(long)]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignCreateArgs {
    pub project_id: Uuid,
    /// Strict JSON containing the campaign name and finite aggregate budget.
    pub file: PathBuf,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignIdArgs {
    pub campaign_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignBindArgs {
    pub campaign_id: Uuid,
    pub generation_id: Uuid,
    #[arg(long)]
    pub sealed_suite_key: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignPrepareArgs {
    pub campaign_id: Uuid,
    /// The ordinary strict experiment-protocol input; no campaign-specific copy exists.
    pub file: PathBuf,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignAuthorizeArgs {
    pub campaign_id: Uuid,
    #[arg(long, default_value = "local-operator")]
    pub authorized_by: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignHandoffArgs {
    pub campaign_id: Uuid,
    pub handoff_id: Uuid,
    pub handoff_fingerprint: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ProductionCampaignCompleteArgs {
    pub campaign_id: Uuid,
    #[arg(long)]
    pub reason: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct NomosWorkspaceArgs {
    /// Isolated Nomos experiment copy. The source repository is never accepted here.
    #[arg(long)]
    pub workspace: PathBuf,
    /// Python executable containing the local Nomos runtime dependencies.
    #[arg(long, default_value = "python")]
    pub python: PathBuf,
}

#[derive(Debug, clap::Args)]
pub struct ExperimentPrepareArgs {
    pub project_id: Uuid,
    /// Strict JSON protocol input containing metrics, gates, budgets, and candidates.
    pub file: PathBuf,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, clap::Args)]
pub struct ExperimentProtocolArgs {
    pub protocol_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, clap::Args)]
pub struct ExperimentRunArgs {
    pub run_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, clap::Args)]
pub struct ExperimentAuthorizeArgs {
    pub run_id: Uuid,
    #[arg(long, default_value = "local-operator")]
    pub authorized_by: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Subcommand)]
pub enum ArchitectCommand {
    /// Resolve and validate a dataset-architecture brief without persisting it.
    BriefValidate { file: PathBuf },
    /// Persist and execute one bounded Dataset Architect run in the foreground.
    Start(ArchitectStartArgs),
    /// Show the durable run, brief, proposal, and tool ledger.
    Status { run_id: Uuid },
    /// Poll durable status until the run stops.
    Watch { run_id: Uuid },
    /// Show the immutable proposal produced by a run.
    Proposal { run_id: Uuid },
    /// Request cancellation before more model or tool work.
    Cancel { run_id: Uuid },
    /// Mark an interrupted run failed without replaying model/tool calls.
    Recover { run_id: Uuid },
    /// Append a human review decision for an immutable proposal.
    Review(ArchitectReviewArgs),
    /// Apply the latest exact approval, failing if dataset coverage changed.
    Apply { proposal_id: Uuid },
    /// Inspect the approved per-cell strategy context for a generation plan.
    Context { plan_id: Uuid },
}

#[derive(Debug, clap::Args)]
pub struct ArchitectStartArgs {
    pub file: PathBuf,
    /// Scripted Pi turns for a deterministic offline fake-provider run.
    #[arg(long)]
    pub script: Option<PathBuf>,
    #[command(flatten)]
    pub runtime: ResearchRuntimeArgs,
}

#[derive(Debug, clap::Args)]
#[command(group(
    ArgGroup::new("decision")
        .required(true)
        .multiple(false)
        .args(["approve", "reject", "request_revision"])
))]
pub struct ArchitectReviewArgs {
    pub proposal_id: Uuid,
    #[arg(long)]
    pub approve: bool,
    #[arg(long)]
    pub reject: bool,
    #[arg(long)]
    pub request_revision: bool,
    #[arg(long, default_value = "local-operator")]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Subcommand)]
pub enum BenchmarkArchitectCommand {
    /// Resolve and validate a row-free benchmark-design brief without persisting it.
    BriefValidate { file: PathBuf },
    /// Persist and execute one bounded Benchmark Architect run in the foreground.
    Start(BenchmarkArchitectStartArgs),
    /// Show the durable run, budget use, proposal, review, and handoff state.
    Status { run_id: Uuid },
    /// Poll durable status until the run stops.
    Watch { run_id: Uuid },
    /// List immutable evidence captured by a run.
    Evidence { run_id: Uuid },
    /// Show the immutable proposal produced by a run.
    Proposal { run_id: Uuid },
    /// Request cancellation before more model or tool work.
    Cancel { run_id: Uuid },
    /// Mark an interrupted run failed without replaying model or tool calls.
    Recover { run_id: Uuid },
    /// Append a human decision to an immutable proposal.
    Review(BenchmarkArchitectReviewArgs),
    /// Compile the latest exact approval into a non-authorizing acquisition handoff.
    Handoff { proposal_id: Uuid },
    /// Show one immutable acquisition handoff.
    HandoffShow { id: Uuid },
    /// Compare row-free acquired-cohort facts against an approved handoff.
    Conformance {
        handoff_id: Uuid,
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Debug, clap::Args)]
pub struct BenchmarkArchitectStartArgs {
    pub file: PathBuf,
    /// Scripted Pi turns for a deterministic offline fake-provider run.
    #[arg(long)]
    pub script: Option<PathBuf>,
    /// Deterministic source corpus for a fake-provider research run.
    #[arg(long)]
    pub corpus: Option<PathBuf>,
    #[command(flatten)]
    pub runtime: ResearchRuntimeArgs,
    /// Environment variable containing the Brave Search API key for a real run.
    #[arg(long, default_value = "BRAVE_SEARCH_API_KEY")]
    pub search_api_key_env: String,
}

#[derive(Debug, clap::Args)]
#[command(group(
    ArgGroup::new("decision")
        .required(true)
        .multiple(false)
        .args(["approve", "reject", "request_revision"])
))]
pub struct BenchmarkArchitectReviewArgs {
    pub proposal_id: Uuid,
    #[arg(long)]
    pub approve: bool,
    #[arg(long)]
    pub reject: bool,
    #[arg(long)]
    pub request_revision: bool,
    #[arg(long, default_value = "local-operator")]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Subcommand)]
pub enum ResearchCommand {
    /// Resolve and validate a research brief without persisting or calling providers.
    BriefValidate { file: PathBuf },
    /// Persist and execute one bounded research run in the foreground.
    Start(ResearchStartArgs),
    /// Show durable lifecycle, plan, progress, budgets, and stop reason.
    Status { run_id: Uuid },
    /// Poll durable status until the run reaches a terminal/review state.
    Watch { run_id: Uuid },
    /// List the bounded evidence captured by a run.
    Evidence { run_id: Uuid },
    /// Show the latest profile drafted by a run.
    Profile { run_id: Uuid },
    /// Request cancellation; a running host observes this before more tool work.
    Cancel { run_id: Uuid },
    /// Mark an interrupted run failed without replaying open external calls.
    Recover { run_id: Uuid },
    /// Append a human review decision for an immutable profile.
    Review(ResearchReviewArgs),
    /// Bind an explicitly approved profile to a dataset or one of its plans.
    Bind {
        dataset_or_plan_id: Uuid,
        profile_id: Uuid,
    },
    /// Resolve the currently approved authenticity context for a dataset or plan.
    Context { dataset_or_plan_id: Uuid },
}

#[derive(Debug, Subcommand)]
pub enum SupervisorCommand {
    /// Resolve a complete quality contract without writing state.
    ContractPreview { file: PathBuf },
    /// Resolve and persist one immutable generation quality contract.
    ContractCreate { file: PathBuf },
    /// Show one immutable generation quality contract.
    ContractShow { id: Uuid },
    /// Queue a finite supervised generation run without provider I/O.
    Start(SupervisorStartArgs),
    /// Execute finite segments until pause, review, canary, or terminal state.
    Run(SupervisorExecutionArgs),
    /// Show exact durable run state, usage, active prompt, and latest decisions.
    Status { id: Uuid },
    /// Poll persisted state. This command never starts provider work.
    Watch {
        id: Uuid,
        #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(50..=60_000))]
        poll_ms: u64,
    },
    /// Persist cancellation before forwarding it to exact linked children.
    Cancel { id: Uuid },
    /// Fail closed after interruption without replaying uncertain calls.
    Recover { id: Uuid },
    /// Inspect normalized quality windows and deterministic issue decisions.
    Issues { id: Uuid },
    /// Run bounded Pi diagnosis for a deterministically paused scope.
    Diagnose(SupervisorDiagnoseArgs),
    /// Show a diagnosis session, proposal, and latest review.
    RevisionShow { session_id: Uuid },
    /// Append an explicit revision review and create a canary candidate on approval.
    RevisionReview(SupervisorRevisionReviewArgs),
    /// Authorize an exact revision through the contract's finite pre-authorization envelope.
    RevisionAuthorize { id: Uuid, session_id: Uuid },
    /// Generate and independently assess the exact authorized revision canary.
    Canary(SupervisorExecutionArgs),
    /// Show exact target and observed coverage per approved generation strategy.
    StrategyCoverage { id: Uuid },
    /// Freeze directly qualified rows and create an ordinary curation proposal.
    Finalize { id: Uuid },
    /// Show one immutable supervisor qualification handoff.
    QualificationShow { id: Uuid },
    /// Trace one generated row through prompt, strategy, generation, and quality facts.
    TraceRow { row_id: Uuid },
    /// Deeply verify all persisted generation-supervisor provenance.
    Integrity,
}

#[derive(Debug, clap::Args)]
pub struct SupervisorStartArgs {
    pub contract_id: Uuid,
    #[arg(long = "guidance", required = true)]
    pub guidance: Vec<String>,
    #[arg(long, default_value_t = 42)]
    pub strategy_seed: u64,
}

#[derive(Debug, Clone, clap::Args)]
pub struct SupervisorExecutionArgs {
    pub id: Uuid,
    /// Environment variable containing the generation API key.
    #[arg(long, default_value = "SYNTH_OPENAI_API_KEY")]
    pub api_key_env: String,
    /// Optional distinct environment variable containing the evaluator API key.
    #[arg(long)]
    pub evaluator_api_key_env: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct SupervisorDiagnoseArgs {
    pub id: Uuid,
    /// Scripted Pi turns for a deterministic offline diagnosis.
    #[arg(long)]
    pub script: Option<PathBuf>,
    #[arg(long, default_value = "fake")]
    pub provider: String,
    #[arg(long, default_value = "scripted")]
    pub model: String,
    /// Environment-variable name passed to Pi; the secret is never persisted.
    #[arg(long)]
    pub api_key_env: Option<String>,
    #[command(flatten)]
    pub runtime: ResearchRuntimeArgs,
}

#[derive(Debug, clap::Args)]
#[command(group(
    ArgGroup::new("decision")
        .required(true)
        .multiple(false)
        .args(["approve", "reject", "request_revision"])
))]
pub struct SupervisorRevisionReviewArgs {
    pub id: Uuid,
    pub session_id: Uuid,
    #[arg(long)]
    pub approve: bool,
    #[arg(long)]
    pub reject: bool,
    #[arg(long)]
    pub request_revision: bool,
    #[arg(long, default_value = "local-operator")]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, clap::Args)]
pub struct ResearchStartArgs {
    pub file: PathBuf,
    /// Scripted Pi turns for an offline fake-provider brief.
    #[arg(long)]
    pub script: Option<PathBuf>,
    /// Deterministic source corpus for an offline fake-provider brief.
    #[arg(long)]
    pub corpus: Option<PathBuf>,
    #[command(flatten)]
    pub runtime: ResearchRuntimeArgs,
    /// Environment variable containing the Brave Search API key for a real run.
    #[arg(long, default_value = "BRAVE_SEARCH_API_KEY")]
    pub search_api_key_env: String,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ResearchRuntimeArgs {
    /// Node.js executable used for the local Pi sidecar.
    #[arg(long, default_value = "node")]
    pub node: PathBuf,
    /// Compiled Pi JSONL sidecar. Defaults to the repository adapter build.
    #[arg(long)]
    pub pi_sidecar: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
#[command(group(
    ArgGroup::new("decision")
        .required(true)
        .multiple(false)
        .args(["approve", "reject", "request_revision"])
))]
pub struct ResearchReviewArgs {
    pub profile_id: Uuid,
    #[arg(long)]
    pub approve: bool,
    #[arg(long)]
    pub reject: bool,
    #[arg(long)]
    pub request_revision: bool,
    #[arg(long, default_value = "local-operator")]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Subcommand)]
pub enum AdvisorCommand {
    Show {
        id: Uuid,
    },
    List {
        #[arg(long)]
        workflow_run_id: Option<Uuid>,
        #[arg(long)]
        analysis_report_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// Validate this project configuration and its artifact directory.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Make a non-generating request to the configured OpenAI-compatible endpoint.
    #[arg(long)]
    pub check_backend: bool,
}

#[derive(Debug, Clone, Copy, clap::Args)]
pub struct PageArgs {
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Include page metadata around the result items.
    #[arg(long)]
    pub summary: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate a project TOML file and all referenced domain settings.
    Validate { file: PathBuf },
    /// Print the fully resolved configuration after defaults and overrides.
    Show(ConfigResolveArgs),
    /// Atomically create the configured dataset, initial plan, and backend settings.
    Init(ConfigResolveArgs),
    /// Compile row recipes and preview provider work without making an external call.
    ConstructionPreview(ConstructionPreviewArgs),
}

#[derive(Debug, clap::Args)]
pub struct ConstructionPreviewArgs {
    pub file: PathBuf,
    #[arg(long)]
    pub label: String,
    #[arg(long = "dimension", value_name = "NAME=VALUE")]
    pub dimensions: Vec<String>,
    #[arg(long, default_value_t = 0)]
    pub start_index: u64,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=1_000))]
    pub count: u32,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Preview local benchmark imports and complete project preparation without writing state.
    BootstrapPreview { manifest: PathBuf },
    /// Atomically import local benchmarks and prepare a startable project.
    Bootstrap { manifest: PathBuf },
    /// Show one immutable local-file bootstrap record.
    BootstrapShow { id: Uuid },
    /// List local-file bootstrap records.
    BootstrapList {
        #[command(flatten)]
        page: PageArgs,
    },
    /// Validate and preview a manifest without creating project artifacts.
    Preview { manifest: PathBuf },
    /// Atomically create all artifacts required to start the declared workflow.
    Prepare { manifest: PathBuf },
    /// Show one prepared project and the workflow definition it created.
    Show { id: Uuid },
    /// List prepared projects.
    List {
        #[command(flatten)]
        page: PageArgs,
    },
}

#[derive(Debug, clap::Args)]
pub struct ConfigResolveArgs {
    pub file: PathBuf,
    #[arg(long)]
    pub dataset_name: Option<String>,
    #[arg(long)]
    pub target_per_cell: Option<u32>,
    #[arg(long)]
    pub snapshot_seed: Option<u64>,
    #[arg(long)]
    pub training_epochs: Option<u32>,
    #[arg(long)]
    pub training_learning_rate: Option<f32>,
    #[arg(long, value_enum)]
    pub evaluation_split: Option<SnapshotSplitArg>,
}

#[derive(Debug, Subcommand)]
pub enum DatasetCommand {
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        task: String,
        #[arg(long = "label", required = true)]
        labels: Vec<String>,
        /// Categorical dimension formatted as name=value1,value2.
        #[arg(long = "dimension")]
        dimensions: Vec<String>,
    },
    List {
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        page: PageArgs,
    },
    Show {
        id: Uuid,
    },
    /// Import external text-classification rows from JSONL or CSV.
    Import(DatasetImportArgs),
    /// List persisted dataset imports.
    Imports {
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long)]
        summary: bool,
    },
    /// Inspect one persisted import.
    ImportShow {
        id: Uuid,
    },
    /// Inspect accepted and rejected rows from one import.
    ImportRows {
        id: Uuid,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long)]
        summary: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum SemanticCommand {
    /// Create version 1 of an immutable semantic profile from JSON or TOML.
    ProfileCreate { file: PathBuf },
    /// Create the next immutable version while retaining profile identity.
    ProfileRevise { id: Uuid, file: PathBuf },
    /// List semantic profiles, newest version first within each key.
    ProfileList {
        #[arg(long)]
        key: Option<String>,
    },
    /// Show one exact semantic profile version.
    ProfileShow { id: Uuid },
    /// Explicitly attach a profile to its matching dataset concept and layer.
    Bind { dataset_id: Uuid, profile_id: Uuid },
    /// Append a decision that removes one active semantic layer.
    Unbind {
        dataset_id: Uuid,
        #[arg(value_enum)]
        target: SemanticTargetArg,
        /// Required when target is dimension.
        #[arg(long)]
        dimension: Option<String>,
        #[arg(value_enum)]
        layer: SemanticLayerArg,
    },
    /// Show active reusable and dataset-override binding decisions.
    Bindings { dataset_id: Uuid },
    /// Resolve the effective semantics a new generation job would pin.
    Resolve { dataset_id: Uuid },
    /// Suggest compatible reusable profiles without attaching anything.
    Suggest { dataset_id: Uuid },
    /// Show the exact resolved semantics pinned to a generation job.
    JobContext { job_id: Uuid },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SemanticTargetArg {
    Labels,
    Dimension,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SemanticLayerArg {
    Reusable,
    DatasetOverride,
}

#[derive(Debug, clap::Args)]
pub struct DatasetImportArgs {
    pub dataset_id: Uuid,
    #[arg(long)]
    pub input: PathBuf,
    /// Input format. Inferred from .jsonl or .csv when omitted.
    #[arg(long, value_enum)]
    pub format: Option<DatasetImportFormatArg>,
    #[arg(long, default_value = "text")]
    pub text_field: String,
    #[arg(long, default_value = "label")]
    pub label_field: String,
    /// Dataset dimension to source-field mapping: dimension=field.path.
    #[arg(long = "dimension")]
    pub dimensions: Vec<String>,
    /// Validate the entire source without writing rows or import metadata.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, default_value_t = 500)]
    pub batch_size: usize,
    #[arg(long, default_value_t = 20)]
    pub max_rejection_samples: usize,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DatasetImportFormatArg {
    Jsonl,
    Csv,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    Create {
        dataset_id: Uuid,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        train_ratio: Option<f64>,
        #[arg(long)]
        validation_ratio: Option<f64>,
        #[arg(long)]
        test_ratio: Option<f64>,
        #[arg(long)]
        seed: Option<u64>,
        /// Keep rows with the same categorical dimension value in one split.
        #[arg(long)]
        group_dimension: Option<String>,
        /// Project TOML supplying defaults; explicit flags take precedence.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Approved curation manifest whose included rows exclusively form the snapshot.
        #[arg(long)]
        quality_manifest: Option<Uuid>,
    },
    List {
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Show {
        id: Uuid,
    },
    Members {
        id: Uuid,
        #[arg(long, value_enum)]
        split: Option<SnapshotSplitArg>,
    },
    Stats {
        id: Uuid,
    },
    Export {
        id: Uuid,
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        #[arg(long = "file")]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum QualityCommand {
    /// Resolve an outcome-oriented quality preset without writing state.
    PolicyPreview(QualityPolicyArgs),
    /// Pin the complete accepted source population and queue one immutable audit.
    AuditCreate(QualityAuditCreateArgs),
    /// Run or resume a queued audit in the foreground.
    AuditStart(QualityAuditExecutionArgs),
    /// Show immutable plan identity/counts and exact persisted audit progress.
    AuditStatus { run_id: Uuid },
    /// Poll exact persisted progress until the audit reaches a terminal state.
    AuditWatch {
        run_id: Uuid,
        #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(50..=60_000))]
        poll_ms: u64,
    },
    /// Request cancellation before any further evaluator request begins.
    AuditCancel { run_id: Uuid },
    /// Recover interrupted attempts and resume within the original finite budget.
    AuditRecover(QualityAuditExecutionArgs),
    /// List immutable normalized row assessments from one audit.
    Assessments(QualityAssessmentsArgs),
    /// Show persisted aggregate quality coverage for one completed audit.
    Summary { run_id: Uuid },
    /// Compile the latest append-only row reviews into a curation proposal.
    Curate { run_id: Uuid },
    /// Append a human decision for an assessment row or an exact report row.
    RowReview(QualityRowReviewArgs),
    /// Show and fully verify an immutable curation proposal.
    Proposal { proposal_id: Uuid },
    /// Append an exact proposal review and create a manifest on approval.
    ManifestReview(QualityManifestReviewArgs),
    /// Show and fully verify an approved curation manifest.
    Manifest { manifest_id: Uuid },
}

#[derive(Debug, Clone, clap::Args)]
pub struct QualityPolicyArgs {
    /// Overall evidence threshold and review depth.
    #[arg(long, value_enum, default_value_t = QualityPresetArg::Balanced)]
    pub preset: QualityPresetArg,
    /// Whether candidate text is permitted to leave this local process.
    #[arg(long, value_enum, default_value_t = QualityEgressArg::LocalOnly)]
    pub egress: QualityEgressArg,
    /// Whether an approved authenticity profile participates in scoring.
    #[arg(long, value_enum, default_value_t = QualityAuthenticityArg::Off)]
    pub authenticity: QualityAuthenticityArg,
    /// Optional hard provider-cost ceiling in millionths of a US dollar.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub max_cost_microusd: Option<u64>,
}

#[derive(Debug, clap::Args)]
pub struct QualityAuditCreateArgs {
    pub dataset_id: Uuid,
    #[command(flatten)]
    pub policy: QualityPolicyArgs,
    /// Replaceable evaluator implementation. V1 defaults to the deterministic offline fake.
    #[arg(long, value_enum, default_value_t = QualityEvaluatorArg::Fake)]
    pub evaluator: QualityEvaluatorArg,
}

#[derive(Debug, clap::Args)]
pub struct QualityAuditExecutionArgs {
    pub run_id: Uuid,
    /// Environment variable containing the process-local evaluator credential.
    #[arg(long, default_value = "SYNTH_OPENAI_API_KEY")]
    pub api_key_env: String,
}

#[derive(Debug, clap::Args)]
pub struct QualityAssessmentsArgs {
    pub run_id: Uuid,
    #[arg(long, value_enum)]
    pub verdict: Option<QualityVerdictArg>,
    #[command(flatten)]
    pub page: PageArgs,
}

#[derive(Debug, clap::Args)]
#[command(
    group(
        ArgGroup::new("target")
            .required(true)
            .multiple(false)
            .args(["assessment_id", "report_id"])
    ),
    group(
        ArgGroup::new("decision")
            .required(true)
            .multiple(false)
            .args(["include", "exclude", "request_reassessment"])
    )
)]
pub struct QualityRowReviewArgs {
    /// Existing compatibility target: review the immutable row referenced by this assessment.
    pub assessment_id: Option<Uuid>,
    /// Review an exact persisted report row, including invalid or unaudited rows.
    #[arg(long, requires = "source_row_id", conflicts_with = "assessment_id")]
    pub report_id: Option<Uuid>,
    /// Source row in --report-id; the complete report is verified before review.
    #[arg(long, requires = "report_id", conflicts_with = "assessment_id")]
    pub source_row_id: Option<Uuid>,
    #[arg(long)]
    pub include: bool,
    #[arg(long)]
    pub exclude: bool,
    #[arg(long)]
    pub request_reassessment: bool,
    #[arg(long, default_value = "local-operator")]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, clap::Args)]
#[command(group(
    ArgGroup::new("decision")
        .required(true)
        .multiple(false)
        .args(["approve", "reject", "request_revision"])
))]
pub struct QualityManifestReviewArgs {
    pub proposal_id: Uuid,
    #[arg(long)]
    pub approve: bool,
    #[arg(long)]
    pub reject: bool,
    #[arg(long)]
    pub request_revision: bool,
    #[arg(long, default_value = "local-operator")]
    pub reviewer: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QualityPresetArg {
    Fast,
    Balanced,
    Strict,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QualityEgressArg {
    LocalOnly,
    ExternalCandidateText,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum QualityAuthenticityArg {
    Off,
    WhenAvailable,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum QualityEvaluatorArg {
    Fake,
    OpenaiCompatible,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QualityVerdictArg {
    Qualified,
    Borderline,
    Quarantined,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SnapshotSplitArg {
    Train,
    Validation,
    Test,
}

#[derive(Debug, Subcommand)]
pub enum TrainingCommand {
    Run(TrainingRunArgs),
    /// Start a new provenance-linked run from an immutable transformer checkpoint.
    Continue(TrainingContinueArgs),
    List {
        #[arg(long)]
        snapshot_id: Option<Uuid>,
        #[arg(long, value_enum)]
        state: Option<TrainingRunStateArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    Status {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Checkpoints {
        run_id: Uuid,
    },
    Checkpoint {
        id: Uuid,
    },
    Predict {
        checkpoint_id: Uuid,
        #[arg(long)]
        text: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum EncoderCommand {
    /// Validate and register an immutable local BERT bundle identity.
    Register {
        #[arg(long)]
        name: String,
        path: PathBuf,
    },
    /// List registered local encoders.
    List,
    /// Inspect one registered encoder and its artifact metadata.
    Show { id: Uuid },
    /// Revalidate a registered encoder against the files currently at its path.
    Verify { id: Uuid },
    /// Run one finite, resumable post-review production improvement cycle.
    Optimize {
        #[command(subcommand)]
        command: Box<EncoderOptimizeCommand>,
    },
}

#[derive(Debug, Subcommand)]
pub enum EncoderOptimizeCommand {
    /// Resolve and verify a strict manifest without persisting or calling a backend.
    Preview(EncoderOptimizeManifestArgs),
    /// Persist one idempotent launch and reserve all downstream identities.
    Start(EncoderOptimizeManifestArgs),
    /// Show concise lifecycle state and the exact next operator action.
    Status(EncoderOptimizeRunArgs),
    /// Inspect the immutable definition, reservations, and linked lifecycle state.
    Inspect(EncoderOptimizeRunArgs),
    /// Verify the already-frozen repair proposal approval used by this run.
    ReviewRepair(EncoderOptimizeRunArgs),
    /// Verify the already-frozen native-delta approval used by this run.
    ReviewDelta(EncoderOptimizeRunArgs),
    /// Execute at most one reserved side-effecting stage and then stop.
    Resume(EncoderOptimizeRunArgs),
    /// Inspect or authorize a specifically reserved external call, if one exists.
    AuthorizeExternal(EncoderOptimizeAuthorizeArgs),
    /// Authorize exactly one selected candidate to use the sealed suite.
    AuthorizeSealed(EncoderOptimizeAuthorizeArgs),
    /// Cancel before another stage starts; running native work stops at its stage boundary.
    Cancel(EncoderOptimizeCancelArgs),
    /// Deeply verify the complete optimization, campaign, experiment, and evidence chain.
    Doctor(EncoderOptimizeRunArgs),
    /// Print a row-free, machine-verifiable provenance bundle.
    Provenance(EncoderOptimizeRunArgs),
    /// Print a deterministic operator report from persisted facts.
    Report(EncoderOptimizeRunArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct EncoderOptimizeManifestArgs {
    /// Strict TOML optimization manifest.
    #[arg(long)]
    pub manifest: PathBuf,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct EncoderOptimizeRunArgs {
    pub run_id: Uuid,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct EncoderOptimizeAuthorizeArgs {
    pub run_id: Uuid,
    #[arg(long, default_value = "local-operator")]
    pub authorized_by: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct EncoderOptimizeCancelArgs {
    pub run_id: Uuid,
    #[arg(long)]
    pub reason: String,
    #[command(flatten)]
    pub backend: NomosWorkspaceArgs,
}

#[derive(Debug, Subcommand)]
pub enum EvaluationCommand {
    Run {
        checkpoint_id: Uuid,
        /// Snapshot to evaluate. Defaults to the checkpoint's training snapshot.
        #[arg(long)]
        snapshot_id: Option<Uuid>,
        #[arg(long, value_enum)]
        split: Option<SnapshotSplitArg>,
        /// Project TOML supplying the default evaluation split.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        batch_size: Option<usize>,
        #[arg(long, value_delimiter = ',')]
        top_k: Option<Vec<usize>>,
        #[arg(long)]
        calibration_bins: Option<usize>,
        #[arg(long)]
        minimum_slice_support: Option<u64>,
        #[arg(long)]
        bootstrap_samples: Option<u32>,
        #[arg(long)]
        statistical_seed: Option<u64>,
        #[arg(long)]
        confidence_level: Option<f64>,
        #[arg(long = "dimension-intersection", value_name = "NAME,NAME")]
        dimension_intersections: Vec<String>,
    },
    List {
        #[arg(long)]
        checkpoint_id: Option<Uuid>,
        #[arg(long)]
        snapshot_id: Option<Uuid>,
        #[arg(long, value_enum)]
        state: Option<EvaluationRunStateArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    Status {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Metrics {
        id: Uuid,
    },
    Predictions {
        run_id: Uuid,
        #[arg(long, conflicts_with = "correct")]
        incorrect: bool,
        #[arg(long)]
        correct: bool,
        #[arg(long)]
        expected_label: Option<String>,
        #[arg(long)]
        predicted_label: Option<String>,
        #[arg(long)]
        minimum_confidence: Option<f64>,
        #[arg(long)]
        maximum_confidence: Option<f64>,
        #[arg(long = "dimension", value_name = "NAME=VALUE")]
        dimensions: Vec<String>,
        #[arg(long)]
        snapshot_member_id: Option<Uuid>,
        #[arg(long)]
        source_row_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Export {
        run_id: Uuid,
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        #[arg(long = "file")]
        output: PathBuf,
    },
    Compare {
        left_run_id: Uuid,
        right_run_id: Uuid,
    },
    Comparison {
        id: Uuid,
    },
    ComparisonExamples {
        id: Uuid,
        #[arg(long, value_enum)]
        change: ComparisonChangeArg,
        #[command(flatten)]
        page: PageArgs,
    },
    ComparisonExport {
        id: Uuid,
        #[arg(long = "file")]
        output: PathBuf,
    },
    Comparisons {
        #[command(flatten)]
        page: PageArgs,
    },
    Leaderboard {
        #[arg(long)]
        snapshot_id: Uuid,
        #[arg(long, value_enum)]
        split: SnapshotSplitArg,
        #[arg(long, value_enum, default_value_t = EvaluationMetricArg::MacroF1)]
        metric: EvaluationMetricArg,
    },
    Select {
        #[arg(long)]
        snapshot_id: Uuid,
        #[arg(long, value_enum)]
        split: SnapshotSplitArg,
        #[arg(long, value_enum, default_value_t = EvaluationMetricArg::MacroF1)]
        metric: EvaluationMetricArg,
        #[arg(long)]
        minimum_improvement: Option<f64>,
        #[arg(long)]
        required_confidence: Option<f64>,
    },
    Selection {
        id: Uuid,
    },
    Selections {
        #[command(flatten)]
        page: PageArgs,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EvaluationMetricArg {
    Accuracy,
    MacroF1,
    WeightedF1,
    LogLoss,
    BrierScore,
    ExpectedCalibrationError,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ComparisonChangeArg {
    Fixed,
    Regressed,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AnalysisFindingKindArg {
    ExpectedLabel,
    PredictedLabel,
    ConfusionPair,
    DimensionValue,
    Cell,
    DimensionIntersection,
    ConfidenceBand,
    CorrectnessConfidence,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AnalysisRankingArg {
    ErrorRate,
    ErrorCount,
    ErrorRateLift,
    ErrorShare,
    HighConfidenceError,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AnalysisFindingSortArg {
    Rank,
    ErrorRate,
    ErrorCount,
    ErrorRateLift,
    ErrorShare,
    HighConfidenceError,
    Support,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AnalysisEvidenceCategoryArg {
    HighestConfidenceError,
    LowestMarginError,
    MedianConfidenceError,
    StableError,
    CorrectContrast,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ComparisonDiagnosisCategoryArg {
    Fixed,
    Regressed,
    Persistent,
    HighConfidenceRegression,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FindingReviewStateArg {
    Open,
    Acknowledged,
    AcceptedLimitation,
    CandidateForMoreData,
    CandidateForLabelSchemaReview,
    ResolvedByLaterEvidence,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EvidenceExportFormatArg {
    Jsonl,
    Csv,
}

#[derive(Debug, Subcommand)]
pub enum AnalysisCommand {
    Create {
        evaluation_run_id: Uuid,
        #[arg(long, default_value_t = 1)]
        minimum_support: u64,
        #[arg(long)]
        comparison_id: Option<Uuid>,
        #[arg(long, default_value_t = 10)]
        maximum_examples: usize,
        #[arg(long = "confidence-threshold", value_delimiter = ',', default_values_t = [0.5, 0.8])]
        confidence_thresholds: Vec<f64>,
        #[arg(long, value_enum, default_value_t = AnalysisRankingArg::ErrorRate)]
        ranking: AnalysisRankingArg,
        /// Repeat with a comma-separated, sorted set such as difficulty,style.
        #[arg(long = "dimension-intersection")]
        dimension_intersections: Vec<String>,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_correct_contrasts: bool,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    List {
        #[arg(long)]
        evaluation_run_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Show {
        id: Uuid,
    },
    Findings {
        report_id: Uuid,
        #[arg(long, value_enum)]
        kind: Option<AnalysisFindingKindArg>,
        #[arg(long)]
        minimum_support: Option<u64>,
        #[arg(long)]
        minimum_error_count: Option<u64>,
        #[arg(long, value_enum, default_value_t = AnalysisFindingSortArg::Rank)]
        sort: AnalysisFindingSortArg,
        #[command(flatten)]
        page: PageArgs,
    },
    Finding {
        report_id: Uuid,
        finding_key: String,
    },
    Evidence {
        report_id: Uuid,
        finding_key: String,
        #[arg(long, value_enum)]
        category: Option<AnalysisEvidenceCategoryArg>,
        #[arg(long, value_enum)]
        format: Option<EvidenceExportFormatArg>,
        #[arg(long = "file", requires = "format")]
        output: Option<PathBuf>,
        #[command(flatten)]
        page: PageArgs,
    },
    HighConfidenceErrors {
        report_id: Uuid,
        #[command(flatten)]
        page: PageArgs,
    },
    WeakCells {
        report_id: Uuid,
        #[command(flatten)]
        page: PageArgs,
    },
    ComparisonGroup {
        report_id: Uuid,
        #[arg(value_enum)]
        category: ComparisonDiagnosisCategoryArg,
        #[command(flatten)]
        page: PageArgs,
    },
    Review {
        report_id: Uuid,
        finding_key: String,
        #[arg(long, value_enum)]
        state: FindingReviewStateArg,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        resolution_evaluation_run_id: Option<Uuid>,
        #[arg(long)]
        resolution_comparison_id: Option<Uuid>,
    },
    Reviews {
        #[arg(long)]
        report_id: Option<Uuid>,
        #[arg(long, value_enum)]
        state: Option<FindingReviewStateArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    Export {
        id: Uuid,
        #[arg(long = "file")]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum OptimizeCommand {
    /// Validate and resolve a decision-grade proposal without persisting it.
    Preview {
        analysis_report_id: Uuid,
        #[arg(long)]
        protocol: PathBuf,
        #[arg(long)]
        training_space: Option<PathBuf>,
    },
    Propose {
        analysis_report_id: Uuid,
        /// Decision-grade optimization protocol in TOML or JSON.
        #[arg(long, conflicts_with = "budget")]
        protocol: Option<PathBuf>,
        /// Historical proposal behavior; prefer --protocol for new work.
        #[arg(long, required_unless_present = "protocol")]
        budget: Option<u32>,
        #[arg(long, default_value_t = 1)]
        minimum_support: u64,
        /// Exact typed training configuration space in TOML or JSON.
        #[arg(long, requires = "protocol")]
        training_space: Option<PathBuf>,
    },
    /// Compare and persist four deterministic bounded policy scenarios.
    Scenarios {
        analysis_report_id: Uuid,
        #[arg(long)]
        protocol: PathBuf,
    },
    /// Show a persisted scenario comparison group.
    ScenarioShow {
        id: Uuid,
    },
    /// Persist one selected scenario as an independently reviewable proposal.
    ScenarioMaterialize {
        group_id: Uuid,
        scenario_id: String,
    },
    List {
        #[arg(long)]
        analysis_report_id: Option<Uuid>,
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Show {
        id: Uuid,
    },
    /// List normalized proposal recommendations with stable filters.
    Recommendations {
        id: Uuid,
        #[arg(long, value_enum)]
        kind: Option<OptimizationRecommendationKindArg>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        dimension: Option<String>,
        #[arg(long)]
        eligible: bool,
        #[arg(long)]
        minimum_score: Option<f64>,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Explain one data recommendation or training candidate.
    Explain {
        id: Uuid,
        recommendation_id: String,
    },
    /// Append an immutable human review decision.
    Review {
        id: Uuid,
        #[arg(long, value_enum)]
        state: ProposalReviewStateArg,
        #[arg(long = "select")]
        selected_recommendation_ids: Vec<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        superseding_proposal_id: Option<Uuid>,
        #[arg(long)]
        campaign_id: Option<Uuid>,
    },
    Apply {
        id: Uuid,
        #[arg(long)]
        approval_id: Uuid,
    },
    /// Historical unapproved application for pre-Slice-6.1 proposals only.
    LegacyApply {
        id: Uuid,
    },
    /// Create a new immutable proposal from current accepted-row coverage.
    Rebase {
        id: Uuid,
    },
    /// Build a fingerprinted finite training configuration space from persisted baseline facts.
    TrainingSpace {
        training_run_id: Uuid,
        checkpoint_id: Uuid,
        /// JSON or TOML document containing a `choices` array.
        #[arg(long)]
        choices: PathBuf,
        #[arg(long = "file")]
        output: PathBuf,
    },
    /// Show the bounded training experiment candidates without starting a run.
    TrainingCandidates {
        id: Uuid,
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        #[arg(long = "file")]
        output: Option<PathBuf>,
    },
    /// Export normalized data recommendation rows as JSONL or CSV.
    ExportRecommendations {
        id: Uuid,
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        #[arg(long = "file")]
        output: PathBuf,
    },
    /// Export a one-row proposal summary as JSONL or CSV.
    ExportSummary {
        id: Uuid,
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
        #[arg(long = "file")]
        output: PathBuf,
    },
    Export {
        id: Uuid,
        #[arg(long = "file")]
        output: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OptimizationRecommendationKindArg {
    DataGeneration,
    TrainingConfiguration,
    ReviewOnly,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProposalReviewStateArg {
    Open,
    ApprovedForPlanCreation,
    Rejected,
    Superseded,
    PartiallyAccepted,
    AcceptedTrainingExperimentCandidate,
    AcceptedReviewOnlyCandidates,
    CompletedAwaitingOutcomeAssessment,
}

#[derive(Debug, Subcommand)]
pub enum CampaignCommand {
    Create {
        proposal_id: Uuid,
        #[arg(long)]
        approval_id: Uuid,
    },
    List {
        #[arg(long)]
        proposal_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Show {
        id: Uuid,
    },
    /// Append one compatible existing artifact link.
    Link(CampaignLinkArgs),
    /// Record one immutable outcome assessment from an existing comparison.
    Assess {
        id: Uuid,
        #[arg(long)]
        comparison_id: Uuid,
        #[arg(long, default_value_t = 0.0)]
        minimum_accuracy_delta: f64,
        #[arg(long, default_value_t = 0.0)]
        minimum_macro_f1_delta: f64,
        #[arg(long)]
        allow_non_significant: bool,
    },
}

#[derive(Debug, clap::Args)]
pub struct CampaignLinkArgs {
    pub id: Uuid,
    #[arg(long)]
    pub generation_plan_id: Option<Uuid>,
    #[arg(long)]
    pub generation_job_id: Option<Uuid>,
    #[arg(long)]
    pub snapshot_id: Option<Uuid>,
    #[arg(long)]
    pub training_run_id: Option<Uuid>,
    #[arg(long)]
    pub checkpoint_id: Option<Uuid>,
    #[arg(long)]
    pub evaluation_run_id: Option<Uuid>,
    #[arg(long)]
    pub comparison_id: Option<Uuid>,
    #[arg(long)]
    pub analysis_report_id: Option<Uuid>,
}

#[derive(Debug, clap::Args)]
pub struct TrainingRunArgs {
    pub snapshot_id: Uuid,
    #[arg(long, value_enum)]
    pub backend: Option<TrainingBackendKind>,
    /// Registered encoder ID required by the bert-cpu backend.
    #[arg(long)]
    pub encoder_id: Option<Uuid>,
    #[arg(long)]
    pub feature_dimension: Option<usize>,
    #[arg(long)]
    pub epochs: Option<u32>,
    #[arg(long)]
    pub learning_rate: Option<f32>,
    #[arg(long)]
    pub l2: Option<f32>,
    #[arg(long)]
    pub checkpoint_every: Option<u32>,
    #[arg(long)]
    pub seed: Option<u64>,
    #[arg(long)]
    pub artifact_root: Option<PathBuf>,
    #[arg(long)]
    pub maximum_sequence_length: Option<usize>,
    #[arg(long)]
    pub batch_size: Option<usize>,
    #[arg(long)]
    pub weight_decay: Option<f64>,
    #[arg(long)]
    pub warmup_ratio: Option<f64>,
    #[arg(long)]
    pub gradient_clip_norm: Option<f64>,
    #[arg(long, value_enum)]
    pub encoder_mode: Option<EncoderTrainingModeArg>,
    /// Project TOML supplying defaults; explicit flags take precedence.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TrainingBackendKind {
    HashingLinear,
    BertCpu,
}

#[derive(Debug, clap::Args)]
pub struct TrainingContinueArgs {
    pub checkpoint_id: Uuid,
    /// Snapshot for the new run; defaults to the parent run's snapshot.
    #[arg(long)]
    pub snapshot_id: Option<Uuid>,
    #[arg(long)]
    pub epochs: Option<u32>,
    #[arg(long)]
    pub learning_rate: Option<f32>,
    #[arg(long)]
    pub checkpoint_every: Option<u32>,
    #[arg(long)]
    pub artifact_root: Option<PathBuf>,
    /// Project TOML supplying common training defaults and artifact root.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EncoderTrainingModeArg {
    FineTune,
    Frozen,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TrainingRunStateArg {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EvaluationRunStateArg {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// Show label/dimension cardinality without materializing every cell.
    Describe {
        dataset_id: Uuid,
    },
    Preview {
        dataset_id: Uuid,
    },
    Create {
        dataset_id: Uuid,
        /// Equal target count for every cell.
        #[arg(long, conflicts_with = "targets")]
        per_cell: Option<u32>,
        /// JSON file containing an array of explicit PlannedCell objects.
        #[arg(long, conflicts_with = "per_cell")]
        targets: Option<PathBuf>,
    },
    Show {
        id: Uuid,
    },
}

#[derive(Debug, Subcommand)]
pub enum AllocationCommand {
    /// Calculate complete absolute cell targets without writing anything.
    Preview(InitialAllocationArgs),
    /// Persist the allocation and its ordinary Slice 1 generation plan atomically.
    Create(InitialAllocationArgs),
    /// Inspect one immutable initial allocation.
    Show { id: Uuid },
    /// Derive compact label and dimension distributions for a persisted allocation.
    Explain { id: Uuid },
    /// List immutable initial allocations.
    List {
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
}

#[derive(Debug, Clone, clap::Args)]
pub struct InitialAllocationArgs {
    pub dataset_id: Uuid,
    /// Exact desired accepted-row total, including any reserved iteration rows.
    #[arg(long)]
    pub total_rows: u32,
    /// Rows held outside the initial generation plan for later iterations.
    #[arg(long, default_value_t = 0)]
    pub reserved_rows: u32,
    #[arg(long, value_enum, default_value_t = InitialAllocationPolicyArg::Balanced)]
    pub policy: InitialAllocationPolicyArg,
    /// JSON/TOML AllocationWeights document for weighted policies.
    #[arg(long)]
    pub weights: Option<PathBuf>,
    /// Absolute floor for every non-excluded cell in minimum-then-weighted mode.
    #[arg(long)]
    pub minimum_per_cell: Option<u32>,
    /// JSON array or TOML/JSON object with a `targets` array for explicit mode.
    #[arg(long)]
    pub targets: Option<PathBuf>,
    /// Exact constraints, or a TOML/JSON object with concise `rules` selectors.
    #[arg(long)]
    pub constraints: Option<PathBuf>,
    /// Include compact label/dimension distributions with the command result.
    #[arg(long)]
    pub explain: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum InitialAllocationPolicyArg {
    Balanced,
    Weighted,
    MinimumThenWeighted,
    Explicit,
}

#[derive(Debug, Subcommand)]
pub enum CohortCommand {
    Create {
        snapshot_id: Uuid,
        #[arg(long)]
        name: String,
        #[arg(long, value_enum)]
        split: SnapshotSplitArg,
        #[arg(long, value_enum, default_value_t = CohortOriginArg::InternalSnapshot)]
        origin: CohortOriginArg,
        #[arg(long, value_enum)]
        role: CohortRoleArg,
        #[arg(long)]
        reason: String,
    },
    List {
        #[arg(long)]
        snapshot_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Show {
        id: Uuid,
    },
    Assign {
        id: Uuid,
        #[arg(long, value_enum)]
        role: CohortRoleArg,
        #[arg(long)]
        reason: String,
    },
    Retire {
        id: Uuid,
        #[arg(long)]
        reason: String,
    },
    History {
        id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CohortOriginArg {
    InternalSnapshot,
    ExternalBenchmark,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CohortRoleArg {
    Training,
    Development,
    Diagnostic,
    SealedAcceptance,
    ExternalBenchmark,
}

#[derive(Debug, Subcommand)]
pub enum ExposureCommand {
    Record {
        cohort_id: Uuid,
        #[arg(long)]
        evaluation_run_id: Option<Uuid>,
        #[arg(long, value_enum)]
        purpose: ExposurePurposeArg,
        #[arg(long, value_enum, default_value_t = DisclosureLevelArg::Aggregate)]
        disclosure: DisclosureLevelArg,
        #[arg(long)]
        adaptation_eligible: bool,
        #[arg(long)]
        note: Option<String>,
        /// Required to atomically retire a sealed cohort after row-level disclosure.
        #[arg(long)]
        retirement_reason: Option<String>,
    },
    List {
        cohort_id: Uuid,
        #[arg(long, value_enum)]
        purpose: Option<ExposurePurposeArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    Risk {
        cohort_id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExposurePurposeArg {
    Training,
    DevelopmentEvaluation,
    Diagnosis,
    Comparison,
    Acceptance,
    ManualInspection,
    Advisor,
    Optimization,
    DatasetArchitecture,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DisclosureLevelArg {
    Aggregate,
    Slices,
    Predictions,
    RowContent,
}

#[derive(Debug, Subcommand)]
pub enum ContaminationCommand {
    Check {
        /// Cohorts to validate and compare.
        #[arg(long = "cohort", required = true, num_args = 1..)]
        cohort_ids: Vec<Uuid>,
        /// Dimension whose values identify related groups across cohorts.
        #[arg(long)]
        group_dimension: Option<String>,
        /// Strict TOML or JSON contamination policy; defaults to zero overlap.
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    Show {
        id: Uuid,
    },
    List {
        #[arg(long)]
        cohort_id: Option<Uuid>,
        #[arg(long, value_enum)]
        status: Option<ContaminationStatusArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    Override {
        id: Uuid,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        approved_by: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ContaminationStatusArg {
    Clean,
    Blocked,
}

#[derive(Debug, Subcommand)]
pub enum BenchmarkCommand {
    Create {
        #[arg(long)]
        definition: PathBuf,
    },
    Validate {
        id: Uuid,
    },
    Show {
        id: Uuid,
    },
    List {
        #[arg(long, value_enum)]
        kind: Option<BenchmarkSuiteKindArg>,
        #[arg(long)]
        cohort_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Assess {
        id: Uuid,
        /// Cohort-to-evaluation mapping: COHORT_ID=RUN_ID.
        #[arg(long = "run", required = true)]
        runs: Vec<String>,
        /// Cohort-to-paired-comparison mapping: COHORT_ID=COMPARISON_ID.
        #[arg(long = "comparison")]
        comparisons: Vec<String>,
        /// Required acknowledgement for a sealed acceptance suite.
        #[arg(long)]
        authorize_sealed: bool,
    },
    AssessmentShow {
        id: Uuid,
    },
    AssessmentList {
        #[arg(long)]
        suite_id: Option<Uuid>,
        #[arg(long)]
        checkpoint_id: Option<Uuid>,
        #[arg(long, value_enum)]
        state: Option<AcceptanceStateArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Show one immutable training-to-benchmark leakage clearance.
    TrainingCheckShow {
        id: Uuid,
    },
    /// Deeply validate one clearance against current snapshots, bundle, and roles.
    TrainingCheckValidate {
        id: Uuid,
    },
    /// List immutable training-to-benchmark clearances.
    TrainingCheckList {
        #[arg(long)]
        snapshot_id: Option<Uuid>,
        #[arg(long)]
        benchmark_bundle_id: Option<Uuid>,
        #[arg(long, value_enum)]
        status: Option<ContaminationStatusArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Derive and persist deterministic readiness evidence for one benchmark bundle.
    QualificationCreate {
        benchmark_bundle_id: Uuid,
        /// Strict JSON/TOML BenchmarkQualificationPolicy; defaults are explicit in output.
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Show one immutable benchmark qualification.
    QualificationShow {
        id: Uuid,
    },
    /// Recompute one qualification from persisted snapshot populations.
    QualificationValidate {
        id: Uuid,
    },
    /// List immutable benchmark qualifications.
    QualificationList {
        #[arg(long)]
        benchmark_bundle_id: Option<Uuid>,
        #[arg(long, value_enum)]
        readiness: Option<BenchmarkReadinessArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Record the one explicit human decision for a qualification.
    QualificationReview {
        id: Uuid,
        #[arg(long, value_enum)]
        decision: BenchmarkQualificationReviewDecisionArg,
        #[arg(long)]
        reviewed_by: String,
        #[arg(long)]
        rationale: String,
    },
    /// Show one append-only qualification review.
    QualificationReviewShow {
        id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BenchmarkSuiteKindArg {
    Development,
    SealedAcceptance,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AcceptanceStateArg {
    Pass,
    Fail,
    Inconclusive,
    Invalid,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BenchmarkReadinessArg {
    Ready,
    Blocked,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BenchmarkQualificationReviewDecisionArg {
    Approve,
    Reject,
}

#[derive(Debug, Subcommand)]
pub enum WorkflowCommand {
    Define {
        #[arg(long)]
        definition: PathBuf,
        /// Optional categorical dimension used as strict cross-suite group identity.
        #[arg(long)]
        group_dimension: Option<String>,
    },
    DefinitionShow {
        id: Uuid,
    },
    DefinitionList {
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[command(flatten)]
        page: PageArgs,
    },
    Start {
        definition_id: Uuid,
        /// Persist the initial owned attempt without executing slice work.
        #[arg(long)]
        initialize_only: bool,
    },
    Resume {
        id: Uuid,
    },
    /// Wait until a running workflow pauses or reaches a terminal/development-complete state.
    Watch {
        id: Uuid,
        #[arg(long, default_value_t = 500)]
        poll_ms: u64,
    },
    /// Approve the current bounded optimization proposal and resume the workflow.
    Approve {
        id: Uuid,
        #[arg(long = "recommendation-id")]
        recommendation_ids: Vec<String>,
        #[arg(long)]
        note: Option<String>,
    },
    ApprovalShow {
        id: Uuid,
    },
    StopShow {
        id: Uuid,
    },
    /// Explicitly run the configured sealed acceptance suite once.
    Finalize {
        id: Uuid,
    },
    /// Record the immutable promotion or rejection decision after final assessment.
    Promote {
        id: Uuid,
    },
    PromotionShow {
        id: Uuid,
    },
    Status {
        id: Uuid,
    },
    List {
        #[arg(long)]
        definition_id: Option<Uuid>,
        #[arg(long, value_enum)]
        state: Option<WorkflowRunStateArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    History {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WorkflowRunStateArg {
    Queued,
    Running,
    AwaitingApproval,
    AwaitingUser,
    DevelopmentComplete,
    Completed,
    Failed,
    Cancelled,
    Exhausted,
    Inconclusive,
}

#[derive(Debug, Subcommand)]
pub enum BackendCommand {
    Configure {
        #[arg(long, default_value = "https://api.openai.com/v1")]
        base_url: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long)]
        max_tokens: Option<u32>,
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Authenticate without generating and verify that the configured model exists.
    Check {
        /// Override the persisted provider base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Override the persisted provider model.
        #[arg(long)]
        model: Option<String>,
        /// Environment variable containing the API key.
        #[arg(long, default_value = "SYNTH_OPENAI_API_KEY")]
        api_key_env: String,
    },
    Show,
}

#[derive(Debug, clap::Args)]
pub struct GenerateArgs {
    pub plan_id: Uuid,
    #[arg(long, value_enum)]
    pub backend: Option<BackendKind>,
    #[arg(long)]
    pub batch_size: Option<u32>,
    #[arg(long)]
    pub max_retries: Option<u32>,
    #[arg(long)]
    pub max_attempt_multiplier: Option<u32>,
    /// Environment variable containing the API key.
    #[arg(long)]
    pub api_key_env: Option<String>,
    /// Project TOML supplying defaults; explicit flags take precedence.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum RecoveryCommand {
    /// Reconcile running workflows with their recorded owning processes.
    Scan,
    /// List pending interrupted workflows, or include their resolution history.
    List {
        #[arg(long)]
        all: bool,
    },
    /// Safely continue an interrupted generation job from persisted coverage.
    ResumeGeneration(ResumeGenerationArgs),
    /// Acknowledge an interrupted workflow without retrying it.
    Dismiss {
        #[arg(value_enum)]
        kind: WorkflowKindArg,
        id: Uuid,
    },
}

#[derive(Debug, clap::Args)]
pub struct ResumeGenerationArgs {
    pub job_id: Uuid,
    #[arg(long)]
    pub batch_size: Option<u32>,
    #[arg(long)]
    pub max_retries: Option<u32>,
    #[arg(long)]
    pub max_attempt_multiplier: Option<u32>,
    /// Environment variable containing the API key.
    #[arg(long)]
    pub api_key_env: Option<String>,
    /// Project TOML supplying defaults; explicit flags take precedence.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum WorkflowKindArg {
    Generation,
    Training,
    Evaluation,
    EncoderWorkflow,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ArtifactKindArg {
    ProjectBootstrap,
    ProjectPreparation,
    ProjectConfiguration,
    Dataset,
    SemanticProfile,
    SemanticBinding,
    GenerationSemanticContext,
    ResearchBrief,
    ResearchRun,
    ResearchEvidence,
    ResearchClaim,
    AuthenticityProfile,
    AuthenticityProfileReview,
    AuthenticityProfileBinding,
    GenerationAuthenticityContext,
    DatasetArchitectBrief,
    DatasetArchitectRun,
    DatasetArchitectureProposal,
    DatasetArchitectureReview,
    DatasetArchitectureApplication,
    GenerationStrategyContext,
    BenchmarkArchitectBrief,
    BenchmarkArchitectRun,
    BenchmarkArchitectEvidence,
    BenchmarkArchitectureProposal,
    BenchmarkArchitectureReview,
    BenchmarkAcquisitionHandoff,
    InitialAllocation,
    GenerationPlan,
    GenerationJob,
    GenerationQualityContract,
    GenerationSupervisorRun,
    SupervisorQualificationHandoff,
    SupervisorQualificationApplication,
    DatasetImport,
    DatasetSourceRow,
    QualityAuditPlan,
    QualitySemanticGuidance,
    QualityAuditRun,
    QualityEvaluatorAttempt,
    RowQualityAssessment,
    DatasetQualityReport,
    RowQualityReview,
    CurationProposal,
    CurationManifestReview,
    ApprovedCurationManifest,
    CurationApplication,
    Snapshot,
    BaseModel,
    TrainingRun,
    Checkpoint,
    EvaluationRun,
    EvaluationComparison,
    ModelSelection,
    AnalysisReport,
    AnalysisFindingReview,
    OptimizationProposal,
    OptimizationProposalReview,
    OptimizationCampaign,
    OptimizationCampaignLink,
    OptimizationOutcome,
    BenchmarkSuite,
    ContaminationReport,
    BenchmarkBundle,
    BenchmarkQualification,
    BenchmarkQualificationReview,
    TrainingBenchmarkCheck,
    WorkflowDefinition,
    WorkflowRun,
    AcceptanceAssessment,
    AdvisoryAssessment,
    WorkflowApproval,
    StopDecision,
    ModelPromotion,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BackendKind {
    Fake,
    OpenaiCompatible,
}

#[derive(Debug, Subcommand)]
pub enum JobCommand {
    List {
        #[arg(long)]
        dataset_id: Option<Uuid>,
        #[arg(long)]
        plan_id: Option<Uuid>,
        #[arg(long, value_enum)]
        state: Option<JobStateArg>,
        #[command(flatten)]
        page: PageArgs,
    },
    Status {
        id: Uuid,
    },
    /// Inspect the immutable non-secret runtime configuration pinned to a job.
    Execution {
        id: Uuid,
    },
    /// Inspect every persisted provider request and outcome for a job.
    Attempts {
        id: Uuid,
    },
    /// Reconstruct and inspect a pinned request without calling the provider.
    Prompt {
        id: Uuid,
        #[arg(long, default_value_t = 0)]
        cell_index: usize,
        #[arg(long)]
        requested_count: Option<u32>,
    },
    Cancel {
        id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum JobStateArg {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, clap::Args)]
pub struct RowsArgs {
    #[arg(long)]
    pub dataset_id: Option<Uuid>,
    #[arg(long)]
    pub job_id: Option<Uuid>,
    #[arg(long, value_enum)]
    pub status: Option<RowStatus>,
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    #[arg(long)]
    pub summary: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RowStatus {
    Accepted,
    Rejected,
}

#[derive(Debug, clap::Args)]
pub struct ExportArgs {
    pub dataset_id: Uuid,
    #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
    pub format: ExportFormat,
    #[arg(long = "file")]
    pub output: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExportFormat {
    Jsonl,
    Csv,
}
