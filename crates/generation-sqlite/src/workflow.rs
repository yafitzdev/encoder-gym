use std::collections::BTreeMap;

use generation_core::domain::GenerationPlan;
use sqlx::{FromRow, QueryBuilder, Sqlite};
use uuid::Uuid;
use workflow_core::{
    allocation::{InitialAllocationFeasibility, InitialAllocationRecord, InitialAllocationResult},
    ports::{BoxFuture, InitialAllocationQuery, InitialAllocationStore, WorkflowStoreError},
};

use crate::SqliteStore;

impl InitialAllocationStore for SqliteStore {
    fn create_initial_allocation(
        &self,
        allocation: &InitialAllocationRecord,
        plan: &GenerationPlan,
    ) -> BoxFuture<'_, Result<(), WorkflowStoreError>> {
        let allocation = allocation.clone();
        let plan = plan.clone();
        Box::pin(async move {
            validate_allocation_plan(&allocation, &plan)?;
            let artifact_json = serde_json::to_string(&allocation).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            crate::insert_plan(&mut transaction, &plan)
                .await
                .map_err(store_error)?;
            sqlx::query(
                "INSERT INTO workflow_initial_allocations \
                 (id, dataset_id, generation_plan_id, requested_total_rows, \
                  initial_target_rows, reserved_rows, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(allocation.id)
            .bind(allocation.result.dataset_id)
            .bind(allocation.generation_plan_id)
            .bind(allocation.result.requested_total_rows)
            .bind(allocation.result.initial_target_rows)
            .bind(allocation.result.reserved_rows)
            .bind(&allocation.fingerprint)
            .bind(artifact_json)
            .bind(allocation.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_initial_allocation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<InitialAllocationRecord>, WorkflowStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, InitialAllocationRow>(
                "SELECT id, dataset_id, generation_plan_id, requested_total_rows, \
                 initial_target_rows, reserved_rows, fingerprint, artifact_json, created_at \
                 FROM workflow_initial_allocations WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?
            .map(InitialAllocationRow::into_domain)
            .transpose()
        })
    }

    fn query_initial_allocations(
        &self,
        query: InitialAllocationQuery,
    ) -> BoxFuture<'_, Result<Vec<InitialAllocationRecord>, WorkflowStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, dataset_id, generation_plan_id, requested_total_rows, \
                 initial_target_rows, reserved_rows, fingerprint, artifact_json, created_at \
                 FROM workflow_initial_allocations",
            );
            if let Some(dataset_id) = query.dataset_id {
                builder.push(" WHERE dataset_id = ").push_bind(dataset_id);
            }
            builder
                .push(" ORDER BY created_at DESC, id ASC LIMIT ")
                .push_bind(query.limit)
                .push(" OFFSET ")
                .push_bind(query.offset);
            builder
                .build_query_as::<InitialAllocationRow>()
                .fetch_all(self.pool())
                .await
                .map_err(store_error)?
                .into_iter()
                .map(InitialAllocationRow::into_domain)
                .collect()
        })
    }
}

fn validate_allocation_plan(
    allocation: &InitialAllocationRecord,
    plan: &GenerationPlan,
) -> Result<(), WorkflowStoreError> {
    if allocation.result.feasibility != InitialAllocationFeasibility::Feasible {
        return Err(WorkflowStoreError("allocation is infeasible".into()));
    }
    if allocation
        .result
        .reproduce_fingerprint()
        .map_err(store_error)?
        != allocation.result.fingerprint
        || allocation.reproduce_fingerprint().map_err(store_error)? != allocation.fingerprint
    {
        return Err(WorkflowStoreError(
            "allocation fingerprint does not reproduce".into(),
        ));
    }
    if allocation.generation_plan_id != plan.id
        || allocation.result.dataset_id != plan.dataset_id
        || plan.total_target_count() != u64::from(allocation.result.initial_target_rows)
    {
        return Err(WorkflowStoreError(
            "allocation and generation plan identities or totals differ".into(),
        ));
    }
    let expected = allocation
        .result
        .cells
        .iter()
        .map(|cell| (cell.cell.key(), cell.target))
        .collect::<BTreeMap<_, _>>();
    let actual = plan
        .cells
        .iter()
        .map(|cell| (cell.cell.key(), cell.target_count))
        .collect::<BTreeMap<_, _>>();
    if expected != actual {
        return Err(WorkflowStoreError(
            "allocation cells differ from generation plan cells".into(),
        ));
    }
    Ok(())
}

#[derive(FromRow)]
struct InitialAllocationRow {
    id: Uuid,
    dataset_id: Uuid,
    generation_plan_id: Uuid,
    requested_total_rows: u32,
    initial_target_rows: u32,
    reserved_rows: u32,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl InitialAllocationRow {
    fn into_domain(self) -> Result<InitialAllocationRecord, WorkflowStoreError> {
        let allocation: InitialAllocationRecord =
            serde_json::from_str(&self.artifact_json).map_err(store_error)?;
        validate_normalized_fields(&self, &allocation)?;
        if allocation
            .result
            .reproduce_fingerprint()
            .map_err(store_error)?
            != allocation.result.fingerprint
            || allocation.reproduce_fingerprint().map_err(store_error)? != allocation.fingerprint
        {
            return Err(WorkflowStoreError(
                "persisted allocation fingerprint does not reproduce".into(),
            ));
        }
        Ok(allocation)
    }
}

fn validate_normalized_fields(
    row: &InitialAllocationRow,
    allocation: &InitialAllocationRecord,
) -> Result<(), WorkflowStoreError> {
    let result: &InitialAllocationResult = &allocation.result;
    if row.id != allocation.id
        || row.dataset_id != result.dataset_id
        || row.generation_plan_id != allocation.generation_plan_id
        || row.requested_total_rows != result.requested_total_rows
        || row.initial_target_rows != result.initial_target_rows
        || row.reserved_rows != result.reserved_rows
        || row.fingerprint != allocation.fingerprint
        || row.created_at != allocation.created_at
    {
        return Err(WorkflowStoreError(
            "normalized allocation columns differ from immutable artifact".into(),
        ));
    }
    Ok(())
}

fn store_error(error: impl std::fmt::Display) -> WorkflowStoreError {
    WorkflowStoreError(error.to_string())
}
