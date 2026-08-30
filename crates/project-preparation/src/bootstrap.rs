use std::collections::BTreeMap;

use chrono::Utc;
use dataset_core::domain::{ImportRowStatus, ImportState, SnapshotSplit};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    BootstrapBundle, BootstrapManifest, BootstrapPreview, BootstrapSourceBundle,
    BootstrapSourcePreview, BootstrapSourceSummary, CohortEvidence, PreparationEvidence,
    PreparationIssue, ProjectBootstrap, compile_project, domain::bootstrap_fingerprint,
    preview_project,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BootstrapError {
    #[error("invalid project bootstrap: {0}")]
    Invalid(String),
    #[error("project bootstrap is blocked: {0}")]
    Blocked(String),
    #[error("could not construct project bootstrap: {0}")]
    Domain(String),
}

pub fn preview_bootstrap(
    manifest: &BootstrapManifest,
    sources: &[BootstrapSourceBundle],
) -> Result<BootstrapPreview, BootstrapError> {
    let context = resolve_context(manifest, sources)?;
    let mut preparation = preview_project(&context.manifest, &context.evidence)
        .map_err(|error| BootstrapError::Domain(error.to_string()))?;
    preparation.manifest_fingerprint = context.bootstrap_fingerprint.clone();
    let mut issues = source_issues(sources);
    issues.extend(preparation.issues.clone());
    let eligible = issues.is_empty() && preparation.eligible;
    Ok(BootstrapPreview {
        bootstrap_fingerprint: context.bootstrap_fingerprint,
        sources: source_previews(manifest, sources)?,
        preparation,
        eligible,
        issues,
    })
}

