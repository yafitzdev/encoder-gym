use std::{future::Future, pin::Pin};

use thiserror::Error;
use uuid::Uuid;

use crate::domain::{DatasetImport, DatasetSnapshot, ImportedRow, SnapshotMember, SourceRow};

#[derive(Debug, Clone, Copy)]
pub struct SnapshotQuery {
    pub dataset_id: Option<Uuid>,
    pub limit: u32,
    pub offset: u32,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("dataset persistence operation failed: {0}")]
pub struct DatasetStoreError(pub String);

pub trait AcceptedRowSource: Send + Sync {
    fn list_accepted_source_rows(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SourceRow>, DatasetStoreError>>;
    fn query_accepted_source_rows(
        &self,
        dataset_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<SourceRow>, DatasetStoreError>>;
}

pub trait ImportStore: Send + Sync {
    fn create_import(
        &self,
        dataset_import: &DatasetImport,
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>>;

    fn save_import(
        &self,
        dataset_import: &DatasetImport,
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>>;

    fn get_import(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetImport>, DatasetStoreError>>;

    fn list_imports(
        &self,
        dataset_id: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<DatasetImport>, DatasetStoreError>>;

    fn insert_imported_rows(
        &self,
        dataset_import: &DatasetImport,
        rows: &[ImportedRow],
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>>;

    fn list_imported_rows(
        &self,
        import_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<ImportedRow>, DatasetStoreError>>;

    fn accepted_normalized_source_texts(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<String>, DatasetStoreError>>;
}

pub trait SnapshotStore: Send + Sync {
    fn create_snapshot(
        &self,
        snapshot: &DatasetSnapshot,
        members: &[SnapshotMember],
    ) -> BoxFuture<'_, Result<(), DatasetStoreError>>;

    fn get_snapshot(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetSnapshot>, DatasetStoreError>>;

    fn list_snapshots(&self) -> BoxFuture<'_, Result<Vec<DatasetSnapshot>, DatasetStoreError>>;

    fn query_snapshots(
        &self,
        query: SnapshotQuery,
    ) -> BoxFuture<'_, Result<Vec<DatasetSnapshot>, DatasetStoreError>>;

    fn list_snapshot_members(
        &self,
        snapshot_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SnapshotMember>, DatasetStoreError>>;

    fn query_snapshot_members(
        &self,
        snapshot_id: Uuid,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<SnapshotMember>, DatasetStoreError>>;
}
