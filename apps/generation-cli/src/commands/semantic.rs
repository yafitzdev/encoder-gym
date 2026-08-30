use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, ensure};
use generation_core::{domain::DatasetDefinition, ports::DatasetStore};
use semantic_catalog::{
    GenerationSemanticAssignment, ResolvedSemanticContext, SemanticBindingDecision,
    SemanticCatalogStore, SemanticDatasetSchema, SemanticLayer, SemanticProfile,
    SemanticProfileDraft, SemanticScope, SemanticTarget, resolve_semantics,
};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::{
    cli::{SemanticCommand, SemanticLayerArg, SemanticTargetArg},
    document, presentation,
};

pub async fn execute(command: SemanticCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        SemanticCommand::ProfileCreate { file } => {
            let draft: SemanticProfileDraft = document::read(&file)?;
            validate_draft_scope(store, &draft).await?;
            let profile = SemanticProfile::create(draft)?;
            store.create_semantic_profile(&profile).await?;
            presentation::print(&profile)
        }
        SemanticCommand::ProfileRevise { id, file } => {
            let previous = require_profile(store, id).await?;
            let draft: SemanticProfileDraft = document::read(&file)?;
            validate_draft_scope(store, &draft).await?;
            let profile = SemanticProfile::revise(&previous, draft)?;
            store.create_semantic_profile(&profile).await?;
            presentation::print(&profile)
        }
        SemanticCommand::ProfileList { key } => {
            presentation::print(&store.list_semantic_profiles(key.as_deref()).await?)
        }
        SemanticCommand::ProfileShow { id } => {
            presentation::print(&require_profile(store, id).await?)
        }
        SemanticCommand::Bind {
            dataset_id,
            profile_id,
        } => {
            let dataset = require_dataset(store, dataset_id).await?;
            let profile = require_profile(store, profile_id).await?;
            let current = store.current_semantic_bindings(dataset_id).await?;
            let layer = SemanticLayer::for_scope(&profile.scope);
            let predecessor_id = current
                .iter()
                .find(|binding| binding.target == profile.target && binding.layer == layer)
                .map(|binding| binding.id);
            let decision = SemanticBindingDecision::bind(dataset_id, &profile, predecessor_id)?;
            validate_candidate_binding(store, &dataset, &current, &decision).await?;
            store.append_semantic_binding(&decision).await?;
            presentation::print(&decision)
        }
        SemanticCommand::Unbind {
            dataset_id,
            target,
            dimension,
            layer,
        } => {
            require_dataset(store, dataset_id).await?;
            let target = semantic_target(target, dimension)?;
            let layer = semantic_layer(layer);
            let current = store.current_semantic_bindings(dataset_id).await?;
            let predecessor = current
                .iter()
                .find(|binding| binding.target == target && binding.layer == layer)
                .context("that semantic target and layer do not have an active decision")?;
            ensure!(
                predecessor.profile_id.is_some(),
                "that semantic target and layer are already unbound"
            );
            let decision =
                SemanticBindingDecision::unbind(dataset_id, target, layer, Some(predecessor.id))?;
            store.append_semantic_binding(&decision).await?;
            presentation::print(&decision)
        }
        SemanticCommand::Bindings { dataset_id } => {
            require_dataset(store, dataset_id).await?;
            presentation::print(&store.current_semantic_bindings(dataset_id).await?)
        }
        SemanticCommand::Resolve { dataset_id } => {
            presentation::print(&resolve_dataset_semantics(store, dataset_id).await?)
        }
        SemanticCommand::Suggest { dataset_id } => {
            let dataset = require_dataset(store, dataset_id).await?;
            let schema = schema_from_dataset(&dataset);
            let mut seen = BTreeSet::new();
            let suggestions = store
                .list_semantic_profiles(None)
                .await?
                .into_iter()
                .filter(|profile| matches!(profile.scope, SemanticScope::Reusable))
                .filter(|profile| seen.insert(profile.key.clone()))
                .filter(|profile| profile_compatible(&schema, profile))
                .collect::<Vec<_>>();
            presentation::print(&suggestions)
        }
        SemanticCommand::JobContext { job_id } => {
            let assignment = store
                .get_generation_semantics(job_id)
                .await?
                .with_context(|| format!("generation job semantic context not found: {job_id}"))?;
            validate_assignment(&assignment)?;
            presentation::print(&assignment)
        }
    }
}

