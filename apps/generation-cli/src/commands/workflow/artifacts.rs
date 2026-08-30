use anyhow::Context;
use workflow_core::workflow::{
    StageAttemptState, WorkflowArtifactLink, WorkflowStage, WorkflowStageAttempt,
};

pub(super) fn link(
    kind: &str,
    artifact_id: uuid::Uuid,
    artifact_fingerprint: &str,
) -> WorkflowArtifactLink {
    WorkflowArtifactLink {
        kind: kind.to_owned(),
        artifact_id,
        artifact_fingerprint: artifact_fingerprint.to_owned(),
    }
}

pub(super) fn artifact_id(
    history: &[WorkflowStageAttempt],
    kind: &str,
) -> anyhow::Result<uuid::Uuid> {
    history
        .iter()
        .rev()
        .flat_map(|attempt| attempt.artifacts.iter().rev())
        .find(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .with_context(|| format!("workflow artifact is missing: {kind}"))
}

pub(super) fn artifact_link(
    history: &[WorkflowStageAttempt],
    kind: &str,
) -> anyhow::Result<WorkflowArtifactLink> {
    history
        .iter()
        .rev()
        .flat_map(|attempt| attempt.artifacts.iter().rev())
        .find(|artifact| artifact.kind == kind)
        .cloned()
        .with_context(|| format!("workflow artifact is missing: {kind}"))
}

pub(super) fn artifact_ids(history: &[WorkflowStageAttempt], kind: &str) -> Vec<uuid::Uuid> {
    history
        .iter()
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .collect()
}

pub(super) fn artifact_ids_for_stage(
    history: &[WorkflowStageAttempt],
    stage: WorkflowStage,
    iteration: u32,
    kind: &str,
) -> Vec<uuid::Uuid> {
    history
        .iter()
        .filter(|attempt| {
            attempt.stage == stage
                && attempt.iteration == iteration
                && attempt.state == StageAttemptState::Completed
        })
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| artifact.kind == kind)
        .map(|artifact| artifact.artifact_id)
        .collect()
}
