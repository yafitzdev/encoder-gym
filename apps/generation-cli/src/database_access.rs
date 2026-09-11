//! Explicit database access policy for every CLI operation.
//! Exhaustive matches force new commands to declare whether they mutate facts.
//! Export commands may write their requested file while reading database facts.
use crate::cli::{
    AdvisorCommand, AllocationCommand, AnalysisCommand, ArchitectCommand, BackendCommand,
    BenchmarkArchitectCommand, BenchmarkCommand, BenchmarkGenerationCommand, CampaignCommand,
    CohortCommand, Command, ConfigCommand, ContaminationCommand, DatasetCommand, EncoderCommand,
    EncoderOptimizeCommand, EvaluationCommand, ExperimentCommand, ExposureCommand, JobCommand,
    OptimizeCommand, PlanCommand, ProductionCampaignCommand, ProductionRepairCommand,
    ProjectCommand, QualityCommand, RecoveryCommand, ResearchCommand, SemanticCommand,
    SnapshotCommand, SupervisorCommand, TrainingCommand, WorkflowCommand,
};
use encoder_experiment_sqlite::SqliteExperimentStore;
use synthetic_data_sqlite::SqliteStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseAccess {
    ReadOnly,
    ReadWrite,
}

impl DatabaseAccess {
    pub async fn classification(self, url: &str) -> anyhow::Result<SqliteStore> {
        Ok(match self {
            Self::ReadOnly => SqliteStore::connect_read_only(url).await?,
            Self::ReadWrite => SqliteStore::connect(url).await?,
        })
    }
    pub async fn production(self, url: &str) -> anyhow::Result<SqliteExperimentStore> {
        Ok(match self {
            Self::ReadOnly => SqliteExperimentStore::connect_read_only(url).await?,
            Self::ReadWrite => SqliteExperimentStore::connect(url).await?,
        })
    }
}

impl Command {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Workspace { .. } => {
                unreachable!("workspace commands use only their own project database")
            }
            Self::Database { .. } | Self::Generate(..) => DatabaseAccess::ReadWrite,
            Self::Advisor { command } => match command {
                AdvisorCommand::Show { .. } | AdvisorCommand::List { .. } => {
                    DatabaseAccess::ReadOnly
                }
            },
            Self::Doctor(..)
            | Self::Provenance { .. }
            | Self::Coverage { .. }
            | Self::Rows(..)
            | Self::Export(..) => DatabaseAccess::ReadOnly,
            Self::Config { command } => command.database_access(),
            Self::Project { command } => command.database_access(),
            Self::Dataset { command } => command.database_access(),
            Self::Semantic { command } => command.database_access(),
            Self::Research { command } => command.database_access(),
            Self::Architect { command } => command.database_access(),
            Self::BenchmarkArchitect { command } => command.database_access(),
            Self::Quality { command } => command.database_access(),
            Self::Supervisor { command } => command.database_access(),
            Self::Snapshot { command } => command.database_access(),
            Self::Training { command } => command.database_access(),
            Self::Encoder { command } => command.database_access(),
            Self::Evaluation { command } => command.database_access(),
            Self::Analysis { command } => command.database_access(),
            Self::Optimize { command } => command.database_access(),
            Self::Campaign { command } => command.database_access(),
            Self::Plan { command } => command.database_access(),
            Self::Allocation { command } => command.database_access(),
            Self::Cohort { command } => command.database_access(),
            Self::Exposure { command } => command.database_access(),
            Self::Contamination { command } => command.database_access(),
            Self::Benchmark { command } => command.database_access(),
            Self::Workflow { command } => command.database_access(),
            Self::Backend { command } => command.database_access(),
            Self::Recovery { command } => command.database_access(),
            Self::Job { command } => command.database_access(),
            Self::Experiment { command } => command.database_access(),
            Self::BenchmarkGeneration { command } => command.database_access(),
            Self::ProductionCampaign { command } => command.database_access(),
            Self::ProductionRepair { command } => command.database_access(),
        }
    }
}