pub(crate) async fn resolve_dataset_semantics(
    store: &SqliteStore,
    dataset_id: Uuid,
) -> anyhow::Result<ResolvedSemanticContext> {
    let dataset = require_dataset(store, dataset_id).await?;
    let bindings = store.current_semantic_bindings(dataset_id).await?;
    let mut profiles = BTreeMap::new();
    for profile_id in bindings.iter().filter_map(|binding| binding.profile_id) {
        profiles.insert(profile_id, require_profile(store, profile_id).await?);
    }
    resolve_semantics(&schema_from_dataset(&dataset), &bindings, &profiles).map_err(Into::into)
}

pub(crate) fn schema_from_dataset(dataset: &DatasetDefinition) -> SemanticDatasetSchema {
    SemanticDatasetSchema {
        dataset_id: dataset.id,
        labels: dataset.labels.clone(),
        dimensions: dataset
            .dimensions
            .iter()
            .map(|dimension| (dimension.name.clone(), dimension.values.clone()))
            .collect(),
    }
}

async fn validate_candidate_binding(
    store: &SqliteStore,
    dataset: &DatasetDefinition,
    current: &[SemanticBindingDecision],
    candidate: &SemanticBindingDecision,
) -> anyhow::Result<()> {
    let mut proposed = current
        .iter()
        .filter(|binding| binding.target != candidate.target || binding.layer != candidate.layer)
        .cloned()
        .collect::<Vec<_>>();
    proposed.push(candidate.clone());
    let mut profiles = BTreeMap::new();
    for profile_id in proposed.iter().filter_map(|binding| binding.profile_id) {
        profiles.insert(profile_id, require_profile(store, profile_id).await?);
    }
    resolve_semantics(&schema_from_dataset(dataset), &proposed, &profiles)?;
    Ok(())
}

async fn validate_draft_scope(
    store: &SqliteStore,
    draft: &SemanticProfileDraft,
) -> anyhow::Result<()> {
    if let SemanticScope::Dataset { dataset_id } = draft.scope {
        let dataset = require_dataset(store, dataset_id).await?;
        let schema = schema_from_dataset(&dataset);
        ensure!(
            schema.values_for(&draft.target).is_some(),
            "dataset {dataset_id} has no target {}",
            draft.target.key()
        );
    }
    Ok(())
}

fn profile_compatible(schema: &SemanticDatasetSchema, profile: &SemanticProfile) -> bool {
    schema
        .values_for(&profile.target)
        .is_some_and(|values| profile.entries.keys().all(|entry| values.contains(entry)))
}

async fn require_dataset(store: &SqliteStore, id: Uuid) -> anyhow::Result<DatasetDefinition> {
    store
        .get_dataset(id)
        .await?
        .with_context(|| format!("dataset not found: {id}"))
}

async fn require_profile(store: &SqliteStore, id: Uuid) -> anyhow::Result<SemanticProfile> {
    let profile = store
        .get_semantic_profile(id)
        .await?
        .with_context(|| format!("semantic profile not found: {id}"))?;
    ensure!(
        profile.reproduce_fingerprint()? == profile.fingerprint,
        "semantic profile {id} failed its integrity check"
    );
    Ok(profile)
}

fn semantic_target(
    target: SemanticTargetArg,
    dimension: Option<String>,
) -> anyhow::Result<SemanticTarget> {
    match target {
        SemanticTargetArg::Labels => {
            ensure!(
                dimension.is_none(),
                "--dimension is only valid for a dimension target"
            );
            Ok(SemanticTarget::Labels)
        }
        SemanticTargetArg::Dimension => Ok(SemanticTarget::Dimension {
            name: dimension.context("--dimension is required for a dimension target")?,
        }),
    }
}

fn semantic_layer(layer: SemanticLayerArg) -> SemanticLayer {
    match layer {
        SemanticLayerArg::Reusable => SemanticLayer::Reusable,
        SemanticLayerArg::DatasetOverride => SemanticLayer::DatasetOverride,
    }
}

fn validate_assignment(assignment: &GenerationSemanticAssignment) -> anyhow::Result<()> {
    ensure!(
        assignment.context.reproduce_fingerprint()? == assignment.context.fingerprint,
        "generation semantic context failed its integrity check"
    );
    ensure!(
        assignment.reproduce_fingerprint()? == assignment.fingerprint,
        "generation semantic assignment failed its integrity check"
    );
    Ok(())
}

trait SchemaValues {
    fn values_for(&self, target: &SemanticTarget) -> Option<&Vec<String>>;
}

impl SchemaValues for SemanticDatasetSchema {
    fn values_for(&self, target: &SemanticTarget) -> Option<&Vec<String>> {
        match target {
            SemanticTarget::Labels => Some(&self.labels),
            SemanticTarget::Dimension { name } => self.dimensions.get(name),
        }
    }
}
