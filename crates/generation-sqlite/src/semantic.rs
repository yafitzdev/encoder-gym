use semantic_catalog::{
    BoxFuture, GenerationSemanticAssignment, SemanticBindingDecision, SemanticCatalogStore,
    SemanticProfile, SemanticScope, SemanticStoreError,
};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::SqliteStore;

impl SemanticCatalogStore for SqliteStore {
    fn create_semantic_profile(
        &self,
        profile: &SemanticProfile,
    ) -> BoxFuture<'_, Result<(), SemanticStoreError>> {
        let profile = profile.clone();
        Box::pin(async move {
            let (scope_kind, scope_dataset_id) = match profile.scope {
                SemanticScope::Reusable => ("reusable", None),
                SemanticScope::Dataset { dataset_id } => ("dataset", Some(dataset_id)),
            };
            sqlx::query(
                "INSERT INTO semantic_profiles \
                 (id, profile_key, version, predecessor_id, scope_kind, scope_dataset_id, \
                  target_key, artifact_json, fingerprint, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(profile.id)
            .bind(&profile.key)
            .bind(i64::from(profile.version))
            .bind(profile.predecessor_id)
            .bind(scope_kind)
            .bind(scope_dataset_id)
            .bind(profile.target.key())
            .bind(to_json(&profile)?)
            .bind(&profile.fingerprint)
            .bind(profile.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_semantic_profile(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SemanticProfile>, SemanticStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ArtifactRecord>(
                "SELECT artifact_json FROM semantic_profiles WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(ArtifactRecord::parse)
            .transpose()
        })
    }

    fn list_semantic_profiles(
        &self,
        key: Option<&str>,
    ) -> BoxFuture<'_, Result<Vec<SemanticProfile>, SemanticStoreError>> {
        let key = key.map(str::to_owned);
        Box::pin(async move {
            let mut query =
                QueryBuilder::<Sqlite>::new("SELECT artifact_json FROM semantic_profiles");
            if let Some(key) = key {
                query.push(" WHERE profile_key = ").push_bind(key);
            }
            query.push(" ORDER BY profile_key, version DESC, id");
            query
                .build_query_as::<ArtifactRecord>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ArtifactRecord::parse)
                .collect()
        })
    }

    fn append_semantic_binding(
        &self,
        decision: &SemanticBindingDecision,
    ) -> BoxFuture<'_, Result<(), SemanticStoreError>> {
        let decision = decision.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO dataset_semantic_binding_decisions \
                 (id, dataset_id, target_key, layer, profile_id, profile_fingerprint, \
                  predecessor_id, artifact_json, fingerprint, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(decision.id)
            .bind(decision.dataset_id)
            .bind(decision.target.key())
            .bind(match decision.layer {
                semantic_catalog::SemanticLayer::Reusable => "reusable",
                semantic_catalog::SemanticLayer::DatasetOverride => "dataset_override",
            })
            .bind(decision.profile_id)
            .bind(&decision.profile_fingerprint)
            .bind(decision.predecessor_id)
            .bind(to_json(&decision)?)
            .bind(&decision.fingerprint)
            .bind(decision.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn current_semantic_bindings(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SemanticBindingDecision>, SemanticStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ArtifactRecord>(
                "SELECT current.artifact_json \
                 FROM dataset_semantic_binding_decisions current \
                 WHERE current.dataset_id = ? AND NOT EXISTS ( \
                     SELECT 1 FROM dataset_semantic_binding_decisions later \
                     WHERE later.dataset_id = current.dataset_id \
                       AND later.target_key = current.target_key \
                       AND later.layer = current.layer \
                       AND later.sequence > current.sequence \
                 ) ORDER BY current.target_key, current.layer",
            )
            .bind(dataset_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?
            .into_iter()
            .map(ArtifactRecord::parse)
            .collect()
        })
    }

    fn save_generation_semantics(
        &self,
        assignment: &GenerationSemanticAssignment,
    ) -> BoxFuture<'_, Result<(), SemanticStoreError>> {
        let assignment = assignment.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO generation_job_semantics \
                 (job_id, context_fingerprint, artifact_json, fingerprint, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(assignment.job_id)
            .bind(&assignment.context.fingerprint)
            .bind(to_json(&assignment)?)
            .bind(&assignment.fingerprint)
            .bind(assignment.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_generation_semantics(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationSemanticAssignment>, SemanticStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ArtifactRecord>(
                "SELECT artifact_json FROM generation_job_semantics WHERE job_id = ?",
            )
            .bind(job_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(ArtifactRecord::parse)
            .transpose()
        })
    }
}

#[derive(Debug, FromRow)]
struct ArtifactRecord {
    artifact_json: String,
}

impl ArtifactRecord {
    fn parse<T: serde::de::DeserializeOwned>(self) -> Result<T, SemanticStoreError> {
        serde_json::from_str(&self.artifact_json).map_err(store_error)
    }
}

fn to_json(value: &impl serde::Serialize) -> Result<String, SemanticStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> SemanticStoreError {
    SemanticStoreError(error.to_string())
}
