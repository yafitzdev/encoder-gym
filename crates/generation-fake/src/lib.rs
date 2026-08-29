//! Deterministic generation backend for local development and tests.

use std::sync::atomic::{AtomicU64, Ordering};

use generation_core::{
    domain::{GeneratedCandidate, GenerationRequest, GenerationResult},
    ports::{BoxFuture, GenerationBackend, GenerationBackendError},
};
use serde_json::json;

#[derive(Debug)]
pub struct FakeGenerationBackend {
    model: String,
    sequence: AtomicU64,
}

impl Default for FakeGenerationBackend {
    fn default() -> Self {
        Self {
            model: "deterministic-v1".to_owned(),
            sequence: AtomicU64::new(0),
        }
    }
}

impl GenerationBackend for FakeGenerationBackend {
    fn name(&self) -> &str {
        "fake"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn generate(
        &self,
        request: GenerationRequest,
    ) -> BoxFuture<'_, Result<GenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            let rows = (0..request.requested_count)
                .map(|_| {
                    let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
                    GeneratedCandidate {
                        text: format!(
                            "Synthetic example {sequence} for label {}",
                            request.target.label
                        ),
                        label: request.target.label.clone(),
                        dimensions: request.target.dimensions.clone(),
                    }
                })
                .collect();

            Ok(GenerationResult {
                rows,
                usage: None,
                backend_metadata: json!({"deterministic": true}),
                errors: vec![],
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use generation_core::{
        domain::{GenerationCell, GenerationParameters, GenerationRequest},
        ports::GenerationBackend,
    };

    use super::FakeGenerationBackend;

    #[tokio::test]
    async fn backend_is_deterministic_and_respects_the_target() {
        let backend = FakeGenerationBackend::default();
        let result = backend
            .generate(GenerationRequest {
                system_prompt: "system".into(),
                user_prompt: "user".into(),
                target: GenerationCell {
                    label: "billing".into(),
                    dimensions: BTreeMap::from([("style".into(), "messy".into())]),
                },
                requested_count: 2,
                parameters: GenerationParameters::default(),
            })
            .await
            .expect("fake generation succeeds");

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0].label, "billing");
        assert_eq!(result.rows[0].dimensions["style"], "messy");
        assert_ne!(result.rows[0].text, result.rows[1].text);
    }
}
