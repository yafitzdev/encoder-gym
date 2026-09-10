//! Custody of completed models, independent of scientific acceptance.
use super::*;

#[derive(Debug, Clone)]
pub struct CompletedModelRegistration {
    pub parent_model_id: Uuid,
    pub name: String,
    pub source_model: BoundIdentity,
    pub source_model_format: String,
    pub source_model_bytes: u64,
    pub producing_run: BoundIdentity,
    pub training_snapshot: BoundIdentity,
    pub trainer: BoundIdentity,
    pub effective_configuration_fingerprint: String,
    pub source_revision: String,
}

/// The application verifies the native output and its scientific run before
/// calling this custody boundary. Evaluation success is deliberately irrelevant.
pub async fn register_completed_model(
    folder: &Path,
    source: &Path,
    request: CompletedModelRegistration,
) -> Result<ManagedWorkspace> {
    let workspace = open_workspace(folder, false).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model history is not initialized.")?;
    let source_model = inspect_model(source)?;
    ensure!(
        source_model.format == request.source_model_format
            && source_model.bytes == request.source_model_bytes,
        "The completed model's format or size does not match its recorded output."
    );
    let relative = format!(
        "models/candidates/{}",
        source_model
            .fingerprint
            .strip_prefix("sha256:")
            .context("Missing model checksum.")?
    );
    let mut artifact = ModelArtifact::trained(
        workspace.manifest.id,
        Uuid::new_v4(),
        request.name,
        relative.clone(),
        &source_model,
        request.parent_model_id,
        request.producing_run,
        request.source_model,
        request.training_snapshot,
        request.trainer,
        request.effective_configuration_fingerprint,
        request.source_revision,
        Utc::now(),
    )?;
    let root = Path::new(&workspace.folder);
    let mut database = connect(root, false, false).await?;
    let mut transaction = database.begin_with("BEGIN IMMEDIATE").await?;
    let records = sqlx::query_scalar::<_, String>("SELECT metadata_json FROM model_artifacts")
        .fetch_all(&mut *transaction)
        .await?;
    for json in records {
        let existing: ModelArtifact = serde_json::from_str(&json)?;
        if existing.source_model == artifact.source_model
            && existing.producing_run.as_ref().map(|v| &v.id)
                == artifact.producing_run.as_ref().map(|v| &v.id)
        {
            artifact.id = existing.id;
            artifact.created_at = existing.created_at;
            artifact.name.clone_from(&existing.name);
            ensure!(
                artifact == existing,
                "The completed model's recorded provenance changed."
            );
            transaction.rollback().await?;
            database.close().await?;
            return open_workspace(root, true).await;
        }
    }
    catalog.with_artifact(artifact.clone())?;
    publish_model_copy(&source_model, root, &relative)?;
    sqlx::query("INSERT INTO model_artifacts (id, project_id, content_fingerprint, origin, metadata_json) VALUES (?, ?, ?, ?, ?)")
        .bind(artifact.id.to_string()).bind(artifact.project_id.to_string())
        .bind(&artifact.fingerprint).bind(model_origin(artifact.origin))
        .bind(serde_json::to_string(&artifact)?).execute(&mut *transaction).await?;
    transaction.commit().await?;
    database.close().await?;
    open_workspace(root, true).await
}
