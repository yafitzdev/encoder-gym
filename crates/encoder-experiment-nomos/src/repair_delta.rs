//! Trusted Nomos boundary for deterministic, payload-free repair-delta qualification.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use chrono::Utc;
use encoder_experiment_core::{
    domain::{EvidenceRole, ExternalArtifactIdentity, ExternalProjectSnapshot, ParameterValue},
    ports::EncoderTaskBackend,
};
use encoder_repair_core::{
    diagnosis::ComparativeDiagnosis,
    ports::{BoxFuture, NativeRepairDeltaBackend, NativeRepairDeltaBackendError},
    proposal::{RepairActionKind, RepairProposal, RepairProposalApplication, RepairProposalReview},
    quality::{NativeDeltaCandidateSet, NativeRepairAuditReference, NativeRepairRowEvidence},
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use super::{NomosBackend, bounded_text, workspace_relative};

const REQUEST_SCHEMA: &str = "encoder-gym-repair-delta-request.v1";
const OUTPUT_SCHEMA: &str = "encoder-gym-repair-delta-output.v1";
const EVIDENCE_SCHEMA: &str = "encoder-gym-native-repair-evidence.v1";
const GENERATION_MANIFEST_SCHEMA: &str = "encoder-gym-repair-delta-manifest.v1";
const DATASET_VERSION: &str = "nomos-encoder-gym-repair-delta.v1";
const RECIPE_VERSION: &str = "encoder-gym-repair-delta-recipe.v1";
const VALIDATOR_VERSION: &str = "encoder-gym-repair-delta-validator.v1";
const MATRIX_VERSION: &str = "matrix.encoder-gym-repair-delta.v1";
const MATERIALIZER_MODULE: &str = "tools.generate_encoder_gym_repair_delta_v1";
const OUTPUT_ROOT: &str = "runs/encoder_gym_repair";
const SUPPORTED_TARGETS: [&str; 3] = [
    "generic_finalize_selection",
    "generic_near_neighbor",
    "retired_bounded_change_preflight",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NativeReferenceRole {
    BaseTraining,
    Development,
    Sealed,
}

impl NativeReferenceRole {
    const fn evidence_role(self) -> EvidenceRole {
        match self {
            Self::BaseTraining => EvidenceRole::Training,
            Self::Development => EvidenceRole::Development,
            Self::Sealed => EvidenceRole::SealedAcceptance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReferenceRequest {
    role: NativeReferenceRole,
    key: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRepairRequest {
    schema_version: String,
    project_revision: String,
    seed: u64,
    target_counts: BTreeMap<String, u64>,
    references: Vec<NativeReferenceRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeArtifact {
    key: String,
    bytes: u64,
    sha256: String,
    row_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeEvidenceArtifact {
    key: String,
    bytes: u64,
    sha256: String,
    fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeGenerationManifest {
    schema_version: String,
    dataset_version: String,
    matrix_version: String,
    recipe_version: String,
    validator_version: String,
    seed: u64,
    project_revision: String,
    target_counts: BTreeMap<String, u64>,
    row_count: u64,
    recipe_fingerprint: String,
    generator_fingerprint: String,
    rows_fingerprint: String,
    fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeQualitySummary {
    candidate_rows: u64,
    invalid_rows: u64,
    exact_duplicate_rows: u64,
    normalized_duplicate_rows: u64,
    source_contamination_rows: u64,
    group_contamination_rows: u64,
    lineage_contamination_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeOutputManifest {
    schema_version: String,
    request_fingerprint: String,
    generation: NativeGenerationManifest,
    delta_artifact: NativeArtifact,
    quality_evidence: NativeEvidenceArtifact,
    summary: NativeQualitySummary,
    reference_count: u64,
    fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeValidationSummary {
    valid: bool,
    issues: Vec<String>,
    rows: u64,
    target_counts: BTreeMap<String, u64>,
    unique_decision_ids: u64,
    unique_exact_inputs: u64,
    unique_normalized_inputs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeAuditReference {
    role: NativeReferenceRole,
    key: String,
    bytes: u64,
    sha256: String,
    row_count: u64,
    identity_set_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRowEvidence {
    row_id_hash: String,
    content_fingerprint: String,
    normalized_content_fingerprint: String,
    source_identity_fingerprint: String,
    group_identity_fingerprint: String,
    lineage_identity_fingerprint: String,
    target_key: String,
    recipe_fingerprint: String,
    generator_fingerprint: String,
    seed: u64,
    project_revision: String,
    task_valid: bool,
    task_validation_reasons: Vec<String>,
    exact_duplicate_matches: u32,
    normalized_duplicate_matches: u32,
    source_contamination_matches: u32,
    group_contamination_matches: u32,
    lineage_contamination_matches: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeQualityEvidence {
    schema_version: String,
    dataset_version: String,
    recipe_version: String,
    validator_version: String,
    seed: u64,
    project_revision: String,
    target_counts: BTreeMap<String, u64>,
    recipe_fingerprint: String,
    generator_fingerprint: String,
    validation: NativeValidationSummary,
    references: Vec<NativeAuditReference>,
    rows: Vec<NativeRowEvidence>,
    summary: NativeQualitySummary,
    request_fingerprint: String,
    delta_artifact: NativeArtifact,
    fingerprint: String,
}

impl NativeRepairDeltaBackend for NomosBackend {
    fn build_native_delta(
        &self,
        project: ExternalProjectSnapshot,
        diagnosis: ComparativeDiagnosis,
        proposal: RepairProposal,
        approval: RepairProposalReview,
        approval_predecessor: Option<RepairProposalReview>,
        application: RepairProposalApplication,
    ) -> BoxFuture<'_, Result<NativeDeltaCandidateSet, NativeRepairDeltaBackendError>> {
        Box::pin(async move {
            let started_at = Utc::now();
            project.validate_integrity().map_err(repair_error)?;
            proposal
                .validate_integrity(&diagnosis)
                .map_err(repair_error)?;
            proposal
                .context
                .execution_project
                .verify(&project)
                .map_err(repair_error)?;
            proposal
                .context
                .benchmark
                .validate_at(started_at)
                .map_err(repair_error)?;
            if started_at > proposal.expires_at {
                return Err(repair_error(
                    "repair proposal expired before delta construction",
                ));
            }
            approval
                .validate_against(&proposal, approval_predecessor.as_ref())
                .map_err(repair_error)?;
            application
                .validate_against(&proposal, &approval)
                .map_err(repair_error)?;
            self.require_current_project(&project)
                .map_err(repair_error)?;
            self.inspect(project.clone()).await.map_err(repair_error)?;

            let (seed, maximum_seconds) = native_action_configuration(&proposal)?;
            let request = native_request(&project, &proposal, seed)?;
            let request_fingerprint = artifact_core::fingerprint(&request).map_err(repair_error)?;
            let request_path = write_request(&self.root, &request, &request_fingerprint)?;
            let request_relative =
                workspace_relative(&self.root, &request_path).map_err(repair_error)?;
            let output_directory = self
                .root
                .join(OUTPUT_ROOT)
                .join(request_fingerprint.trim_start_matches("sha256:"));
            let manifest_path = output_directory.join("manifest.json");

            let manifest = if manifest_path.is_file() {
                read_native_json(&manifest_path, "repair output manifest")?
            } else {
                let arguments = [
                    "-m".to_owned(),
                    MATERIALIZER_MODULE.to_owned(),
                    "--request".to_owned(),
                    request_relative,
                    "--output-root".to_owned(),
                    OUTPUT_ROOT.to_owned(),
                ];
                let stdout = run_materializer(self, &arguments, maximum_seconds).await?;
                let emitted: NativeOutputManifest =
                    serde_json::from_slice(&stdout).map_err(|error| {
                        repair_error(format!("Nomos repair manifest output is invalid: {error}"))
                    })?;
                let persisted: NativeOutputManifest =
                    read_native_json(&manifest_path, "repair output manifest")?;
                if emitted != persisted {
                    return Err(repair_error(
                        "Nomos repair materializer output disagrees with its persisted manifest",
                    ));
                }
                persisted
            };

            let (delta_artifact, audit_references, rows) = verify_native_outputs(
                self,
                &project,
                &request,
                &request_fingerprint,
                &manifest_path,
                &manifest,
            )?;
            NativeDeltaCandidateSet::create(
                &proposal,
                &diagnosis,
                &approval,
                approval_predecessor.as_ref(),
                &application,
                self.repair_delta_identity.clone(),
                delta_artifact,
                audit_references,
                rows,
                Utc::now(),
            )
            .map_err(repair_error)
        })
    }
}

fn native_action_configuration(
    proposal: &RepairProposal,
) -> Result<(u64, u64), NativeRepairDeltaBackendError> {
    let actions = proposal
        .actions
        .iter()
        .filter(|action| action.kind == RepairActionKind::GenerateNativeRows)
        .collect::<Vec<_>>();
    if actions.len() != 1 {
        return Err(repair_error(
            "Nomos repair requires exactly one native generation action",
        ));
    }
    let action = actions[0];
    let expected_targets = proposal
        .targets
        .iter()
        .map(|target| target.key.as_str())
        .collect::<Vec<_>>();
    if action
        .target_keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != expected_targets
        || action
            .parameters
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != vec!["maximum_seconds", "recipe_version", "seed"]
    {
        return Err(repair_error(
            "Nomos native generation action does not exactly bind targets and adapter parameters",
        ));
    }
    match action.parameters.get("recipe_version") {
        Some(ParameterValue::Text(value)) if value == RECIPE_VERSION => {}
        _ => return Err(repair_error("Nomos repair recipe version is unsupported")),
    }
    let seed = positive_integer(action.parameters.get("seed"), "seed")?;
    let maximum_seconds =
        positive_integer(action.parameters.get("maximum_seconds"), "maximum_seconds")?;
    if maximum_seconds > proposal.budget.maximum_evaluation_seconds {
        return Err(repair_error(
            "Nomos repair materialization time exceeds the proposal quality budget",
        ));
    }
    Ok((seed, maximum_seconds))
}

fn positive_integer(
    value: Option<&ParameterValue>,
    field: &str,
) -> Result<u64, NativeRepairDeltaBackendError> {
    match value {
        Some(ParameterValue::Integer(value)) if *value > 0 => {
            u64::try_from(*value).map_err(repair_error)
        }
        _ => Err(repair_error(format!(
            "Nomos native generation {field} must be a positive integer"
        ))),
    }
}

fn native_request(
    project: &ExternalProjectSnapshot,
    proposal: &RepairProposal,
    seed: u64,
) -> Result<NativeRepairRequest, NativeRepairDeltaBackendError> {
    let target_counts = proposal
        .targets
        .iter()
        .map(|target| (target.key.clone(), target.absolute_row_target))
        .collect::<BTreeMap<_, _>>();
    if target_counts.keys().map(String::as_str).collect::<Vec<_>>() != SUPPORTED_TARGETS {
        return Err(repair_error(
            "Nomos repair proposal does not contain the exact supported target vocabulary",
        ));
    }
    let mut references = project
        .inputs
        .iter()
        .filter_map(|artifact| {
            let role = match artifact.role {
                EvidenceRole::Training => NativeReferenceRole::BaseTraining,
                EvidenceRole::Development => NativeReferenceRole::Development,
                EvidenceRole::SealedAcceptance => NativeReferenceRole::Sealed,
                EvidenceRole::Calibration => return None,
            };
            Some(NativeReferenceRequest {
                role,
                key: artifact.key.clone(),
                path: artifact.key.clone(),
            })
        })
        .collect::<Vec<_>>();
    references.sort_by(|left, right| {
        left.role
            .cmp(&right.role)
            .then_with(|| left.key.cmp(&right.key))
    });
    let roles = references
        .iter()
        .map(|value| value.role)
        .collect::<BTreeSet<_>>();
    if roles
        != BTreeSet::from([
            NativeReferenceRole::BaseTraining,
            NativeReferenceRole::Development,
            NativeReferenceRole::Sealed,
        ])
    {
        return Err(repair_error(
            "Nomos repair audit must cover training, development, and sealed references",
        ));
    }
    Ok(NativeRepairRequest {
        schema_version: REQUEST_SCHEMA.into(),
        project_revision: project.source_revision.clone(),
        seed,
        target_counts,
        references,
    })
}

fn write_request(
    root: &Path,
    request: &NativeRepairRequest,
    request_fingerprint: &str,
) -> Result<PathBuf, NativeRepairDeltaBackendError> {
    let digest = request_fingerprint
        .strip_prefix("sha256:")
        .ok_or_else(|| repair_error("repair request fingerprint is malformed"))?;
    let path = root
        .join(OUTPUT_ROOT)
        .join("requests")
        .join(format!("{digest}.json"));
    let payload = serde_json::to_vec_pretty(request).map_err(repair_error)?;
    write_immutable(&path, &[payload.as_slice(), b"\n"].concat())?;
    Ok(path)
}

fn write_immutable(path: &Path, payload: &[u8]) -> Result<(), NativeRepairDeltaBackendError> {
    if path.is_file() {
        if fs::read(path).map_err(repair_error)? == payload {
            return Ok(());
        }
        return Err(repair_error(
            "immutable Nomos repair request exists with different bytes",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| repair_error("repair request has no parent directory"))?;
    fs::create_dir_all(parent).map_err(repair_error)?;
    let temporary = parent.join(format!(".repair-request-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut handle = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(repair_error)?;
        handle.write_all(payload).map_err(repair_error)?;
        handle.sync_all().map_err(repair_error)?;
        fs::rename(&temporary, path).map_err(repair_error)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

async fn run_materializer(
    backend: &NomosBackend,
    arguments: &[String],
    maximum_seconds: u64,
) -> Result<Vec<u8>, NativeRepairDeltaBackendError> {
    let mut command = Command::new(&backend.python);
    command
        .args(arguments)
        .current_dir(&backend.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(maximum_seconds), command.output())
        .await
        .map_err(|_| repair_error("Nomos repair materializer exceeded its finite time limit"))?
        .map_err(|error| {
            repair_error(format!(
                "could not start Nomos repair materializer: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(repair_error(format!(
            "Nomos repair materializer failed with status {}: {}",
            output.status,
            bounded_text(&output.stderr, 2_000)
        )));
    }
    Ok(output.stdout)
}

fn verify_native_outputs(
    backend: &NomosBackend,
    project: &ExternalProjectSnapshot,
    request: &NativeRepairRequest,
    request_fingerprint: &str,
    manifest_path: &Path,
    manifest: &NativeOutputManifest,
) -> Result<
    (
        ExternalArtifactIdentity,
        Vec<NativeRepairAuditReference>,
        Vec<NativeRepairRowEvidence>,
    ),
    NativeRepairDeltaBackendError,
> {
    verify_embedded_fingerprint(manifest, &manifest.fingerprint, "repair output manifest")?;
    verify_embedded_fingerprint(
        &manifest.generation,
        &manifest.generation.fingerprint,
        "repair generation manifest",
    )?;
    let expected_rows = request.target_counts.values().sum::<u64>();
    if manifest.schema_version != OUTPUT_SCHEMA
        || manifest.request_fingerprint != request_fingerprint
        || manifest.generation.schema_version != GENERATION_MANIFEST_SCHEMA
        || manifest.generation.dataset_version != DATASET_VERSION
        || manifest.generation.matrix_version != MATRIX_VERSION
        || manifest.generation.recipe_version != RECIPE_VERSION
        || manifest.generation.validator_version != VALIDATOR_VERSION
        || manifest.generation.seed != request.seed
        || manifest.generation.project_revision != request.project_revision
        || manifest.generation.target_counts != request.target_counts
        || manifest.generation.row_count != expected_rows
        || manifest.delta_artifact.row_count != expected_rows
        || manifest.reference_count != request.references.len() as u64
    {
        return Err(repair_error(
            "Nomos repair output manifest does not match the exact request",
        ));
    }
    let expected_manifest_path = backend
        .root
        .join(OUTPUT_ROOT)
        .join(request_fingerprint.trim_start_matches("sha256:"))
        .join("manifest.json");
    if manifest_path != expected_manifest_path {
        return Err(repair_error(
            "Nomos repair output path is not content addressed",
        ));
    }
    let expected_delta_key = workspace_relative(
        &backend.root,
        &expected_manifest_path.with_file_name("repair-delta.jsonl"),
    )
    .map_err(repair_error)?;
    let expected_evidence_key = workspace_relative(
        &backend.root,
        &expected_manifest_path.with_file_name("native-quality-evidence.json"),
    )
    .map_err(repair_error)?;
    if manifest.delta_artifact.key != expected_delta_key
        || manifest.quality_evidence.key != expected_evidence_key
    {
        return Err(repair_error(
            "Nomos repair manifest points outside its content-addressed output set",
        ));
    }
    let delta_path = backend
        .resolve_existing(&manifest.delta_artifact.key)
        .map_err(repair_error)?;
    verify_native_file(
        &delta_path,
        manifest.delta_artifact.bytes,
        &manifest.delta_artifact.sha256,
        "repair delta",
    )?;
    let evidence_path = backend
        .resolve_existing(&manifest.quality_evidence.key)
        .map_err(repair_error)?;
    verify_native_file(
        &evidence_path,
        manifest.quality_evidence.bytes,
        &manifest.quality_evidence.sha256,
        "repair quality evidence",
    )?;
    let evidence: NativeQualityEvidence =
        read_native_json(&evidence_path, "repair quality evidence")?;
    verify_embedded_fingerprint(&evidence, &evidence.fingerprint, "repair quality evidence")?;
    if evidence.fingerprint != manifest.quality_evidence.fingerprint
        || evidence.schema_version != EVIDENCE_SCHEMA
        || evidence.dataset_version != DATASET_VERSION
        || evidence.recipe_version != RECIPE_VERSION
        || evidence.validator_version != VALIDATOR_VERSION
        || evidence.seed != request.seed
        || evidence.project_revision != request.project_revision
        || evidence.target_counts != request.target_counts
        || evidence.request_fingerprint != request_fingerprint
        || evidence.delta_artifact != manifest.delta_artifact
        || evidence.recipe_fingerprint != manifest.generation.recipe_fingerprint
        || evidence.generator_fingerprint != manifest.generation.generator_fingerprint
        || evidence.summary != manifest.summary
        || evidence.validation.rows != expected_rows
        || evidence.validation.target_counts != request.target_counts
        || evidence.validation.unique_decision_ids != expected_rows
        || evidence.validation.unique_exact_inputs != expected_rows
        || evidence.validation.unique_normalized_inputs != expected_rows
        || !evidence.validation.valid
        || !evidence.validation.issues.is_empty()
        || evidence.rows.len() as u64 != expected_rows
    {
        return Err(repair_error(
            "Nomos repair quality evidence does not reproduce the requested delta",
        ));
    }

    let audit_references = verify_audit_references(project, request, &evidence.references)?;
    let rows = verify_row_evidence(request, &evidence)?;
    verify_summary(&evidence.summary, &evidence.rows)?;
    let relative_delta = workspace_relative(&backend.root, &delta_path).map_err(repair_error)?;
    let delta_artifact = ExternalArtifactIdentity::new(
        relative_delta,
        EvidenceRole::Training,
        manifest.delta_artifact.bytes,
        manifest.delta_artifact.sha256.clone(),
    )
    .map_err(repair_error)?;
    Ok((delta_artifact, audit_references, rows))
}

fn verify_audit_references(
    project: &ExternalProjectSnapshot,
    request: &NativeRepairRequest,
    observed: &[NativeAuditReference],
) -> Result<Vec<NativeRepairAuditReference>, NativeRepairDeltaBackendError> {
    if observed.len() != request.references.len() {
        return Err(repair_error(
            "Nomos repair audit reference set is incomplete",
        ));
    }
    let expected = request
        .references
        .iter()
        .map(|reference| {
            let artifact = project
                .inputs
                .iter()
                .find(|artifact| artifact.key == reference.key)
                .ok_or_else(|| repair_error("repair reference is absent from the project"))?;
            if artifact.role != reference.role.evidence_role() {
                return Err(repair_error("repair reference role changed"));
            }
            Ok((
                reference.role,
                reference.key.as_str(),
                artifact.bytes,
                artifact.fingerprint.as_str(),
            ))
        })
        .collect::<Result<Vec<_>, NativeRepairDeltaBackendError>>()?;
    let mut result = Vec::with_capacity(observed.len());
    for (reference, (role, key, bytes, fingerprint)) in observed.iter().zip(expected) {
        if reference.role != role
            || reference.key != key
            || reference.bytes != bytes
            || reference.sha256 != fingerprint
            || reference.row_count == 0
        {
            return Err(repair_error(
                "Nomos repair audit references do not match the exact project population",
            ));
        }
        result.push(
            NativeRepairAuditReference::create(
                role.evidence_role(),
                reference.key.clone(),
                reference.bytes,
                reference.sha256.clone(),
                reference.row_count,
                reference.identity_set_fingerprint.clone(),
            )
            .map_err(repair_error)?,
        );
    }
    Ok(result)
}

fn verify_row_evidence(
    request: &NativeRepairRequest,
    evidence: &NativeQualityEvidence,
) -> Result<Vec<NativeRepairRowEvidence>, NativeRepairDeltaBackendError> {
    let mut target_counts = BTreeMap::<&str, u64>::new();
    let mut result = Vec::with_capacity(evidence.rows.len());
    for row in &evidence.rows {
        if row.project_revision != request.project_revision
            || row.recipe_fingerprint != evidence.recipe_fingerprint
            || row.generator_fingerprint != evidence.generator_fingerprint
            || !request.target_counts.contains_key(&row.target_key)
        {
            return Err(repair_error(
                "Nomos repair row evidence is foreign to the request or generator",
            ));
        }
        *target_counts.entry(&row.target_key).or_default() += 1;
        result.push(
            NativeRepairRowEvidence::create(
                row.row_id_hash.clone(),
                row.content_fingerprint.clone(),
                row.normalized_content_fingerprint.clone(),
                row.source_identity_fingerprint.clone(),
                row.group_identity_fingerprint.clone(),
                row.lineage_identity_fingerprint.clone(),
                row.target_key.clone(),
                row.recipe_fingerprint.clone(),
                row.generator_fingerprint.clone(),
                row.seed,
                row.project_revision.clone(),
                row.task_valid,
                row.task_validation_reasons.clone(),
                row.exact_duplicate_matches,
                row.normalized_duplicate_matches,
                row.source_contamination_matches,
                row.group_contamination_matches,
                row.lineage_contamination_matches,
            )
            .map_err(repair_error)?,
        );
    }
    let expected = request
        .target_counts
        .iter()
        .map(|(key, value)| (key.as_str(), *value))
        .collect::<BTreeMap<_, _>>();
    if target_counts != expected {
        return Err(repair_error(
            "Nomos repair row evidence does not exactly satisfy target counts",
        ));
    }
    Ok(result)
}

fn verify_summary(
    summary: &NativeQualitySummary,
    rows: &[NativeRowEvidence],
) -> Result<(), NativeRepairDeltaBackendError> {
    let recomputed = NativeQualitySummary {
        candidate_rows: rows.len() as u64,
        invalid_rows: rows.iter().filter(|row| !row.task_valid).count() as u64,
        exact_duplicate_rows: rows
            .iter()
            .filter(|row| row.exact_duplicate_matches > 0)
            .count() as u64,
        normalized_duplicate_rows: rows
            .iter()
            .filter(|row| row.normalized_duplicate_matches > 0)
            .count() as u64,
        source_contamination_rows: rows
            .iter()
            .filter(|row| row.source_contamination_matches > 0)
            .count() as u64,
        group_contamination_rows: rows
            .iter()
            .filter(|row| row.group_contamination_matches > 0)
            .count() as u64,
        lineage_contamination_rows: rows
            .iter()
            .filter(|row| row.lineage_contamination_matches > 0)
            .count() as u64,
    };
    if *summary != recomputed {
        return Err(repair_error(
            "Nomos repair quality summary does not reproduce",
        ));
    }
    Ok(())
}

fn verify_native_file(
    path: &Path,
    bytes: u64,
    fingerprint: &str,
    kind: &str,
) -> Result<(), NativeRepairDeltaBackendError> {
    let expected = fingerprint
        .strip_prefix("sha256:")
        .ok_or_else(|| repair_error(format!("Nomos {kind} fingerprint is malformed")))?;
    super::verify_file(path, bytes, expected).map_err(repair_error)
}

fn verify_embedded_fingerprint<T: Serialize>(
    value: &T,
    observed: &str,
    kind: &str,
) -> Result<(), NativeRepairDeltaBackendError> {
    let mut value = serde_json::to_value(value).map_err(repair_error)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| repair_error(format!("Nomos {kind} is not an object")))?;
    object.remove("fingerprint");
    let expected = artifact_core::fingerprint(&value).map_err(repair_error)?;
    if observed != expected {
        return Err(repair_error(format!("Nomos {kind} fingerprint changed")));
    }
    Ok(())
}

fn read_native_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    kind: &str,
) -> Result<T, NativeRepairDeltaBackendError> {
    let bytes = fs::read(path)
        .map_err(|error| repair_error(format!("could not read Nomos {kind}: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| repair_error(format!("Nomos {kind} is invalid: {error}")))
}

fn repair_error(error: impl std::fmt::Display) -> NativeRepairDeltaBackendError {
    NativeRepairDeltaBackendError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn native_row() -> NativeRowEvidence {
        NativeRowEvidence {
            row_id_hash: digest('1'),
            content_fingerprint: digest('2'),
            normalized_content_fingerprint: digest('3'),
            source_identity_fingerprint: digest('4'),
            group_identity_fingerprint: digest('5'),
            lineage_identity_fingerprint: digest('6'),
            target_key: "generic_finalize_selection".into(),
            recipe_fingerprint: digest('7'),
            generator_fingerprint: digest('8'),
            seed: 1,
            project_revision: "revision".into(),
            task_valid: true,
            task_validation_reasons: vec![],
            exact_duplicate_matches: 0,
            normalized_duplicate_matches: 0,
            source_contamination_matches: 0,
            group_contamination_matches: 0,
            lineage_contamination_matches: 0,
        }
    }

    fn native_request_fixture() -> NativeRepairRequest {
        NativeRepairRequest {
            schema_version: REQUEST_SCHEMA.into(),
            project_revision: "revision".into(),
            seed: 1,
            target_counts: BTreeMap::from([("generic_finalize_selection".into(), 1)]),
            references: vec![],
        }
    }

    fn native_evidence_fixture() -> NativeQualityEvidence {
        let row = native_row();
        NativeQualityEvidence {
            schema_version: EVIDENCE_SCHEMA.into(),
            dataset_version: DATASET_VERSION.into(),
            recipe_version: RECIPE_VERSION.into(),
            validator_version: VALIDATOR_VERSION.into(),
            seed: 1,
            project_revision: "revision".into(),
            target_counts: BTreeMap::from([("generic_finalize_selection".into(), 1)]),
            recipe_fingerprint: row.recipe_fingerprint.clone(),
            generator_fingerprint: row.generator_fingerprint.clone(),
            validation: NativeValidationSummary {
                valid: true,
                issues: vec![],
                rows: 1,
                target_counts: BTreeMap::from([("generic_finalize_selection".into(), 1)]),
                unique_decision_ids: 1,
                unique_exact_inputs: 1,
                unique_normalized_inputs: 1,
            },
            references: vec![],
            rows: vec![row],
            summary: NativeQualitySummary {
                candidate_rows: 1,
                invalid_rows: 0,
                exact_duplicate_rows: 0,
                normalized_duplicate_rows: 0,
                source_contamination_rows: 0,
                group_contamination_rows: 0,
                lineage_contamination_rows: 0,
            },
            request_fingerprint: digest('9'),
            delta_artifact: NativeArtifact {
                key: "delta".into(),
                bytes: 1,
                sha256: digest('a'),
                row_count: 1,
            },
            fingerprint: digest('b'),
        }
    }

    #[test]
    fn native_quality_summary_replays_all_failure_axes() {
        let row = NativeRowEvidence {
            task_valid: false,
            task_validation_reasons: vec!["invalid".into()],
            exact_duplicate_matches: 1,
            normalized_duplicate_matches: 1,
            source_contamination_matches: 1,
            group_contamination_matches: 1,
            lineage_contamination_matches: 1,
            ..native_row()
        };
        let expected = NativeQualitySummary {
            candidate_rows: 1,
            invalid_rows: 1,
            exact_duplicate_rows: 1,
            normalized_duplicate_rows: 1,
            source_contamination_rows: 1,
            group_contamination_rows: 1,
            lineage_contamination_rows: 1,
        };
        verify_summary(&expected, &[row]).unwrap();
    }

    #[test]
    fn embedded_fingerprint_rejects_tampering() {
        let mut value = serde_json::json!({"value": 1});
        let fingerprint = artifact_core::fingerprint(&value).unwrap();
        value["fingerprint"] = serde_json::Value::String(fingerprint.clone());
        verify_embedded_fingerprint(&value, &fingerprint, "test evidence").unwrap();
        value["value"] = serde_json::json!(2);
        assert!(verify_embedded_fingerprint(&value, &fingerprint, "test evidence").is_err());
    }

    #[test]
    fn native_row_normalization_rejects_foreign_revision_and_generator() {
        let request = native_request_fixture();
        let evidence = native_evidence_fixture();
        let rows = verify_row_evidence(&request, &evidence).unwrap();
        assert_eq!(rows.len(), 1);

        let mut foreign_revision = evidence.clone();
        foreign_revision.rows[0].project_revision = "other".into();
        assert!(verify_row_evidence(&request, &foreign_revision).is_err());

        let mut foreign_generator = evidence;
        foreign_generator.rows[0].generator_fingerprint = digest('c');
        assert!(verify_row_evidence(&request, &foreign_generator).is_err());
    }
}
