//! Project-scoped, non-secret provider configuration.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Invalid, require, validate_hash, validate_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRole {
    Generation,
    Advisor,
    Evaluator,
}

impl ProviderRole {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Generation => "generation",
            Self::Advisor => "advisor",
            Self::Evaluator => "evaluator",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Fake,
    OpenaiCompatible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthentication {
    None,
    Bearer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretReference {
    /// Stable account key for an OS credential service. It is not a secret.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_fallback: Option<String>,
}

impl SecretReference {
    pub fn for_connection(project_id: Uuid, connection_id: Uuid) -> Result<Self, Invalid> {
        require(
            !project_id.is_nil() && !connection_id.is_nil(),
            "Invalid credential connection identity.",
        )?;
        Ok(Self {
            id: format!("{project_id}:connection:{connection_id}"),
            environment_fallback: None,
        })
    }

    pub fn connection_id(&self, project_id: Uuid) -> Option<Uuid> {
        let raw = self.id.strip_prefix(&format!("{project_id}:connection:"))?;
        let id = Uuid::parse_str(raw).ok()?;
        (!id.is_nil() && id.to_string() == raw).then_some(id)
    }

    /// A connection uses its own process environment slot, never a mutable role
    /// default. This is a variable name only, not a credential value.
    pub fn execution_environment(
        &self,
        project_id: Uuid,
        role: ProviderRole,
    ) -> Result<Option<String>, Invalid> {
        self.validate(project_id, role)?;
        Ok(self
            .connection_id(project_id)
            .map(|id| {
                format!(
                    "ENCODER_GYM_CONNECTION_{}_API_KEY",
                    id.simple().to_string().to_uppercase()
                )
            })
            .or_else(|| self.environment_fallback.clone()))
    }

    pub fn for_role(
        project_id: Uuid,
        role: ProviderRole,
        environment_fallback: Option<String>,
    ) -> Result<Self, Invalid> {
        let value = Self {
            id: format!("{project_id}:{}", role.key()),
            environment_fallback,
        };
        value.validate(project_id, role)?;
        Ok(value)
    }

    pub fn validate(&self, project_id: Uuid, role: ProviderRole) -> Result<(), Invalid> {
        let connection = self.connection_id(project_id);
        require(
            self.id == format!("{project_id}:{}", role.key()) || connection.is_some(),
            "Secret reference does not match its project and provider role.",
        )?;
        require(
            connection.is_none() || self.environment_fallback.is_none(),
            "Pinned connections cannot fall back to a role credential.",
        )?;
        if let Some(name) = &self.environment_fallback {
            require(
                valid_environment_name(name),
                "Credential fallback must name an uppercase environment variable, never contain a secret.",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderLimits {
    pub maximum_requests: u32,
    pub maximum_input_tokens: u64,
    pub maximum_output_tokens: u64,
    /// Zero means the provider is configured for no paid spend.
    pub maximum_cost_microusd: u64,
}

impl ProviderLimits {
    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            self.maximum_requests > 0
                && self.maximum_requests <= 1_000_000
                && self.maximum_input_tokens > 0
                && self.maximum_output_tokens > 0,
            "Provider limits must be positive and finite.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfiguration {
    pub role: ProviderRole,
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub model: String,
    pub authentication: ProviderAuthentication,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretReference>,
    pub limits: ProviderLimits,
}

impl ProviderConfiguration {
    pub fn validate(&self, project_id: Uuid) -> Result<(), Invalid> {
        validate_name(&self.model)?;
        self.limits.validate()?;
        match self.kind {
            ProviderKind::Fake => require(
                self.endpoint.is_none()
                    && self.authentication == ProviderAuthentication::None
                    && self.secret.is_none()
                    && self.limits.maximum_cost_microusd == 0,
                "Fake providers cannot have endpoints, credentials, or paid spend.",
            )?,
            ProviderKind::OpenaiCompatible => {
                let endpoint = self.endpoint.as_deref().ok_or_else(|| {
                    Invalid("OpenAI-compatible provider requires an endpoint.".into())
                })?;
                require(
                    valid_endpoint(endpoint),
                    "Provider endpoint is not a safe HTTP(S) URL.",
                )?;
                match self.authentication {
                    ProviderAuthentication::None => require(
                        self.secret.is_none(),
                        "Unauthenticated providers cannot reference a secret.",
                    )?,
                    ProviderAuthentication::Bearer => self
                        .secret
                        .as_ref()
                        .ok_or_else(|| {
                            Invalid("Bearer authentication requires a secret reference.".into())
                        })?
                        .validate(project_id, self.role)?,
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCatalog {
    pub id: Uuid,
    pub project_id: Uuid,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_revision_id: Option<Uuid>,
    pub providers: Vec<ProviderConfiguration>,
    pub actor: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProviderCatalog {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        id: Uuid,
        project_id: Uuid,
        sequence: u64,
        previous_revision_id: Option<Uuid>,
        mut providers: Vec<ProviderConfiguration>,
        actor: impl Into<String>,
        reason: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, Invalid> {
        providers.sort_by_key(|provider| provider.role);
        let mut value = Self {
            id,
            project_id,
            sequence,
            previous_revision_id,
            providers,
            actor: actor.into(),
            reason: reason.into(),
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate()?;
        Ok(value)
    }

    pub fn provider(&self, role: ProviderRole) -> Option<&ProviderConfiguration> {
        self.providers.iter().find(|provider| provider.role == role)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, Invalid> {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "id": self.id,
            "projectId": self.project_id,
            "sequence": self.sequence,
            "previousRevisionId": self.previous_revision_id,
            "providers": self.providers,
            "actor": self.actor,
            "reason": self.reason,
            "createdAt": self.created_at,
        }))
        .map_err(|error| Invalid(format!("Could not fingerprint provider settings: {error}")))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.id.is_nil() && !self.project_id.is_nil() && self.sequence > 0,
            "Provider settings identities are invalid.",
        )?;
        require(
            (self.sequence == 1 && self.previous_revision_id.is_none())
                || (self.sequence > 1 && self.previous_revision_id.is_some()),
            "Provider settings must form an ordered revision chain.",
        )?;
        validate_name(&self.actor)?;
        validate_name(&self.reason)?;
        let mut roles = BTreeSet::new();
        for provider in &self.providers {
            provider.validate(self.project_id)?;
            require(
                roles.insert(provider.role),
                "Provider roles must be unique.",
            )?;
        }
        require(
            roles.contains(&ProviderRole::Generation) && roles.contains(&ProviderRole::Advisor),
            "Generation and advisor providers must be configured separately.",
        )?;
        validate_hash(&self.fingerprint)?;
        require(
            self.reproduce_fingerprint()? == self.fingerprint,
            "Provider settings fingerprint does not reproduce.",
        )
    }
}

fn valid_environment_name(value: &str) -> bool {
    value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|first| first == b'_' || first.is_ascii_uppercase())
        && value
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn valid_endpoint(value: &str) -> bool {
    value.len() <= 2_048
        && (value.starts_with("https://") || value.starts_with("http://"))
        && !value.contains(['@', '?', '#'])
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_connections_are_project_bound_and_cannot_fall_back_to_role_defaults() {
        let project = Uuid::new_v4();
        let connection = Uuid::new_v4();
        let mut reference = SecretReference::for_connection(project, connection).unwrap();
        reference.validate(project, ProviderRole::Advisor).unwrap();
        reference
            .validate(project, ProviderRole::Generation)
            .unwrap();
        assert!(
            reference
                .validate(Uuid::new_v4(), ProviderRole::Advisor)
                .is_err()
        );
        assert_eq!(
            reference
                .execution_environment(project, ProviderRole::Advisor)
                .unwrap(),
            Some(format!(
                "ENCODER_GYM_CONNECTION_{}_API_KEY",
                connection.simple().to_string().to_uppercase()
            ))
        );
        reference.environment_fallback = Some("SYNTH_ADVISOR_API_KEY".into());
        assert!(reference.validate(project, ProviderRole::Advisor).is_err());
        assert!(SecretReference::for_connection(project, Uuid::nil()).is_err());
    }

    fn provider(project_id: Uuid, role: ProviderRole, environment: &str) -> ProviderConfiguration {
        ProviderConfiguration {
            role,
            kind: ProviderKind::OpenaiCompatible,
            endpoint: Some("https://api.example.test/v1".into()),
            model: "bounded-model".into(),
            authentication: ProviderAuthentication::Bearer,
            secret: Some(
                SecretReference::for_role(project_id, role, Some(environment.into())).unwrap(),
            ),
            limits: ProviderLimits {
                maximum_requests: 10,
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 2_000,
                maximum_cost_microusd: 50_000,
            },
        }
    }

    #[test]
    fn separate_roles_have_project_scoped_secret_references() {
        let project_id = Uuid::new_v4();
        let catalog = ProviderCatalog::create(
            Uuid::new_v4(),
            project_id,
            1,
            None,
            vec![
                provider(project_id, ProviderRole::Advisor, "SYNTH_ADVISOR_API_KEY"),
                provider(project_id, ProviderRole::Generation, "SYNTH_OPENAI_API_KEY"),
            ],
            "operator",
            "configure separate providers",
            Utc::now(),
        )
        .unwrap();
        assert_ne!(
            catalog.provider(ProviderRole::Generation).unwrap().secret,
            catalog.provider(ProviderRole::Advisor).unwrap().secret
        );
        catalog.validate().unwrap();
    }

    #[test]
    fn secrets_and_credential_bearing_endpoints_are_rejected() {
        let project_id = Uuid::new_v4();
        let mut value = provider(project_id, ProviderRole::Generation, "GENERATION_KEY");
        value.secret.as_mut().unwrap().environment_fallback = Some("sk-live-secret".into());
        assert!(value.validate(project_id).is_err());
        value.secret = SecretReference::for_role(
            project_id,
            ProviderRole::Generation,
            Some("GENERATION_KEY".into()),
        )
        .ok();
        value.endpoint = Some("https://secret@example.test/v1".into());
        assert!(value.validate(project_id).is_err());
    }

    #[test]
    fn fake_providers_cannot_hide_external_configuration() {
        let project_id = Uuid::new_v4();
        let mut value = provider(project_id, ProviderRole::Advisor, "ADVISOR_KEY");
        value.kind = ProviderKind::Fake;
        assert!(value.validate(project_id).is_err());
        value.endpoint = None;
        value.authentication = ProviderAuthentication::None;
        value.secret = None;
        value.limits.maximum_cost_microusd = 0;
        assert!(value.validate(project_id).is_ok());
    }
}
