use encoder_campaign_core::{
    BoxFuture, CampaignEvent, CampaignStore, CampaignStoreError, ProductionCampaign,
    replay_campaign,
};
use encoder_experiment_core::ports::ExperimentStore;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::SqliteExperimentStore;

impl CampaignStore for SqliteExperimentStore {
    fn create_campaign(
        &self,
        campaign: &ProductionCampaign,
        first_event: &CampaignEvent,
    ) -> BoxFuture<'_, Result<(), CampaignStoreError>> {
        let campaign = campaign.clone();
        let first_event = first_event.clone();
        Box::pin(async move {
            campaign.validate_integrity().map_err(store_error)?;
            replay_campaign(&campaign, std::slice::from_ref(&first_event)).map_err(store_error)?;
            let project = self
                .get_project(campaign.project_snapshot_id)
                .await
                .map_err(store_error)?
                .ok_or_else(|| CampaignStoreError("experiment project does not exist".into()))?;
            if project.fingerprint != campaign.project_snapshot_fingerprint {
                return Err(CampaignStoreError(
                    "campaign project fingerprint does not match persistence".into(),
                ));
            }
            let campaign_json = serde_json::to_string(&campaign).map_err(store_error)?;
            let event_json = serde_json::to_string(&first_event).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_production_campaigns \
                 (id, project_snapshot_id, fingerprint, artifact_json, last_sequence, \
                  last_event_fingerprint, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, 1, ?, ?, ?)",
            )
            .bind(campaign.id)
            .bind(campaign.project_snapshot_id)
            .bind(&campaign.fingerprint)
            .bind(campaign_json)
            .bind(&first_event.fingerprint)
            .bind(campaign.created_at)
            .bind(first_event.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            insert_event(&mut transaction, &first_event, &event_json).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_campaign(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProductionCampaign>, CampaignStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_production_campaigns WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            artifact
                .map(|value| {
                    let campaign: ProductionCampaign =
                        serde_json::from_str(&value).map_err(store_error)?;
                    campaign.validate_integrity().map_err(store_error)?;
                    Ok(campaign)
                })
                .transpose()
        })
    }

    fn append_campaign_event(
        &self,
        event: &CampaignEvent,
    ) -> BoxFuture<'_, Result<(), CampaignStoreError>> {
        let event = event.clone();
        Box::pin(async move {
            event.validate_integrity().map_err(store_error)?;
            let campaign = self
                .get_campaign(event.campaign_id)
                .await?
                .ok_or_else(|| CampaignStoreError("production campaign does not exist".into()))?;
            let mut events = self.list_campaign_events(event.campaign_id).await?;
            events.push(event.clone());
            replay_campaign(&campaign, &events).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            append_in_transaction(&mut transaction, &event).await?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn list_campaign_events(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CampaignEvent>, CampaignStoreError>> {
        Box::pin(async move {
            let artifacts: Vec<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_production_campaign_events \
                 WHERE campaign_id = ? ORDER BY sequence",
            )
            .bind(campaign_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            artifacts
                .into_iter()
                .map(|artifact| {
                    let event: CampaignEvent =
                        serde_json::from_str(&artifact).map_err(store_error)?;
                    event.validate_integrity().map_err(store_error)?;
                    Ok(event)
                })
                .collect()
        })
    }
}

async fn append_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CampaignEvent,
) -> Result<(), CampaignStoreError> {
    let previous = event.previous_event_fingerprint.as_deref().ok_or_else(|| {
        CampaignStoreError("appended campaign event requires a predecessor".into())
    })?;
    let head = sqlx::query(
        "SELECT last_sequence, last_event_fingerprint \
         FROM encoder_production_campaigns WHERE id = ?",
    )
    .bind(event.campaign_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(store_error)?
    .ok_or_else(|| CampaignStoreError("production campaign does not exist".into()))?;
    let last_sequence: i64 = head.try_get("last_sequence").map_err(store_error)?;
    let last_fingerprint: String = head
        .try_get("last_event_fingerprint")
        .map_err(store_error)?;
    if last_sequence + 1 != i64::from(event.sequence) || last_fingerprint != previous {
        return Err(CampaignStoreError(
            "campaign append conflicted with the durable journal head".into(),
        ));
    }
    let event_json = serde_json::to_string(event).map_err(store_error)?;
    insert_event(transaction, event, &event_json).await?;
    let updated = sqlx::query(
        "UPDATE encoder_production_campaigns SET last_sequence = ?, \
         last_event_fingerprint = ?, updated_at = ? \
         WHERE id = ? AND last_sequence = ? AND last_event_fingerprint = ?",
    )
    .bind(i64::from(event.sequence))
    .bind(&event.fingerprint)
    .bind(event.created_at)
    .bind(event.campaign_id)
    .bind(last_sequence)
    .bind(previous)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    if updated.rows_affected() != 1 {
        return Err(CampaignStoreError(
            "campaign append lost an optimistic concurrency race".into(),
        ));
    }
    Ok(())
}

async fn insert_event(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CampaignEvent,
    artifact_json: &str,
) -> Result<(), CampaignStoreError> {
    sqlx::query(
        "INSERT INTO encoder_production_campaign_events \
         (id, campaign_id, sequence, previous_event_fingerprint, fingerprint, \
          artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(event.id)
    .bind(event.campaign_id)
    .bind(i64::from(event.sequence))
    .bind(&event.previous_event_fingerprint)
    .bind(&event.fingerprint)
    .bind(artifact_json)
    .bind(event.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn store_error(error: impl std::fmt::Display) -> CampaignStoreError {
    CampaignStoreError(error.to_string())
}
