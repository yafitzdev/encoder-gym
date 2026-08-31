use chrono::{DateTime, Utc};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use training_core::{
    domain::{
        EncoderArchitecture, RegisteredEncoder, TrainingCheckpoint, TrainingExample,
        TrainingInputBinding, TrainingRequest, TrainingRun, TrainingRunState,
    },
    ports::{
        BoxFuture, EncoderRegistry, EncoderRegistryError, TrainingRunQuery, TrainingStore,
        TrainingStoreError,
    },
};
use uuid::Uuid;

use super::SqliteStore;

impl EncoderRegistry for SqliteStore {
    fn register_encoder(
        &self,
        encoder: &RegisteredEncoder,
    ) -> BoxFuture<'_, Result<(), EncoderRegistryError>> {
        let encoder = encoder.clone();
        Box::pin(async move {
            encoder
                .validate()
                .map_err(|error| EncoderRegistryError(error.to_string()))?;
            sqlx::query(
                "INSERT INTO registered_encoders \
                 (id, name, architecture, source_path, fingerprint, configuration_artifact_json, \
                  tokenizer_artifact_json, weights_artifact_json, metadata_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(encoder.id)
            .bind(encoder.name)
            .bind(architecture_text(encoder.architecture))
            .bind(encoder.source_path)
            .bind(encoder.fingerprint)
            .bind(encoder_json(&encoder.configuration)?)
            .bind(encoder_json(&encoder.tokenizer)?)
            .bind(encoder_json(&encoder.weights)?)
            .bind(encoder_json(&encoder.metadata)?)
            .bind(encoder.created_at)
            .execute(&self.pool)
            .await
            .map_err(encoder_store_error)?;
            Ok(())
        })
    }

    fn get_encoder(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RegisteredEncoder>, EncoderRegistryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, EncoderRecord>(
                "SELECT id, name, architecture, source_path, fingerprint, \
                 configuration_artifact_json, tokenizer_artifact_json, weights_artifact_json, \
                 metadata_json, created_at FROM registered_encoders WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(encoder_store_error)?
            .map(EncoderRecord::into_domain)
            .transpose()
        })
    }

    fn list_encoders(&self) -> BoxFuture<'_, Result<Vec<RegisteredEncoder>, EncoderRegistryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, EncoderRecord>(
                "SELECT id, name, architecture, source_path, fingerprint, \
                 configuration_artifact_json, tokenizer_artifact_json, weights_artifact_json, \
                 metadata_json, created_at FROM registered_encoders ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(encoder_store_error)?
            .into_iter()
            .map(EncoderRecord::into_domain)
            .collect()
        })
    }
}

#[derive(Debug, FromRow)]
struct EncoderRecord {
    id: Uuid,
    name: String,
    architecture: String,
    source_path: String,
    fingerprint: String,
    configuration_artifact_json: String,
    tokenizer_artifact_json: String,
    weights_artifact_json: String,
    metadata_json: String,
    created_at: DateTime<Utc>,
}

impl EncoderRecord {
    fn into_domain(self) -> Result<RegisteredEncoder, EncoderRegistryError> {
        Ok(RegisteredEncoder {
            id: self.id,
            name: self.name,
            architecture: parse_architecture(&self.architecture)?,
            source_path: self.source_path,
            fingerprint: self.fingerprint,
            configuration: encoder_from_json(&self.configuration_artifact_json)?,
            tokenizer: encoder_from_json(&self.tokenizer_artifact_json)?,
            weights: encoder_from_json(&self.weights_artifact_json)?,
            metadata: encoder_from_json(&self.metadata_json)?,
            created_at: self.created_at,
        })
    }
}