impl ConfigCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Init { .. } => DatabaseAccess::ReadWrite,
            Self::Validate { .. } | Self::Show { .. } | Self::ConstructionPreview { .. } => {
                DatabaseAccess::ReadOnly
            }
        }
    }
}

impl ProjectCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Bootstrap { .. } | Self::Prepare { .. } => DatabaseAccess::ReadWrite,
            Self::BootstrapPreview { .. }
            | Self::BootstrapShow { .. }
            | Self::BootstrapList { .. }
            | Self::Preview { .. }
            | Self::Show { .. }
            | Self::List { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl DatasetCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Import(args) if args.dry_run => DatabaseAccess::ReadOnly,
            Self::Create { .. } | Self::Import { .. } => DatabaseAccess::ReadWrite,
            Self::List { .. }
            | Self::Show { .. }
            | Self::Imports { .. }
            | Self::ImportShow { .. }
            | Self::ImportRows { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl SemanticCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::ProfileCreate { .. }
            | Self::ProfileRevise { .. }
            | Self::Bind { .. }
            | Self::Unbind { .. } => DatabaseAccess::ReadWrite,
            Self::ProfileList { .. }
            | Self::ProfileShow { .. }
            | Self::Bindings { .. }
            | Self::Resolve { .. }
            | Self::Suggest { .. }
            | Self::JobContext { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl ResearchCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Start { .. }
            | Self::Cancel { .. }
            | Self::Recover { .. }
            | Self::Review { .. }
            | Self::Bind { .. } => DatabaseAccess::ReadWrite,
            Self::BriefValidate { .. }
            | Self::Status { .. }
            | Self::Watch { .. }
            | Self::Evidence { .. }
            | Self::Profile { .. }
            | Self::Context { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl ArchitectCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::BriefValidate { .. }
            | Self::Start { .. }
            | Self::Cancel { .. }
            | Self::Recover { .. }
            | Self::Review { .. }
            | Self::Apply { .. } => DatabaseAccess::ReadWrite,
            Self::Status { .. }
            | Self::Watch { .. }
            | Self::Proposal { .. }
            | Self::Context { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl BenchmarkArchitectCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Start { .. }
            | Self::Cancel { .. }
            | Self::Recover { .. }
            | Self::Review { .. }
            | Self::Handoff { .. } => DatabaseAccess::ReadWrite,
            Self::BriefValidate { .. }
            | Self::Status { .. }
            | Self::Watch { .. }
            | Self::Evidence { .. }
            | Self::Proposal { .. }
            | Self::HandoffShow { .. }
            | Self::Conformance { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl QualityCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::AuditCreate { .. }
            | Self::AuditStart { .. }
            | Self::AuditCancel { .. }
            | Self::AuditRecover { .. }
            | Self::Curate { .. }
            | Self::RowReview { .. }
            | Self::ManifestReview { .. } => DatabaseAccess::ReadWrite,
            Self::PolicyPreview { .. }
            | Self::AuditStatus { .. }
            | Self::AuditWatch { .. }
            | Self::Assessments { .. }
            | Self::Summary { .. }
            | Self::Proposal { .. }
            | Self::Manifest { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl SupervisorCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::ContractCreate { .. }
            | Self::Start { .. }
            | Self::Run { .. }
            | Self::Cancel { .. }
            | Self::Recover { .. }
            | Self::Diagnose { .. }
            | Self::RevisionReview { .. }
            | Self::RevisionAuthorize { .. }
            | Self::Canary { .. }
            | Self::Finalize { .. } => DatabaseAccess::ReadWrite,
            Self::ContractPreview { .. }
            | Self::ContractShow { .. }
            | Self::Status { .. }
            | Self::Watch { .. }
            | Self::Issues { .. }
            | Self::RevisionShow { .. }
            | Self::StrategyCoverage { .. }
            | Self::QualificationShow { .. }
            | Self::TraceRow { .. }
            | Self::Integrity { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl SnapshotCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. } => DatabaseAccess::ReadWrite,
            Self::List { .. }
            | Self::Show { .. }
            | Self::Members { .. }
            | Self::Stats { .. }
            | Self::Export { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl TrainingCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Run { .. } | Self::Continue { .. } | Self::Cancel { .. } => {
                DatabaseAccess::ReadWrite
            }
            Self::List { .. }
            | Self::Status { .. }
            | Self::Checkpoints { .. }
            | Self::Checkpoint { .. }
            | Self::Predict { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl EncoderCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Register { .. } => DatabaseAccess::ReadWrite,
            Self::List { .. } | Self::Show { .. } | Self::Verify { .. } => DatabaseAccess::ReadOnly,
            Self::Optimize { command } => command.database_access(),
        }
    }
}

impl EvaluationCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Run { .. } | Self::Cancel { .. } | Self::Compare { .. } | Self::Select { .. } => {
                DatabaseAccess::ReadWrite
            }
            Self::List { .. }
            | Self::Status { .. }
            | Self::Metrics { .. }
            | Self::Predictions { .. }
            | Self::Export { .. }
            | Self::Comparison { .. }
            | Self::ComparisonExamples { .. }
            | Self::ComparisonExport { .. }
            | Self::Comparisons { .. }
            | Self::Leaderboard { .. }
            | Self::Selection { .. }
            | Self::Selections { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl AnalysisCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. } | Self::Review { .. } => DatabaseAccess::ReadWrite,
            Self::List { .. }
            | Self::Show { .. }
            | Self::Findings { .. }
            | Self::Finding { .. }
            | Self::Evidence { .. }
            | Self::HighConfidenceErrors { .. }
            | Self::WeakCells { .. }
            | Self::ComparisonGroup { .. }
            | Self::Reviews { .. }
            | Self::Export { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl OptimizeCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Propose { .. }
            | Self::Scenarios { .. }
            | Self::ScenarioMaterialize { .. }
            | Self::Review { .. }
            | Self::Apply { .. }
            | Self::LegacyApply { .. }
            | Self::Rebase { .. } => DatabaseAccess::ReadWrite,
            Self::Preview { .. }
            | Self::ScenarioShow { .. }
            | Self::List { .. }
            | Self::Show { .. }
            | Self::Recommendations { .. }
            | Self::Explain { .. }
            | Self::TrainingSpace { .. }
            | Self::TrainingCandidates { .. }
            | Self::ExportRecommendations { .. }
            | Self::ExportSummary { .. }
            | Self::Export { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl CampaignCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. } | Self::Link { .. } | Self::Assess { .. } => {
                DatabaseAccess::ReadWrite
            }
            Self::List { .. } | Self::Show { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl PlanCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. } => DatabaseAccess::ReadWrite,
            Self::Describe { .. } | Self::Preview { .. } | Self::Show { .. } => {
                DatabaseAccess::ReadOnly
            }
        }
    }
}

impl AllocationCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. } => DatabaseAccess::ReadWrite,
            Self::Preview { .. } | Self::Show { .. } | Self::Explain { .. } | Self::List { .. } => {
                DatabaseAccess::ReadOnly
            }
        }
    }
}

