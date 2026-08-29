use std::sync::Arc;

use synthetic_data_sqlite::SqliteStore;
use tokio::sync::{RwLock, Semaphore};

#[derive(Clone)]
pub struct AppState {
    pub store: SqliteStore,
    pub api_key: Arc<RwLock<Option<String>>>,
    pub worker: Arc<Semaphore>,
}

impl AppState {
    pub fn new(store: SqliteStore) -> Self {
        Self {
            store,
            api_key: Arc::new(RwLock::new(std::env::var("SYNTH_OPENAI_API_KEY").ok())),
            worker: Arc::new(Semaphore::new(1)),
        }
    }
}
