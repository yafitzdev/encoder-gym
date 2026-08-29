//! Declarative, side-effect-free preparation of a finite encoder workflow.

mod compiler;
mod domain;
mod manifest;
mod ports;

pub use compiler::{PreparationError, compile_project, preview_project};
pub use domain::{
    CellTargetPreview, CohortEvidence, ContaminationPreview, PreparationBundle,
    PreparationEvidence, PreparationIssue, PreparationPreview, PreparationStoreError,
    PreparedProject,
};
pub use manifest::{
    CohortManifest, ContaminationManifest, PreparationManifest, SuiteManifest, WorkflowManifest,
};
pub use ports::{BoxFuture, PreparationStore};
