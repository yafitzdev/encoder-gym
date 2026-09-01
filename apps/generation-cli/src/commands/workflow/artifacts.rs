use anyhow::{Context, ensure};
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

pub(super) fn stable_artifact_uuid(value: &impl serde::Serialize) -> anyhow::Result<uuid::Uuid> {
    let fingerprint = artifact_core::fingerprint(value)?;
    let hex = fingerprint
        .strip_prefix("sha256:")
        .context("artifact fingerprint has no sha256 prefix")?;
    ensure!(hex.len() >= 32, "artifact fingerprint is too short");
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&hex[start..start + 2], 16)
            .context("artifact fingerprint is not hexadecimal")?;
    }
    // RFC 9562 version 8 is reserved for application-defined deterministic
    // UUIDs. Preserve the RFC variant bits while deriving the payload from the
    // canonical artifact fingerprint.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid::Uuid::from_bytes(bytes))
}
