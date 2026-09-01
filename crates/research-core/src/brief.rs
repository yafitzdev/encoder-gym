use std::collections::{BTreeMap, BTreeSet};

use artifact_core::fingerprint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{ResearchError, nonempty, normalized_list, optional};

pub const RESEARCH_BRIEF_SCHEMA_VERSION: u32 = 1;
pub const RESEARCH_RUNTIME: &str = "pi";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub id: Uuid,
    pub fingerprint: String,
}

impl ArtifactReference {
    fn normalize(mut self, path: &str) -> Result<Self, ResearchError> {
        self.fingerprint = nonempty(self.fingerprint, &format!("{path}.fingerprint"))?;
        if !self.fingerprint.starts_with("sha256:") {
            return Err(ResearchError::Validation(format!(
                "{path}.fingerprint must be a sha256 fingerprint"
            )));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchTarget {
    pub language: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub time_period: Option<String>,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
}

impl ResearchTarget {
    fn normalize(mut self) -> Result<Self, ResearchError> {
        self.language = nonempty(self.language, "target.language")?;
        self.region = optional(self.region, "target.region")?;
        self.time_period = optional(self.time_period, "target.time_period")?;
        self.audience = optional(self.audience, "target.audience")?;
        self.channel = optional(self.channel, "target.channel")?;
        self.domain = optional(self.domain, "target.domain")?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchBudgets {
    pub max_model_turns: u32,
    pub max_searches: u32,
    pub max_fetched_pages: u32,
    pub max_fetched_bytes: u64,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_wall_clock_seconds: u64,
    pub max_cost_microusd: u64,
    pub max_retries_per_call: u32,
}

impl ResearchBudgets {
    pub fn validate(&self) -> Result<(), ResearchError> {
        let limits = [
            ("max_model_turns", u64::from(self.max_model_turns), 128),
            ("max_searches", u64::from(self.max_searches), 256),
            (
                "max_fetched_pages",
                u64::from(self.max_fetched_pages),
                1_024,
            ),
            ("max_fetched_bytes", self.max_fetched_bytes, 268_435_456),
            ("max_input_tokens", self.max_input_tokens, 10_000_000),
            ("max_output_tokens", self.max_output_tokens, 2_000_000),
            (
                "max_wall_clock_seconds",
                self.max_wall_clock_seconds,
                86_400,
            ),
            ("max_cost_microusd", self.max_cost_microusd, 100_000_000),
            (
                "max_retries_per_call",
                u64::from(self.max_retries_per_call),
                10,
            ),
        ];
        for (name, value, hard_maximum) in limits {
            if value == 0 && name != "max_cost_microusd" && name != "max_retries_per_call" {
                return Err(ResearchError::Validation(format!(
                    "budgets.{name} must be greater than zero"
                )));
            }
            if value > hard_maximum {
                return Err(ResearchError::Validation(format!(
                    "budgets.{name} exceeds the local safety maximum {hard_maximum}"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePolicy {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default)]
    pub allowed_source_classes: Vec<String>,
}

impl SourcePolicy {
    /// Produces the canonical source policy used by every bounded research
    /// capability, independent of the capability's final artifact type.
    pub fn normalize(mut self) -> Result<Self, ResearchError> {
        self.allowed_domains = normalize_domains(self.allowed_domains, "allowed_domains")?;
        self.blocked_domains = normalize_domains(self.blocked_domains, "blocked_domains")?;
        self.allowed_source_classes =
            normalized_list(self.allowed_source_classes, "allowed_source_classes")?;
        if let Some(overlap) = self
            .allowed_domains
            .iter()
            .find(|domain| self.blocked_domains.contains(domain))
        {
            return Err(ResearchError::Validation(format!(
                "source domain {overlap:?} cannot be both allowed and blocked"
            )));
        }
        Ok(self)
    }

    pub fn permits_url(&self, value: &str) -> Result<bool, ResearchError> {
        let url = Url::parse(value)
            .map_err(|error| ResearchError::Validation(format!("invalid source URL: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Ok(false);
        }
        let host = url
            .host_str()
            .ok_or_else(|| ResearchError::Validation("source URL must contain a host".into()))?;
        let blocked = self
            .blocked_domains
            .iter()
            .any(|domain| domain_matches(host, domain));
        if blocked {
            return Ok(false);
        }
        Ok(self.allowed_domains.is_empty()
            || self
                .allowed_domains
                .iter()
                .any(|domain| domain_matches(host, domain)))
    }
}

fn normalize_domains(values: Vec<String>, path: &str) -> Result<Vec<String>, ResearchError> {
    let mut domains = BTreeSet::new();
    for value in values {
        let value = nonempty(value, path)?.to_ascii_lowercase();
        if value.contains('/') || value.contains(':') || value.starts_with('.') {
            return Err(ResearchError::Validation(format!(
                "{path} entry {value:?} must be a bare domain"
            )));
        }
        domains.insert(value);
    }
    Ok(domains.into_iter().collect())
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host.eq_ignore_ascii_case(domain)
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchProviderConfiguration {
    pub runtime: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
}

impl ResearchProviderConfiguration {
    fn normalize(mut self) -> Result<Self, ResearchError> {
        self.runtime = nonempty(self.runtime, "provider.runtime")?.to_ascii_lowercase();
        if self.runtime != RESEARCH_RUNTIME {
            return Err(ResearchError::Validation(format!(
                "provider.runtime must be {RESEARCH_RUNTIME:?}"
            )));
        }
        self.provider = nonempty(self.provider, "provider.provider")?;
        self.model = nonempty(self.model, "provider.model")?;
        self.api_key_env = optional(self.api_key_env, "provider.api_key_env")?;
        if self.api_key_env.as_ref().is_some_and(|name| {
            !name.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
        }) {
            return Err(ResearchError::Validation(
                "provider.api_key_env must name an uppercase environment variable".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchBriefDraft {
    pub schema_version: u32,
    pub dataset: ArtifactReference,
    pub task: String,
    pub labels: Vec<String>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub semantic_context: Option<ArtifactReference>,
    pub target: ResearchTarget,
    pub questions: Vec<String>,
    pub desired_source_diversity: u32,
    pub source_policy: SourcePolicy,
    pub budgets: ResearchBudgets,
    pub provider: ResearchProviderConfiguration,
    pub required_profile_sections: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedResearchBrief {
    pub id: Uuid,
    pub schema_version: u32,
    pub dataset: ArtifactReference,
    pub task: String,
    pub labels: Vec<String>,
    pub dimensions: BTreeMap<String, Vec<String>>,
    pub semantic_context: Option<ArtifactReference>,
    pub target: ResearchTarget,
    pub questions: Vec<String>,
    pub desired_source_diversity: u32,
    pub source_policy: SourcePolicy,
    pub budgets: ResearchBudgets,
    pub provider: ResearchProviderConfiguration,
    pub required_profile_sections: Vec<String>,
    pub resolved_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ResolvedResearchBrief {
    pub fn create(draft: ResearchBriefDraft) -> Result<Self, ResearchError> {
        if draft.schema_version != RESEARCH_BRIEF_SCHEMA_VERSION {
            return Err(ResearchError::Validation(format!(
                "unsupported research brief schema_version {}; expected {RESEARCH_BRIEF_SCHEMA_VERSION}",
                draft.schema_version
            )));
        }
        let dataset = draft.dataset.normalize("dataset")?;
        let task = nonempty(draft.task, "task")?;
        let labels = normalized_list(draft.labels, "labels")?;
        if labels.is_empty() {
            return Err(ResearchError::Validation(
                "labels must contain at least one value".into(),
            ));
        }
        let mut dimensions = BTreeMap::new();
        for (name, values) in draft.dimensions {
            let name = nonempty(name, "dimension name")?;
            let values = normalized_list(values, &format!("dimensions.{name}"))?;
            if values.is_empty() {
                return Err(ResearchError::Validation(format!(
                    "dimensions.{name} must contain at least one value"
                )));
            }
            dimensions.insert(name, values);
        }
        let semantic_context = draft
            .semantic_context
            .map(|reference| reference.normalize("semantic_context"))
            .transpose()?;
        let questions = normalized_list(draft.questions, "questions")?;
        if questions.is_empty() {
            return Err(ResearchError::Validation(
                "questions must contain at least one research question".into(),
            ));
        }
        if draft.desired_source_diversity == 0 || draft.desired_source_diversity > 100 {
            return Err(ResearchError::Validation(
                "desired_source_diversity must be between 1 and 100".into(),
            ));
        }
        draft.budgets.validate()?;
        let required_profile_sections =
            normalized_list(draft.required_profile_sections, "required_profile_sections")?;
        if required_profile_sections.is_empty() {
            return Err(ResearchError::Validation(
                "required_profile_sections must not be empty".into(),
            ));
        }
        let mut brief = Self {
            id: Uuid::new_v4(),
            schema_version: draft.schema_version,
            dataset,
            task,
            labels,
            dimensions,
            semantic_context,
            target: draft.target.normalize()?,
            questions,
            desired_source_diversity: draft.desired_source_diversity,
            source_policy: draft.source_policy.normalize()?,
            budgets: draft.budgets,
            provider: draft.provider.normalize()?,
            required_profile_sections,
            resolved_at: Utc::now(),
            fingerprint: String::new(),
        };
        brief.fingerprint = brief.reproduce_fingerprint()?;
        Ok(brief)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ResearchError> {
        fingerprint(&(
            self.id,
            self.schema_version,
            &self.dataset,
            &self.task,
            &self.labels,
            &self.dimensions,
            &self.semantic_context,
            &self.target,
            &self.questions,
            self.desired_source_diversity,
            &self.source_policy,
            &self.budgets,
            &self.provider,
            &self.required_profile_sections,
            self.resolved_at,
        ))
        .map_err(|error| ResearchError::Fingerprint(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ResearchBriefDraft {
        ResearchBriefDraft {
            schema_version: 1,
            dataset: ArtifactReference {
                id: Uuid::new_v4(),
                fingerprint: "sha256:dataset".into(),
            },
            task: "Classify support messages".into(),
            labels: vec!["billing".into(), "billing".into(), "fraud".into()],
            dimensions: BTreeMap::from([("style".into(), vec!["messy".into()])]),
            semantic_context: None,
            target: ResearchTarget {
                language: "English".into(),
                channel: Some("support chat".into()),
                ..ResearchTarget::default()
            },
            questions: vec!["How do real messages express urgency?".into()],
            desired_source_diversity: 3,
            source_policy: SourcePolicy {
                allowed_domains: vec!["Example.COM".into()],
                blocked_domains: vec!["private.example.com".into()],
                allowed_source_classes: vec!["public_forum".into()],
            },
            budgets: ResearchBudgets {
                max_model_turns: 10,
                max_searches: 10,
                max_fetched_pages: 20,
                max_fetched_bytes: 1_000_000,
                max_input_tokens: 100_000,
                max_output_tokens: 20_000,
                max_wall_clock_seconds: 600,
                max_cost_microusd: 1_000_000,
                max_retries_per_call: 2,
            },
            provider: ResearchProviderConfiguration {
                runtime: "PI".into(),
                provider: "openai".into(),
                model: "example-model".into(),
                api_key_env: Some("RESEARCH_API_KEY".into()),
            },
            required_profile_sections: vec!["language".into(), "noise".into()],
        }
    }

    #[test]
    fn resolves_normalized_immutable_brief() {
        let brief = ResolvedResearchBrief::create(draft()).expect("brief");
        assert_eq!(brief.labels, ["billing", "fraud"]);
        assert_eq!(brief.source_policy.allowed_domains, ["example.com"]);
        assert_eq!(brief.provider.runtime, "pi");
        assert_eq!(brief.reproduce_fingerprint().unwrap(), brief.fingerprint);
    }

    #[test]
    fn source_policy_handles_subdomains_and_block_overrides() {
        let brief = ResolvedResearchBrief::create(draft()).expect("brief");
        assert!(
            brief
                .source_policy
                .permits_url("https://docs.example.com/a")
                .unwrap()
        );
        assert!(
            !brief
                .source_policy
                .permits_url("https://private.example.com/a")
                .unwrap()
        );
        assert!(!brief.source_policy.permits_url("file:///secret").unwrap());
    }

    #[test]
    fn rejects_secret_value_masquerading_as_environment_name() {
        let mut input = draft();
        input.provider.api_key_env = Some("sk-secret".into());
        assert!(matches!(
            ResolvedResearchBrief::create(input),
            Err(ResearchError::Validation(message)) if message.contains("environment variable")
        ));
    }

    #[test]
    fn rejects_unbounded_brief() {
        let mut input = draft();
        input.budgets.max_model_turns = 0;
        assert!(matches!(
            ResolvedResearchBrief::create(input),
            Err(ResearchError::Validation(message)) if message.contains("max_model_turns")
        ));
    }
}