impl CohortCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. } | Self::Assign { .. } | Self::Retire { .. } => {
                DatabaseAccess::ReadWrite
            }
            Self::List { .. } | Self::Show { .. } | Self::History { .. } => {
                DatabaseAccess::ReadOnly
            }
        }
    }
}

impl ExposureCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Record { .. } => DatabaseAccess::ReadWrite,
            Self::List { .. } | Self::Risk { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl ContaminationCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Check { .. } | Self::Override { .. } => DatabaseAccess::ReadWrite,
            Self::Show { .. } | Self::List { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl BenchmarkCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. }
            | Self::Assess { .. }
            | Self::QualificationCreate { .. }
            | Self::QualificationReview { .. } => DatabaseAccess::ReadWrite,
            Self::Validate { .. }
            | Self::Show { .. }
            | Self::List { .. }
            | Self::AssessmentShow { .. }
            | Self::AssessmentList { .. }
            | Self::TrainingCheckShow { .. }
            | Self::TrainingCheckValidate { .. }
            | Self::TrainingCheckList { .. }
            | Self::QualificationShow { .. }
            | Self::QualificationValidate { .. }
            | Self::QualificationList { .. }
            | Self::QualificationReviewShow { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl WorkflowCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Define { .. }
            | Self::Start { .. }
            | Self::Resume { .. }
            | Self::Approve { .. }
            | Self::Finalize { .. }
            | Self::Promote { .. }
            | Self::Cancel { .. } => DatabaseAccess::ReadWrite,
            Self::DefinitionShow { .. }
            | Self::DefinitionList { .. }
            | Self::Watch { .. }
            | Self::ApprovalShow { .. }
            | Self::StopShow { .. }
            | Self::PromotionShow { .. }
            | Self::Status { .. }
            | Self::List { .. }
            | Self::History { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl BackendCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Configure { .. } => DatabaseAccess::ReadWrite,
            Self::Check { .. } | Self::Show { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl RecoveryCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Scan { .. } | Self::ResumeGeneration { .. } | Self::Dismiss { .. } => {
                DatabaseAccess::ReadWrite
            }
            Self::List { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl JobCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Cancel { .. } => DatabaseAccess::ReadWrite,
            Self::List { .. }
            | Self::Status { .. }
            | Self::Execution { .. }
            | Self::Attempts { .. }
            | Self::Prompt { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl ExperimentCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::NomosRegister { .. }
            | Self::Prepare { .. }
            | Self::Start { .. }
            | Self::RunDevelopment { .. }
            | Self::AuthorizeSealed { .. }
            | Self::RunSealed { .. } => DatabaseAccess::ReadWrite,
            Self::NomosVerify { .. } | Self::Status { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl BenchmarkGenerationCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::NomosBuildAuthority { .. }
            | Self::MigrateConsumed { .. }
            | Self::NomosCreate { .. }
            | Self::Import { .. }
            | Self::MarkReady { .. }
            | Self::ActivateInitial { .. }
            | Self::ActivateSuccessor { .. }
            | Self::Exhaust { .. } => DatabaseAccess::ReadWrite,
            Self::Show { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl ProductionCampaignCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Create { .. }
            | Self::BindGeneration { .. }
            | Self::Prepare { .. }
            | Self::Start { .. }
            | Self::Advance { .. }
            | Self::AuthorizeSealed { .. }
            | Self::LinkRenewalHandoff { .. }
            | Self::Complete { .. } => DatabaseAccess::ReadWrite,
            Self::Show { .. }
            | Self::Doctor { .. }
            | Self::Provenance { .. }
            | Self::Readiness { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl ProductionRepairCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Diagnose { .. }
            | Self::Propose { .. }
            | Self::Review { .. }
            | Self::Apply { .. }
            | Self::DeltaBuild { .. }
            | Self::DeltaReview { .. }
            | Self::DeltaSelect { .. }
            | Self::TrainingSnapshotBuild { .. }
            | Self::TrainingExperimentPrepare { .. } => DatabaseAccess::ReadWrite,
            Self::Show { .. }
            | Self::Doctor { .. }
            | Self::Evidence { .. }
            | Self::ProposalShow { .. }
            | Self::ProposalDoctor { .. }
            | Self::DeltaShow { .. }
            | Self::DeltaDoctor { .. }
            | Self::TrainingSnapshotShow { .. }
            | Self::TrainingSnapshotDoctor { .. } => DatabaseAccess::ReadOnly,
        }
    }
}

impl EncoderOptimizeCommand {
    pub fn database_access(&self) -> DatabaseAccess {
        match self {
            Self::Start { .. }
            | Self::Resume { .. }
            | Self::Drive { .. }
            | Self::AuthorizeSealed { .. }
            | Self::Cancel { .. } => DatabaseAccess::ReadWrite,
            Self::Preview { .. }
            | Self::Status { .. }
            | Self::Inspect { .. }
            | Self::ReviewRepair { .. }
            | Self::ReviewDelta { .. }
            | Self::AuthorizeExternal { .. }
            | Self::Doctor { .. }
            | Self::Provenance { .. }
            | Self::Report { .. } => DatabaseAccess::ReadOnly,
        }
    }
}
