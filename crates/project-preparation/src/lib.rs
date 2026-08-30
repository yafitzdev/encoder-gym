//! Declarative, side-effect-free preparation of a finite encoder workflow.

mod bootstrap;
mod compiler;
mod domain;
mod manifest;
mod ports;

pub use bootstrap::{BootstrapError, compile_bootstrap, preview_bootstrap};
pub use compiler::{PreparationError, compile_project, preview_project};
pub use domain::{
    BootstrapBundle, BootstrapPreview, BootstrapSourceBundle, BootstrapSourcePreview,
    BootstrapSourceSummary, CellTargetPreview, CohortEvidence, ContaminationPreview,
    PreparationBundle, PreparationEvidence, PreparationIssue, PreparationPreview,
    PreparationStoreError, PreparedProject, ProjectBootstrap,
};
pub use manifest::{
    BootstrapCohortManifest, BootstrapManifest, BootstrapSuiteManifest, CohortManifest,
    ContaminationManifest, LocalSourceManifest, PreparationManifest, SuiteManifest,
    WorkflowManifest,
};
pub use ports::{BootstrapStore, BoxFuture, PreparationStore};
