use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use uuid::Uuid;

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
    /// Create and inspect immutable dataset snapshots.
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
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
    /// JSON array or TOML/JSON object with a `constraints` array.
    #[arg(long)]
    pub constraints: Option<PathBuf>,
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

#[derive(Debug, Subcommand)]
pub enum WorkflowCommand {
    Define {
        #[arg(long)]
        definition: PathBuf,
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
    GenerationPlan,
    GenerationJob,
    DatasetImport,
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
