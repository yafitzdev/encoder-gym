//! Deterministic generation backend for local development and tests.

use std::sync::atomic::{AtomicU64, Ordering};

use generation_core::{
    construction::FieldValueType,
    domain::{GeneratedCandidate, GenerationRequest, GenerationResult},
    ports::{BoxFuture, GenerationBackend, GenerationBackendError},
};
use serde_json::json;

#[derive(Debug)]
pub struct FakeGenerationBackend {
    model: String,
    namespace: String,
    sequence: AtomicU64,
}

/// Deterministic supervisor fixture. It intentionally emits shortcut-heavy
/// rows until a bounded guidance revision asks for concrete situational
/// detail, allowing offline CLI tests to exercise pause, review, and canary
/// activation without pretending the fixture is a semantic generator.
#[derive(Debug, Default)]
pub struct RepairableFakeGenerationBackend {
    sequence: AtomicU64,
}

impl Default for FakeGenerationBackend {
    fn default() -> Self {
        Self {
            model: "deterministic-v1".to_owned(),
            namespace: "default".to_owned(),
            sequence: AtomicU64::new(0),
        }
    }
}

impl FakeGenerationBackend {
    pub fn with_namespace(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            ..Self::default()
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
                    let fields = request
                        .construction
                        .as_ref()
                        .map(|construction| {
                            construction
                                .llm_fields
                                .iter()
                                .filter(|field| field.name != "text")
                                .map(|field| {
                                    let value = match field.value_type {
                                        FieldValueType::String => json!(format!(
                                            "Synthetic {name} {sequence}",
                                            name = field.name
                                        )),
                                        FieldValueType::Integer => json!(sequence),
                                        FieldValueType::Number => json!(sequence as f64),
                                        FieldValueType::Boolean => json!(sequence % 2 == 0),
                                        FieldValueType::Object => json!({"sequence": sequence}),
                                        FieldValueType::Array => json!([sequence]),
                                        FieldValueType::Any => json!(format!("value-{sequence}")),
                                    };
                                    (field.name.clone(), value)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    GeneratedCandidate {
                        text: format!(
                            "Synthetic example {} {sequence} for label {}",
                            self.namespace, request.target.label
                        ),
                        label: request.target.label.clone(),
                        dimensions: request.target.dimensions.clone(),
                        fields,
                        construction: None,
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

impl GenerationBackend for RepairableFakeGenerationBackend {
    fn name(&self) -> &str {
        "supervised-fake"
    }

    fn model(&self) -> &str {
        "repairable-deterministic-v1"
    }

    fn generate(
        &self,
        request: GenerationRequest,
    ) -> BoxFuture<'_, Result<GenerationResult, GenerationBackendError>> {
        Box::pin(async move {
            let repaired = request
                .user_prompt
                .to_lowercase()
                .contains("concrete situational detail");
            let rows = (0..request.requested_count)
                .map(|_| {
                    let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
                    let text = if repaired {
                        let context = request
                            .target
                            .dimensions
                            .values()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(" ");
                        format!(
                            "I need help with {} after a concrete situation changed this morning {context} {sequence}",
                            request.target.label
                        )
                    } else {
                        format!(
                            "Synthetic example {sequence} for label {}",
                            request.target.label
                        )
                    };
                    GeneratedCandidate {
                        text,
                        label: request.target.label.clone(),
                        dimensions: request.target.dimensions.clone(),
                        fields: Default::default(),
                        construction: None,
                    }
                })
                .collect();
            Ok(GenerationResult {
                rows,
                usage: None,
                backend_metadata: json!({"deterministic": true, "repaired": repaired}),
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
                construction: None,
            })
            .await
            .expect("fake generation succeeds");

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0].label, "billing");
        assert_eq!(result.rows[0].dimensions["style"], "messy");
        assert_ne!(result.rows[0].text, result.rows[1].text);
    }
}