pub fn compile_bootstrap(
    manifest: &BootstrapManifest,
    sources: Vec<BootstrapSourceBundle>,
) -> Result<BootstrapBundle, BootstrapError> {
    let context = resolve_context(manifest, &sources)?;
    let issues = source_issues(&sources);
    if !issues.is_empty() {
        return Err(BootstrapError::Blocked(
            issues
                .iter()
                .map(|issue| issue.message.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let preparation = compile_project(&context.manifest, &context.evidence)
        .map_err(|error| BootstrapError::Domain(error.to_string()))?;
    let now = Utc::now();
    let mut bootstrap = ProjectBootstrap {
        id: Uuid::new_v4(),
        name: manifest.name.trim().to_owned(),
        bootstrap_fingerprint: context.bootstrap_fingerprint,
        preparation_id: preparation.preparation.id,
        sources: sources
            .iter()
            .map(|source| BootstrapSourceSummary {
                key: source.key.clone(),
                content_fingerprint: source.content_fingerprint.clone(),
                dataset_id: source.dataset.id,
                import_id: source.dataset_import.id,
                snapshot_id: source.snapshot.id,
                accepted_rows: source.dataset_import.accepted_rows,
            })
            .collect(),
        created_at: now,
        fingerprint: String::new(),
    };
    bootstrap.fingerprint = bootstrap_fingerprint(&bootstrap)
        .map_err(|error| BootstrapError::Domain(error.to_string()))?;
    Ok(BootstrapBundle {
        sources,
        preparation,
        bootstrap,
    })
}

struct ResolvedContext {
    manifest: crate::PreparationManifest,
    evidence: PreparationEvidence,
    bootstrap_fingerprint: String,
}

fn resolve_context(
    manifest: &BootstrapManifest,
    sources: &[BootstrapSourceBundle],
) -> Result<ResolvedContext, BootstrapError> {
    validate_source_identity(manifest, sources)?;
    let source_fingerprints = sources
        .iter()
        .map(|source| (source.key.clone(), source.content_fingerprint.clone()))
        .collect::<BTreeMap<_, _>>();
    let snapshot_ids = sources
        .iter()
        .map(|source| (source.key.clone(), source.snapshot.id))
        .collect::<BTreeMap<_, _>>();
    let resolved = manifest
        .resolve(&snapshot_ids)
        .map_err(|error| BootstrapError::Invalid(error.to_string()))?;
    let evidence = PreparationEvidence {
        cohorts: sources
            .iter()
            .map(|source| {
                (
                    source.snapshot.id,
                    CohortEvidence {
                        snapshot: source.snapshot.clone(),
                        source_dataset: source.dataset.clone(),
                        members: source.members.clone(),
                    },
                )
            })
            .collect(),
    };
    let bootstrap_fingerprint = manifest
        .fingerprint(&source_fingerprints)
        .map_err(|error| BootstrapError::Invalid(error.to_string()))?;
    Ok(ResolvedContext {
        manifest: resolved,
        evidence,
        bootstrap_fingerprint,
    })
}

fn validate_source_identity(
    manifest: &BootstrapManifest,
    sources: &[BootstrapSourceBundle],
) -> Result<(), BootstrapError> {
    let expected = manifest.source_keys();
    let actual = sources
        .iter()
        .map(|source| source.key.clone())
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(BootstrapError::Invalid(format!(
            "source order or keys differ; expected {expected:?}, got {actual:?}"
        )));
    }
    for source in sources {
        if source.content_fingerprint.trim().is_empty()
            || source.dataset_import.dataset_id != source.dataset.id
            || source.snapshot.source_dataset_id != source.dataset.id
            || source.dataset_import.state != ImportState::Completed
            || source.dataset_import.processed_rows != source.imported_rows.len() as u64
            || source.dataset_import.accepted_rows
                != source
                    .imported_rows
                    .iter()
                    .filter(|row| row.status == ImportRowStatus::Accepted)
                    .count() as u64
            || source.dataset_import.rejected_rows
                != source
                    .imported_rows
                    .iter()
                    .filter(|row| row.status == ImportRowStatus::Rejected)
                    .count() as u64
            || source.snapshot.member_count != source.members.len() as u64
            || source
                .members
                .iter()
                .any(|member| member.snapshot_id != source.snapshot.id)
        {
            return Err(BootstrapError::Invalid(format!(
                "source artifact references or counts are inconsistent: {}",
                source.key
            )));
        }
    }
    Ok(())
}

fn source_issues(sources: &[BootstrapSourceBundle]) -> Vec<PreparationIssue> {
    let mut issues = Vec::new();
    for source in sources {
        if source.dataset_import.processed_rows == 0 {
            issues.push(issue(
                "empty_bootstrap_source",
                format!("bootstrap source {} contains no rows", source.key),
            ));
        }
        if source.dataset_import.rejected_rows > 0 {
            issues.push(issue(
                "rejected_bootstrap_rows",
                format!(
                    "bootstrap source {} rejected {} of {} rows",
                    source.key,
                    source.dataset_import.rejected_rows,
                    source.dataset_import.processed_rows
                ),
            ));
        }
        if source
            .members
            .iter()
            .any(|member| member.split != SnapshotSplit::Test)
        {
            issues.push(issue(
                "bootstrap_snapshot_not_all_test",
                format!(
                    "bootstrap source {} must materialize an all-test snapshot",
                    source.key
                ),
            ));
        }
    }
    issues
}

fn source_previews(
    manifest: &BootstrapManifest,
    sources: &[BootstrapSourceBundle],
) -> Result<Vec<BootstrapSourcePreview>, BootstrapError> {
    let declarations = manifest
        .development
        .cohorts
        .iter()
        .chain(
            manifest
                .sealed
                .iter()
                .flat_map(|suite| suite.cohorts.iter()),
        )
        .collect::<Vec<_>>();
    if declarations.len() != sources.len() {
        return Err(BootstrapError::Invalid(
            "source declarations differ from resolved artifacts".into(),
        ));
    }
    Ok(declarations
        .into_iter()
        .zip(sources)
        .map(|(cohort, source)| BootstrapSourcePreview {
            key: source.key.clone(),
            cohort_name: cohort.name.clone(),
            declared_path: cohort.source.path.to_string_lossy().into_owned(),
            content_fingerprint: source.content_fingerprint.clone(),
            processed_rows: source.dataset_import.processed_rows,
            accepted_rows: source.dataset_import.accepted_rows,
            rejected_rows: source.dataset_import.rejected_rows,
        })
        .collect())
}

fn issue(code: impl Into<String>, message: impl Into<String>) -> PreparationIssue {
    PreparationIssue {
        code: code.into(),
        message: message.into(),
    }
}