impl TrainingStore for SqliteStore {
    fn create_training_run(
        &self,
        run: &TrainingRun,
    ) -> BoxFuture<'_, Result<(), TrainingStoreError>> {
        let run = run.clone();
        Box::pin(async move {
            run.validate_new().map_err(store_error)?;
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            validate_training_input_authority(&mut transaction, &run, true).await?;
            sqlx::query(
                "INSERT INTO training_runs \
                 (id, snapshot_id, backend_name, model_format, state, configuration_json, \
                  base_model_id, parent_checkpoint_id, backend_configuration_fingerprint, \
                  transformer_configuration_json, input_protocol, input_population_fingerprint, \
                  input_member_count, input_fingerprint, input_authority_kind, input_authority_id, \
                  input_authority_fingerprint, \
                  completed_epochs, current_epoch, completed_batches, batches_in_epoch, \
                  processed_examples, latest_training_loss, latest_validation_loss, \
                  latest_learning_rate, elapsed_milliseconds, cancel_requested, error_message, \
                  created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(run.id)
            .bind(run.snapshot_id)
            .bind(run.backend_name)
            .bind(run.model_format)
            .bind(run_state_text(run.state))
            .bind(to_json(&run.configuration)?)
            .bind(run.base_model_id)
            .bind(run.parent_checkpoint_id)
            .bind(run.backend_configuration_fingerprint)
            .bind(
                run.transformer_configuration
                    .as_ref()
                    .map(to_json)
                    .transpose()?,
            )
            .bind(run.input_binding.as_ref().map(|value| &value.protocol))
            .bind(
                run.input_binding
                    .as_ref()
                    .map(|value| &value.population_fingerprint),
            )
            .bind(
                run.input_binding
                    .as_ref()
                    .map(|value| i64::try_from(value.member_count))
                    .transpose()
                    .map_err(store_error)?,
            )
            .bind(
                run.input_binding
                    .as_ref()
                    .map(|value| &value.input_fingerprint),
            )
            .bind(
                run.input_binding
                    .as_ref()
                    .map(|value| &value.authority_kind),
            )
            .bind(run.input_binding.as_ref().map(|value| value.authority_id))
            .bind(
                run.input_binding
                    .as_ref()
                    .map(|value| &value.authority_fingerprint),
            )
            .bind(i64::from(run.completed_epochs))
            .bind(i64::from(run.current_epoch))
            .bind(i64::from(run.completed_batches))
            .bind(i64::from(run.batches_in_epoch))
            .bind(i64::try_from(run.processed_examples).map_err(store_error)?)
            .bind(run.latest_training_loss)
            .bind(run.latest_validation_loss)
            .bind(run.latest_learning_rate)
            .bind(i64::try_from(run.elapsed_milliseconds).map_err(store_error)?)
            .bind(run.cancel_requested)
            .bind(run.error_message)
            .bind(run.created_at)
            .bind(run.updated_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn save_training_run(
        &self,
        run: &TrainingRun,
    ) -> BoxFuture<'_, Result<(), TrainingStoreError>> {
        let run = run.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            let previous = load_training_run_in_transaction(&mut transaction, run.id)
                .await?
                .ok_or_else(|| TrainingStoreError(format!("training run not found: {}", run.id)))?;
            run.validate_update_from(&previous).map_err(store_error)?;
            if run.state == TrainingRunState::Completed {
                let final_epoch: Option<i64> = sqlx::query_scalar(
                    "SELECT epoch FROM training_checkpoints \
                     WHERE run_id = ? AND is_final = 1",
                )
                .bind(run.id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(store_error)?;
                if final_epoch != Some(i64::from(run.configuration.epochs)) {
                    return Err(TrainingStoreError(
                        "completed training run has no exact final checkpoint".into(),
                    ));
                }
            }
            let result = sqlx::query(
                "UPDATE training_runs SET state = ?, completed_epochs = ?, current_epoch = ?, \
                 completed_batches = ?, batches_in_epoch = ?, processed_examples = ?, \
                 latest_training_loss = ?, latest_validation_loss = ?, latest_learning_rate = ?, \
                 elapsed_milliseconds = ?, cancel_requested = MAX(cancel_requested, ?), \
                 error_message = ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(run_state_text(run.state))
            .bind(i64::from(run.completed_epochs))
            .bind(i64::from(run.current_epoch))
            .bind(i64::from(run.completed_batches))
            .bind(i64::from(run.batches_in_epoch))
            .bind(i64::try_from(run.processed_examples).map_err(store_error)?)
            .bind(run.latest_training_loss)
            .bind(run.latest_validation_loss)
            .bind(run.latest_learning_rate)
            .bind(i64::try_from(run.elapsed_milliseconds).map_err(store_error)?)
            .bind(run.cancel_requested)
            .bind(run.error_message)
            .bind(run.updated_at)
            .bind(run.id)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if result.rows_affected() != 1 {
                return Err(TrainingStoreError(format!(
                    "training run not found: {}",
                    run.id
                )));
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_training_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<TrainingRun>, TrainingStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, TrainingRunRecord>(
                "SELECT id, snapshot_id, backend_name, model_format, state, configuration_json, \
                 base_model_id, parent_checkpoint_id, backend_configuration_fingerprint, \
                 transformer_configuration_json, input_protocol, input_population_fingerprint, \
                 input_member_count, input_fingerprint, input_authority_kind, input_authority_id, \
                 input_authority_fingerprint, \
                 completed_epochs, current_epoch, completed_batches, batches_in_epoch, \
                 processed_examples, latest_training_loss, latest_validation_loss, \
                 latest_learning_rate, elapsed_milliseconds, \
                 cancel_requested, error_message, created_at, updated_at \
                 FROM training_runs WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(TrainingRunRecord::into_domain)
            .transpose()
        })
    }

    fn list_training_runs(&self) -> BoxFuture<'_, Result<Vec<TrainingRun>, TrainingStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, TrainingRunRecord>(
                "SELECT id, snapshot_id, backend_name, model_format, state, configuration_json, \
                 base_model_id, parent_checkpoint_id, backend_configuration_fingerprint, \
                 transformer_configuration_json, input_protocol, input_population_fingerprint, \
                 input_member_count, input_fingerprint, input_authority_kind, input_authority_id, \
                 input_authority_fingerprint, \
                 completed_epochs, current_epoch, completed_batches, batches_in_epoch, \
                 processed_examples, latest_training_loss, latest_validation_loss, \
                 latest_learning_rate, elapsed_milliseconds, \
                 cancel_requested, error_message, created_at, updated_at \
                 FROM training_runs ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(TrainingRunRecord::into_domain)
            .collect()
        })
    }

    fn query_training_runs(
        &self,
        query: TrainingRunQuery,
    ) -> BoxFuture<'_, Result<Vec<TrainingRun>, TrainingStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, snapshot_id, backend_name, model_format, state, configuration_json, \
                 base_model_id, parent_checkpoint_id, backend_configuration_fingerprint, \
                 transformer_configuration_json, input_protocol, input_population_fingerprint, \
                 input_member_count, input_fingerprint, input_authority_kind, input_authority_id, \
                 input_authority_fingerprint, \
                 completed_epochs, current_epoch, completed_batches, batches_in_epoch, \
                 processed_examples, latest_training_loss, latest_validation_loss, \
                 latest_learning_rate, elapsed_milliseconds, \
                 cancel_requested, error_message, created_at, updated_at FROM training_runs",
            );
            let mut has_condition = false;
            if let Some(snapshot_id) = query.snapshot_id {
                builder.push(" WHERE snapshot_id = ").push_bind(snapshot_id);
                has_condition = true;
            }
            if let Some(authority_id) = query.input_authority_id {
                builder.push(if has_condition { " AND " } else { " WHERE " });
                builder
                    .push("input_authority_id = ")
                    .push_bind(authority_id);
                has_condition = true;
            }
            if let Some(state) = query.state {
                builder.push(if has_condition { " AND " } else { " WHERE " });
                builder.push("state = ").push_bind(run_state_text(state));
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<TrainingRunRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(TrainingRunRecord::into_domain)
                .collect()
        })
    }

    fn request_training_cancellation(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<bool, TrainingStoreError>> {
        Box::pin(async move {
            let result = sqlx::query(
                "UPDATE training_runs SET cancel_requested = 1, updated_at = ? \
                 WHERE id = ? AND state IN ('queued', 'running')",
            )
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn create_checkpoint(
        &self,
        checkpoint: &TrainingCheckpoint,
    ) -> BoxFuture<'_, Result<(), TrainingStoreError>> {
        let checkpoint = checkpoint.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            let run = load_training_run_in_transaction(&mut transaction, checkpoint.run_id)
                .await?
                .ok_or_else(|| {
                    TrainingStoreError(format!(
                        "checkpoint training run not found: {}",
                        checkpoint.run_id
                    ))
                })?;
            checkpoint.validate_for_run(&run).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO training_checkpoints \
                 (id, run_id, epoch, artifact_path, artifact_checksum, model_format, \
                  artifact_size_bytes, training_loss, validation_loss, is_final, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(checkpoint.id)
            .bind(checkpoint.run_id)
            .bind(i64::from(checkpoint.epoch))
            .bind(checkpoint.artifact_path)
            .bind(checkpoint.artifact_checksum)
            .bind(checkpoint.model_format)
            .bind(i64::try_from(checkpoint.artifact_size_bytes).map_err(store_error)?)
            .bind(checkpoint.training_loss)
            .bind(checkpoint.validation_loss)
            .bind(checkpoint.is_final)
            .bind(checkpoint.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_checkpoint(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<TrainingCheckpoint>, TrainingStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, CheckpointRecord>(
                "SELECT id, run_id, epoch, artifact_path, artifact_checksum, model_format, \
                 artifact_size_bytes, training_loss, validation_loss, is_final, created_at \
                 FROM training_checkpoints WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(CheckpointRecord::into_domain)
            .transpose()
        })
    }

    fn list_checkpoints(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<TrainingCheckpoint>, TrainingStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, CheckpointRecord>(
                "SELECT id, run_id, epoch, artifact_path, artifact_checksum, model_format, \
                 artifact_size_bytes, training_loss, validation_loss, is_final, created_at \
                 FROM training_checkpoints WHERE run_id = ? ORDER BY epoch, id",
            )
            .bind(run_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(CheckpointRecord::into_domain)
            .collect()
        })
    }
}

#[derive(Debug, FromRow)]
struct TrainingRunRecord {
    id: Uuid,
    snapshot_id: Uuid,
    backend_name: String,
    model_format: String,
    state: String,
    configuration_json: String,
    base_model_id: Option<Uuid>,
    parent_checkpoint_id: Option<Uuid>,
    backend_configuration_fingerprint: Option<String>,
    transformer_configuration_json: Option<String>,
    input_protocol: Option<String>,
    input_population_fingerprint: Option<String>,
    input_member_count: Option<i64>,
    input_fingerprint: Option<String>,
    input_authority_kind: Option<String>,
    input_authority_id: Option<Uuid>,
    input_authority_fingerprint: Option<String>,
    completed_epochs: i64,
    current_epoch: i64,
    completed_batches: i64,
    batches_in_epoch: i64,
    processed_examples: i64,
    latest_training_loss: Option<f64>,
    latest_validation_loss: Option<f64>,
    latest_learning_rate: Option<f64>,
    elapsed_milliseconds: i64,
    cancel_requested: bool,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

async fn load_training_run_in_transaction(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    id: Uuid,
) -> Result<Option<TrainingRun>, TrainingStoreError> {
    sqlx::query_as::<_, TrainingRunRecord>(
        "SELECT id, snapshot_id, backend_name, model_format, state, configuration_json, \
         base_model_id, parent_checkpoint_id, backend_configuration_fingerprint, \
         transformer_configuration_json, input_protocol, input_population_fingerprint, \
         input_member_count, input_fingerprint, input_authority_kind, input_authority_id, \
         input_authority_fingerprint, completed_epochs, current_epoch, completed_batches, \
         batches_in_epoch, processed_examples, latest_training_loss, latest_validation_loss, \
         latest_learning_rate, elapsed_milliseconds, cancel_requested, error_message, \
         created_at, updated_at FROM training_runs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(store_error)?
    .map(TrainingRunRecord::into_domain)
    .transpose()
}

pub(crate) async fn validate_training_input_authority(
    connection: &mut SqliteConnection,
    run: &TrainingRun,
    require_current_roles: bool,
) -> Result<(), TrainingStoreError> {
    let Some(binding) = &run.input_binding else {
        return Ok(());
    };
    binding.validate().map_err(store_error)?;
    if binding.authority_kind != "training_benchmark_check" {
        return Err(TrainingStoreError(format!(
            "unsupported training input authority: {}",
            binding.authority_kind
        )));
    }
    let check = crate::training_benchmark::load_training_benchmark_check(
        connection,
        binding.authority_id,
        require_current_roles,
    )
    .await
    .map_err(store_error)?
    .ok_or_else(|| TrainingStoreError("training input authority check not found".into()))?;
    if !check.training_allowed()
        || check.fingerprint != binding.authority_fingerprint
        || check.training_snapshot_id != run.snapshot_id
        || check.training_input_protocol.stable_name() != binding.protocol
        || check.training_population_fingerprint != binding.population_fingerprint
        || check.training_member_count != binding.member_count
    {
        return Err(TrainingStoreError(
            "training run input binding differs from its clean authority".into(),
        ));
    }
    let (snapshot, members) =
        crate::load_verified_snapshot_with_members(connection, run.snapshot_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| TrainingStoreError("training snapshot not found".into()))?;
    let labels_json: String =
        sqlx::query_scalar("SELECT labels_json FROM dataset_definitions WHERE id = ?")
            .bind(snapshot.source_dataset_id)
            .fetch_one(&mut *connection)
            .await
            .map_err(store_error)?;
    let labels: Vec<String> = from_json(&labels_json)?;
    let mut training = Vec::new();
    let mut validation = Vec::new();
    for member in members {
        let example = TrainingExample {
            snapshot_member_id: member.id,
            text: member.text,
            label: member.label,
        };
        match member.split {
            dataset_core::domain::SnapshotSplit::Train => training.push(example),
            dataset_core::domain::SnapshotSplit::Validation => validation.push(example),
            dataset_core::domain::SnapshotSplit::Test => {}
        }
    }
    let request = TrainingRequest::in_memory(
        Uuid::nil(),
        snapshot.id,
        labels,
        training,
        validation,
        run.configuration.clone(),
    );
    if request.reproduce_input_fingerprint().map_err(store_error)? != binding.input_fingerprint {
        return Err(TrainingStoreError(
            "training run backend-facing input fingerprint differs from its snapshot".into(),
        ));
    }
    Ok(())
}

impl SqliteStore {
    /// Reproduces a persisted run's complete opaque authority, including the
    /// exact Train/Validation request digest, from immutable database facts.
    pub async fn verify_training_run_input_authority(
        &self,
        run_id: Uuid,
        require_current_roles: bool,
    ) -> Result<(), TrainingStoreError> {
        let run = TrainingStore::get_training_run(self, run_id)
            .await?
            .ok_or_else(|| TrainingStoreError(format!("training run not found: {run_id}")))?;
        let mut connection = self.pool.acquire().await.map_err(store_error)?;
        validate_training_input_authority(&mut connection, &run, require_current_roles).await
    }
}

impl TrainingRunRecord {
    fn into_domain(self) -> Result<TrainingRun, TrainingStoreError> {
        let value = TrainingRun {
            id: self.id,
            snapshot_id: self.snapshot_id,
            base_model_id: self.base_model_id,
            parent_checkpoint_id: self.parent_checkpoint_id,
            backend_configuration_fingerprint: self.backend_configuration_fingerprint,
            transformer_configuration: self
                .transformer_configuration_json
                .as_deref()
                .map(from_json)
                .transpose()?,
            input_binding: training_input_binding(
                self.input_protocol,
                self.input_population_fingerprint,
                self.input_member_count,
                self.input_fingerprint,
                self.input_authority_kind,
                self.input_authority_id,
                self.input_authority_fingerprint,
            )?,
            backend_name: self.backend_name,
            model_format: self.model_format,
            state: parse_run_state(&self.state)?,
            configuration: from_json(&self.configuration_json)?,
            completed_epochs: u32::try_from(self.completed_epochs).map_err(store_error)?,
            current_epoch: u32::try_from(self.current_epoch).map_err(store_error)?,
            completed_batches: u32::try_from(self.completed_batches).map_err(store_error)?,
            batches_in_epoch: u32::try_from(self.batches_in_epoch).map_err(store_error)?,
            processed_examples: u64::try_from(self.processed_examples).map_err(store_error)?,
            latest_training_loss: self.latest_training_loss,
            latest_validation_loss: self.latest_validation_loss,
            latest_learning_rate: self.latest_learning_rate,
            elapsed_milliseconds: u64::try_from(self.elapsed_milliseconds).map_err(store_error)?,
            cancel_requested: self.cancel_requested,
            error_message: self.error_message,
            created_at: self.created_at,
            updated_at: self.updated_at,
        };
        if value.input_binding.is_some() {
            value.validate_persisted().map_err(store_error)?;
        }
        Ok(value)
    }
}

fn training_input_binding(
    protocol: Option<String>,
    population_fingerprint: Option<String>,
    member_count: Option<i64>,
    input_fingerprint: Option<String>,
    authority_kind: Option<String>,
    authority_id: Option<Uuid>,
    authority_fingerprint: Option<String>,
) -> Result<Option<TrainingInputBinding>, TrainingStoreError> {
    match (
        protocol,
        population_fingerprint,
        member_count,
        input_fingerprint,
        authority_kind,
        authority_id,
        authority_fingerprint,
    ) {
        (None, None, None, None, None, None, None) => Ok(None),
        (
            Some(protocol),
            Some(population_fingerprint),
            Some(member_count),
            Some(input_fingerprint),
            Some(authority_kind),
            Some(authority_id),
            Some(authority_fingerprint),
        ) => {
            let value = TrainingInputBinding {
                protocol,
                population_fingerprint,
                member_count: u64::try_from(member_count).map_err(store_error)?,
                input_fingerprint,
                authority_kind,
                authority_id,
                authority_fingerprint,
            };
            value.validate().map_err(store_error)?;
            Ok(Some(value))
        }
        _ => Err(TrainingStoreError(
            "training input authority binding is incomplete".into(),
        )),
    }
}

#[derive(Debug, FromRow)]
struct CheckpointRecord {
    id: Uuid,
    run_id: Uuid,
    epoch: i64,
    artifact_path: String,
    artifact_checksum: String,
    model_format: String,
    artifact_size_bytes: i64,
    training_loss: f64,
    validation_loss: Option<f64>,
    is_final: bool,
    created_at: DateTime<Utc>,
}

impl CheckpointRecord {
    fn into_domain(self) -> Result<TrainingCheckpoint, TrainingStoreError> {
        Ok(TrainingCheckpoint {
            id: self.id,
            run_id: self.run_id,
            epoch: u32::try_from(self.epoch).map_err(store_error)?,
            artifact_path: self.artifact_path,
            artifact_checksum: self.artifact_checksum,
            artifact_size_bytes: u64::try_from(self.artifact_size_bytes).map_err(store_error)?,
            model_format: self.model_format,
            training_loss: self.training_loss,
            validation_loss: self.validation_loss,
            is_final: self.is_final,
            created_at: self.created_at,
        })
    }
}

fn run_state_text(state: TrainingRunState) -> &'static str {
    match state {
        TrainingRunState::Queued => "queued",
        TrainingRunState::Running => "running",
        TrainingRunState::Completed => "completed",
        TrainingRunState::Failed => "failed",
        TrainingRunState::Cancelled => "cancelled",
    }
}

fn parse_run_state(value: &str) -> Result<TrainingRunState, TrainingStoreError> {
    match value {
        "queued" => Ok(TrainingRunState::Queued),
        "running" => Ok(TrainingRunState::Running),
        "completed" => Ok(TrainingRunState::Completed),
        "failed" => Ok(TrainingRunState::Failed),
        "cancelled" => Ok(TrainingRunState::Cancelled),
        other => Err(TrainingStoreError(format!(
            "unknown training run state: {other}"
        ))),
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, TrainingStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, TrainingStoreError> {
    serde_json::from_str(value).map_err(store_error)
}

fn store_error(error: impl std::fmt::Display) -> TrainingStoreError {
    TrainingStoreError(error.to_string())
}

const fn architecture_text(architecture: EncoderArchitecture) -> &'static str {
    match architecture {
        EncoderArchitecture::Bert => "bert",
    }
}

fn parse_architecture(value: &str) -> Result<EncoderArchitecture, EncoderRegistryError> {
    match value {
        "bert" => Ok(EncoderArchitecture::Bert),
        other => Err(EncoderRegistryError(format!(
            "unknown encoder architecture: {other}"
        ))),
    }
}

fn encoder_json<T: serde::Serialize>(value: &T) -> Result<String, EncoderRegistryError> {
    serde_json::to_string(value).map_err(encoder_store_error)
}

fn encoder_from_json<T: serde::de::DeserializeOwned>(
    value: &str,
) -> Result<T, EncoderRegistryError> {
    serde_json::from_str(value).map_err(encoder_store_error)
}

fn encoder_store_error(error: impl std::fmt::Display) -> EncoderRegistryError {
    EncoderRegistryError(error.to_string())
}
