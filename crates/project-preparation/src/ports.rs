use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::{PreparationBundle, PreparationStoreError, PreparedProject};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait PreparationStore: Send + Sync {
    fn create_preparation(
        &self,
        bundle: &PreparationBundle,
    ) -> BoxFuture<'_, Result<PreparedProject, PreparationStoreError>>;

    fn get_preparation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<PreparedProject>, PreparationStoreError>>;

    fn get_preparation_by_manifest(
        &self,
        manifest_fingerprint: &str,
    ) -> BoxFuture<'_, Result<Option<PreparedProject>, PreparationStoreError>>;

    fn list_preparations(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<PreparedProject>, PreparationStoreError>>;
}
