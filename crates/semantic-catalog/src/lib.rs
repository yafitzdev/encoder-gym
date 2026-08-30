//! Reusable, versioned semantics for dataset labels and categorical dimensions.
//!
//! This crate owns immutable semantic profiles, explicit dataset bindings, and
//! deterministic resolution. It deliberately knows nothing about prompts,
//! generation providers, SQLite, HTTP, or presentation.

use std::{collections::BTreeMap, future::Future, pin::Pin};

use artifact_core::fingerprint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticScope {
    Reusable,
    Dataset { dataset_id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticTarget {
    Labels,
    Dimension { name: String },
}

impl SemanticTarget {
    pub fn key(&self) -> String {
        match self {
            Self::Labels => "labels".into(),
            Self::Dimension { name } => format!("dimension:{name}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticEntry {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub counterexamples: Vec<String>,
    #[serde(default)]
    pub inclusion_rules: Vec<String>,
    #[serde(default)]
    pub exclusion_rules: Vec<String>,
}

impl SemanticEntry {
    fn normalize(mut self, path: &str) -> Result<Self, SemanticError> {
        self.description = normalize_optional(self.description, &format!("{path}.description"))?;
        self.examples = normalize_list(self.examples, &format!("{path}.examples"))?;
        self.counterexamples =
            normalize_list(self.counterexamples, &format!("{path}.counterexamples"))?;
        self.inclusion_rules =
            normalize_list(self.inclusion_rules, &format!("{path}.inclusion_rules"))?;
        self.exclusion_rules =
            normalize_list(self.exclusion_rules, &format!("{path}.exclusion_rules"))?;
        if self.description.is_none()
            && self.examples.is_empty()
            && self.counterexamples.is_empty()
            && self.inclusion_rules.is_empty()
            && self.exclusion_rules.is_empty()
        {
            return Err(SemanticError::Validation(format!(
                "{path} must contain semantic information"
            )));
        }
        Ok(self)
    }

    fn overlay(&mut self, overlay: &Self) {
        if overlay.description.is_some() {
            self.description.clone_from(&overlay.description);
        }
        if !overlay.examples.is_empty() {
            self.examples.clone_from(&overlay.examples);
        }
        if !overlay.counterexamples.is_empty() {
            self.counterexamples.clone_from(&overlay.counterexamples);
        }
        if !overlay.inclusion_rules.is_empty() {
            self.inclusion_rules.clone_from(&overlay.inclusion_rules);
        }
        if !overlay.exclusion_rules.is_empty() {
            self.exclusion_rules.clone_from(&overlay.exclusion_rules);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticProfileDraft {
    pub schema_version: u32,
    pub key: String,
    pub scope: SemanticScope,
    pub target: SemanticTarget,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub entries: BTreeMap<String, SemanticEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticProfile {
    pub id: Uuid,
    pub key: String,
    pub version: u32,
    pub predecessor_id: Option<Uuid>,
    pub scope: SemanticScope,
    pub target: SemanticTarget,
    pub description: Option<String>,
    pub entries: BTreeMap<String, SemanticEntry>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SemanticProfile {
    pub fn create(draft: SemanticProfileDraft) -> Result<Self, SemanticError> {
        Self::from_draft(draft, 1, None)
    }

    pub fn revise(previous: &Self, draft: SemanticProfileDraft) -> Result<Self, SemanticError> {
        if draft.key.trim() != previous.key
            || draft.scope != previous.scope
            || normalize_target(draft.target.clone())? != previous.target
        {
            return Err(SemanticError::Validation(
                "a revision cannot change profile key, scope, or target".into(),
            ));
        }
        Self::from_draft(
            draft,
            previous
                .version
                .checked_add(1)
                .ok_or_else(|| SemanticError::Validation("profile version overflowed".into()))?,
            Some(previous.id),
        )
    }

    fn from_draft(
        draft: SemanticProfileDraft,
        version: u32,
        predecessor_id: Option<Uuid>,
    ) -> Result<Self, SemanticError> {
        if draft.schema_version != 1 {
            return Err(SemanticError::Validation(format!(
                "unsupported semantic profile schema_version {}; expected 1",
                draft.schema_version
            )));
        }
        let key = nonempty(draft.key, "key")?;
        let target = normalize_target(draft.target)?;
        let description = normalize_optional(draft.description, "description")?;
        let mut entries = BTreeMap::new();
        for (name, entry) in draft.entries {
            let name = nonempty(name, "entry name")?;
            if entries
                .insert(name.clone(), entry.normalize(&format!("entries.{name}"))?)
                .is_some()
            {
                return Err(SemanticError::Validation(format!(
                    "duplicate semantic entry {name:?}"
                )));
            }
        }
        if description.is_none() && entries.is_empty() {
            return Err(SemanticError::Validation(
                "profile must contain a description or at least one entry".into(),
            ));
        }
        let mut profile = Self {
            id: Uuid::new_v4(),
            key,
            version,
            predecessor_id,
            scope: draft.scope,
            target,
            description,
            entries,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        profile.fingerprint = profile.reproduce_fingerprint()?;
        Ok(profile)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SemanticError> {
        fingerprint(&(
            self.id,
            &self.key,
            self.version,
            self.predecessor_id,
            &self.scope,
            &self.target,
            &self.description,
            &self.entries,
            self.created_at,
        ))
        .map_err(|error| SemanticError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticLayer {
    Reusable,
    DatasetOverride,
}

impl SemanticLayer {
    pub fn for_scope(scope: &SemanticScope) -> Self {
        match scope {
            SemanticScope::Reusable => Self::Reusable,
            SemanticScope::Dataset { .. } => Self::DatasetOverride,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticBindingDecision {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub target: SemanticTarget,
    pub layer: SemanticLayer,
    pub profile_id: Option<Uuid>,
    pub profile_fingerprint: Option<String>,
    pub predecessor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SemanticBindingDecision {
    pub fn bind(
        dataset_id: Uuid,
        profile: &SemanticProfile,
        predecessor_id: Option<Uuid>,
    ) -> Result<Self, SemanticError> {
        if let SemanticScope::Dataset {
            dataset_id: scoped_id,
        } = profile.scope
            && scoped_id != dataset_id
        {
            return Err(SemanticError::Validation(format!(
                "dataset-scoped profile belongs to {scoped_id}, not {dataset_id}"
            )));
        }
        Self::new(
            dataset_id,
            profile.target.clone(),
            SemanticLayer::for_scope(&profile.scope),
            Some(profile.id),
            Some(profile.fingerprint.clone()),
            predecessor_id,
        )
    }

    pub fn unbind(
        dataset_id: Uuid,
        target: SemanticTarget,
        layer: SemanticLayer,
        predecessor_id: Option<Uuid>,
    ) -> Result<Self, SemanticError> {
        Self::new(dataset_id, target, layer, None, None, predecessor_id)
    }

    fn new(
        dataset_id: Uuid,
        target: SemanticTarget,
        layer: SemanticLayer,
        profile_id: Option<Uuid>,
        profile_fingerprint: Option<String>,
        predecessor_id: Option<Uuid>,
    ) -> Result<Self, SemanticError> {
        let mut decision = Self {
            id: Uuid::new_v4(),
            dataset_id,
            target: normalize_target(target)?,
            layer,
            profile_id,
            profile_fingerprint,
            predecessor_id,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        decision.fingerprint = decision.reproduce_fingerprint()?;
        Ok(decision)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SemanticError> {
        fingerprint(&(
            self.id,
            self.dataset_id,
            &self.target,
            self.layer,
            self.profile_id,
            &self.profile_fingerprint,
            self.predecessor_id,
            self.created_at,
        ))
        .map_err(|error| SemanticError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDatasetSchema {
    pub dataset_id: Uuid,
    pub labels: Vec<String>,
    pub dimensions: BTreeMap<String, Vec<String>>,
}

impl SemanticDatasetSchema {
    fn values(&self, target: &SemanticTarget) -> Option<&[String]> {
        match target {
            SemanticTarget::Labels => Some(&self.labels),
            SemanticTarget::Dimension { name } => self.dimensions.get(name).map(Vec::as_slice),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSemanticSource {
    pub target: SemanticTarget,
    pub layer: SemanticLayer,
    pub binding_id: Uuid,
    pub binding_fingerprint: String,
    pub profile_id: Uuid,
    pub profile_key: String,
    pub profile_version: u32,
    pub profile_fingerprint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSemanticTarget {
    pub description: Option<String>,
    pub entries: BTreeMap<String, SemanticEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSemanticContext {
    pub dataset_id: Uuid,
    pub targets: BTreeMap<String, ResolvedSemanticTarget>,
    pub sources: Vec<ResolvedSemanticSource>,
    pub resolved_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ResolvedSemanticContext {
    pub fn target(&self, target: &SemanticTarget) -> Option<&ResolvedSemanticTarget> {
        self.targets.get(&target.key())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SemanticError> {
        fingerprint(&(
            self.dataset_id,
            &self.targets,
            &self.sources,
            self.resolved_at,
        ))
        .map_err(|error| SemanticError::Fingerprint(error.to_string()))
    }
}

/// Resolves only explicit current bindings. Dataset overrides are layered over
/// reusable profiles; unbound or unnamed concepts remain ordinary schema text.
pub fn resolve_semantics(
    schema: &SemanticDatasetSchema,
    current_bindings: &[SemanticBindingDecision],
    profiles: &BTreeMap<Uuid, SemanticProfile>,
) -> Result<ResolvedSemanticContext, SemanticError> {
    let mut bindings = current_bindings.to_vec();
    bindings.sort_by_key(|binding| (binding.target.key(), binding.layer));
    let mut targets: BTreeMap<String, ResolvedSemanticTarget> = BTreeMap::new();
    let mut sources = Vec::new();
    for binding in bindings {
        if binding.dataset_id != schema.dataset_id {
            return Err(SemanticError::Validation(format!(
                "binding {} belongs to another dataset",
                binding.id
            )));
        }
        if binding.reproduce_fingerprint()? != binding.fingerprint {
            return Err(SemanticError::Integrity(format!(
                "binding {} fingerprint mismatch",
                binding.id
            )));
        }
        let Some(profile_id) = binding.profile_id else {
            continue;
        };
        let profile = profiles.get(&profile_id).ok_or_else(|| {
            SemanticError::Integrity(format!("bound profile {profile_id} is missing"))
        })?;
        if profile.reproduce_fingerprint()? != profile.fingerprint
            || binding.profile_fingerprint.as_deref() != Some(&profile.fingerprint)
        {
            return Err(SemanticError::Integrity(format!(
                "bound profile {profile_id} fingerprint mismatch"
            )));
        }
        if profile.target != binding.target
            || SemanticLayer::for_scope(&profile.scope) != binding.layer
        {
            return Err(SemanticError::Integrity(format!(
                "binding {} does not match profile target or scope",
                binding.id
            )));
        }
        if let SemanticScope::Dataset { dataset_id } = profile.scope
            && dataset_id != schema.dataset_id
        {
            return Err(SemanticError::Validation(format!(
                "profile {profile_id} is scoped to dataset {dataset_id}"
            )));
        }
        let valid_values = schema.values(&profile.target).ok_or_else(|| {
            SemanticError::Validation(format!("dataset has no target {}", profile.target.key()))
        })?;
        for entry in profile.entries.keys() {
            if !valid_values.contains(entry) {
                return Err(SemanticError::Validation(format!(
                    "profile {} defines unknown value {entry:?} for {}",
                    profile.key,
                    profile.target.key()
                )));
            }
        }
        let resolved = targets.entry(profile.target.key()).or_default();
        if profile.description.is_some() {
            resolved.description.clone_from(&profile.description);
        }
        for (name, entry) in &profile.entries {
            resolved
                .entries
                .entry(name.clone())
                .or_default()
                .overlay(entry);
        }
        sources.push(ResolvedSemanticSource {
            target: profile.target.clone(),
            layer: binding.layer,
            binding_id: binding.id,
            binding_fingerprint: binding.fingerprint,
            profile_id,
            profile_key: profile.key.clone(),
            profile_version: profile.version,
            profile_fingerprint: profile.fingerprint.clone(),
        });
    }
    let mut context = ResolvedSemanticContext {
        dataset_id: schema.dataset_id,
        targets,
        sources,
        resolved_at: Utc::now(),
        fingerprint: String::new(),
    };
    context.fingerprint = context.reproduce_fingerprint()?;
    Ok(context)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationSemanticAssignment {
    pub job_id: Uuid,
    pub context: ResolvedSemanticContext,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl GenerationSemanticAssignment {
    pub fn new(job_id: Uuid, context: ResolvedSemanticContext) -> Result<Self, SemanticError> {
        let mut assignment = Self {
            job_id,
            context,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        assignment.fingerprint = assignment.reproduce_fingerprint()?;
        Ok(assignment)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, SemanticError> {
        fingerprint(&(self.job_id, &self.context, self.created_at))
            .map_err(|error| SemanticError::Fingerprint(error.to_string()))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SemanticError {
    #[error("invalid semantic catalog data: {0}")]
    Validation(String),
    #[error("semantic catalog integrity failure: {0}")]
    Integrity(String),
    #[error("could not fingerprint semantic artifact: {0}")]
    Fingerprint(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("semantic catalog persistence failed: {0}")]
pub struct SemanticStoreError(pub String);

pub trait SemanticCatalogStore: Send + Sync {
    fn create_semantic_profile(
        &self,
        profile: &SemanticProfile,
    ) -> BoxFuture<'_, Result<(), SemanticStoreError>>;
    fn get_semantic_profile(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<SemanticProfile>, SemanticStoreError>>;
    fn list_semantic_profiles(
        &self,
        key: Option<&str>,
    ) -> BoxFuture<'_, Result<Vec<SemanticProfile>, SemanticStoreError>>;
    fn append_semantic_binding(
        &self,
        decision: &SemanticBindingDecision,
    ) -> BoxFuture<'_, Result<(), SemanticStoreError>>;
    fn current_semantic_bindings(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SemanticBindingDecision>, SemanticStoreError>>;
    fn save_generation_semantics(
        &self,
        assignment: &GenerationSemanticAssignment,
    ) -> BoxFuture<'_, Result<(), SemanticStoreError>>;
    fn get_generation_semantics(
        &self,
        job_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<GenerationSemanticAssignment>, SemanticStoreError>>;
}

fn normalize_target(target: SemanticTarget) -> Result<SemanticTarget, SemanticError> {
    match target {
        SemanticTarget::Labels => Ok(SemanticTarget::Labels),
        SemanticTarget::Dimension { name } => Ok(SemanticTarget::Dimension {
            name: nonempty(name, "target dimension name")?,
        }),
    }
}

fn nonempty(value: String, path: &str) -> Result<String, SemanticError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(SemanticError::Validation(format!(
            "{path} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

fn normalize_optional(value: Option<String>, path: &str) -> Result<Option<String>, SemanticError> {
    value.map(|value| nonempty(value, path)).transpose()
}

fn normalize_list(values: Vec<String>, path: &str) -> Result<Vec<String>, SemanticError> {
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = nonempty(value, path)?;
        if normalized.contains(&value) {
            return Err(SemanticError::Validation(format!(
                "{path} contains duplicate value {value:?}"
            )));
        }
        normalized.push(value);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn draft(scope: SemanticScope, description: &str) -> SemanticProfileDraft {
        SemanticProfileDraft {
            schema_version: 1,
            key: "difficulty-taxonomy".into(),
            scope,
            target: SemanticTarget::Dimension {
                name: "difficulty".into(),
            },
            description: Some(description.into()),
            entries: BTreeMap::from([(
                "hard".into(),
                SemanticEntry {
                    description: Some("Requires multiple clues".into()),
                    examples: vec!["Indirect intent".into()],
                    ..SemanticEntry::default()
                },
            )]),
        }
    }

    #[test]
    fn revisions_are_immutable_and_keep_identity() {
        let first =
            SemanticProfile::create(draft(SemanticScope::Reusable, "Difficulty")).expect("profile");
        let second =
            SemanticProfile::revise(&first, draft(SemanticScope::Reusable, "Revised difficulty"))
                .expect("revision");
        assert_eq!(second.version, 2);
        assert_eq!(second.predecessor_id, Some(first.id));
        assert_ne!(second.id, first.id);
        assert_ne!(second.fingerprint, first.fingerprint);
    }

    #[test]
    fn dataset_override_wins_without_erasing_unspecified_reusable_fields() {
        let dataset_id = Uuid::new_v4();
        let reusable =
            SemanticProfile::create(draft(SemanticScope::Reusable, "Shared")).expect("reusable");
        let mut override_draft = draft(SemanticScope::Dataset { dataset_id }, "Local");
        override_draft
            .entries
            .get_mut("hard")
            .expect("hard")
            .description = Some("Dataset-specific hard".into());
        override_draft
            .entries
            .get_mut("hard")
            .expect("hard")
            .examples
            .clear();
        let local = SemanticProfile::create(override_draft).expect("override");
        let reusable_binding =
            SemanticBindingDecision::bind(dataset_id, &reusable, None).expect("binding");
        let local_binding =
            SemanticBindingDecision::bind(dataset_id, &local, None).expect("binding");
        let schema = SemanticDatasetSchema {
            dataset_id,
            labels: vec!["billing".into()],
            dimensions: BTreeMap::from([("difficulty".into(), vec!["easy".into(), "hard".into()])]),
        };
        let profiles = BTreeMap::from([(reusable.id, reusable), (local.id, local)]);
        let context = resolve_semantics(&schema, &[local_binding, reusable_binding], &profiles)
            .expect("resolved");
        let hard = context
            .target(&SemanticTarget::Dimension {
                name: "difficulty".into(),
            })
            .expect("target")
            .entries
            .get("hard")
            .expect("hard");
        assert_eq!(hard.description.as_deref(), Some("Dataset-specific hard"));
        assert_eq!(hard.examples, vec!["Indirect intent"]);
        assert_eq!(context.sources[0].layer, SemanticLayer::Reusable);
        assert_eq!(context.sources[1].layer, SemanticLayer::DatasetOverride);
    }

    #[test]
    fn resolution_rejects_unknown_values() {
        let dataset_id = Uuid::new_v4();
        let profile =
            SemanticProfile::create(draft(SemanticScope::Reusable, "Difficulty")).expect("profile");
        let binding = SemanticBindingDecision::bind(dataset_id, &profile, None).expect("binding");
        let schema = SemanticDatasetSchema {
            dataset_id,
            labels: vec![],
            dimensions: BTreeMap::from([("difficulty".into(), vec!["easy".into()])]),
        };
        assert!(
            resolve_semantics(
                &schema,
                &[binding],
                &BTreeMap::from([(profile.id, profile)])
            )
            .is_err()
        );
    }
}
