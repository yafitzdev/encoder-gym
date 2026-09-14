//! Generation transport for task-owned schemas. Unlike classification generation,
//! the transport does not infer labels or admit native rows. The task adapter
//! constructs prompts and validates the returned content before publication.

use serde::{Deserialize, Serialize};

use crate::{
    domain::UsageMetadata,
    ports::{BoxFuture, GenerationBackendError},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredGenerationRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub maximum_output_tokens: u32,
}

impl StructuredGenerationRequest {
    pub fn validate(&self) -> Result<(), GenerationBackendError> {
        if self.system_prompt.trim().is_empty()
            || self.user_prompt.trim().is_empty()
            || self.system_prompt.len() + self.user_prompt.len() > 262144
            || !(1..=65536).contains(&self.maximum_output_tokens)
        {
            return Err(GenerationBackendError::Configuration(
                "Invalid bounded structured generation request".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredGenerationResult {
    /// Untrusted provider output, retained with usage even if task admission fails.
    pub content: String,
    pub usage: Option<UsageMetadata>,
}

pub trait StructuredGenerationBackend: Send + Sync {
    fn model(&self) -> &str;
    /// Exactly one request; retries require another durable host reservation.
    fn generate_structured(
        &self,
        request: StructuredGenerationRequest,
    ) -> BoxFuture<'_, Result<StructuredGenerationResult, GenerationBackendError>>;
}
