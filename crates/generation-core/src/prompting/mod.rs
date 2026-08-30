//! Provider-neutral prompt and generation-request construction.

use artifact_core::{FingerprintError, fingerprint};
use semantic_catalog::{ResolvedSemanticContext, SemanticTarget};
use serde_json::{Value, json};

use crate::construction::PreparedConstructionBatch;
use crate::domain::{
    DatasetDefinition, GeneratedCandidate, GenerationCell, GenerationParameters, GenerationRequest,
};
use crate::jobs::PromptTemplateIdentity;

const PROMPT_TEMPLATE_NAME: &str = "text-classification-json";
const PROMPT_TEMPLATE_VERSION: u32 = 1;
const SYSTEM_PROMPT: &str = concat!(
    "You generate synthetic text-classification examples. ",
    "Return only one valid JSON object with a 'rows' array. ",
    "Every row must match the requested label and dimensions exactly. ",
    "Create diverse examples and do not include Markdown fences or commentary."
);
const USER_PROMPT_TEMPLATE: &str = concat!(
    "Task:\n{task}\n\nGenerate exactly {requested_count} rows for this target:\n{target}",
    "\n\n{semantic_section}\n\nOutput schema:\n{schema}",
    "\n\nAvoid duplicating these existing examples:\n{existing_examples}"
);
const HYBRID_PROMPT_TEMPLATE_NAME: &str = "hybrid-row-json";
const HYBRID_PROMPT_TEMPLATE_VERSION: u32 = 2;
const HYBRID_SYSTEM_PROMPT: &str = concat!(
    "You fill only the explicitly requested semantic fields in synthetic rows. ",
    "Return one valid JSON object with a 'rows' array in exactly the supplied row order. ",
    "Do not repeat, modify, or infer fields already supplied by the deterministic construction engine. ",
    "Do not include Markdown fences or commentary."
);
const HYBRID_USER_PROMPT_TEMPLATE: &str = concat!(
    "Task:\n{task}\n\nTarget:\n{target}\n\n{semantic_section}",
    "\n\nDeterministic row seeds:\n{row_seeds}",
    "\n\nFill only these unresolved fields:\n{llm_fields}",
    "\n\nReturn exactly {requested_count} rows using this response shape:\n{schema}",
    "\n\nAvoid duplicating these existing examples:\n{existing_examples}"
);

#[derive(Debug, Clone, Default)]
pub struct PromptBuilder {
    semantics: Option<ResolvedSemanticContext>,
}

impl PromptBuilder {
    pub fn template_identity() -> Result<PromptTemplateIdentity, FingerprintError> {
        Ok(PromptTemplateIdentity {
            name: HYBRID_PROMPT_TEMPLATE_NAME.into(),
            version: HYBRID_PROMPT_TEMPLATE_VERSION,
            fingerprint: fingerprint(&(HYBRID_SYSTEM_PROMPT, HYBRID_USER_PROMPT_TEMPLATE))?,
        })
    }

    pub fn legacy_template_identity() -> Result<PromptTemplateIdentity, FingerprintError> {
        Ok(PromptTemplateIdentity {
            name: PROMPT_TEMPLATE_NAME.into(),
            version: PROMPT_TEMPLATE_VERSION,
            fingerprint: fingerprint(&(SYSTEM_PROMPT, USER_PROMPT_TEMPLATE))?,
        })
    }

    pub fn with_semantics(semantics: ResolvedSemanticContext) -> Self {
        Self {
            semantics: Some(semantics),
        }
    }

    pub fn semantic_context(&self) -> Option<&ResolvedSemanticContext> {
        self.semantics.as_ref()
    }

    pub fn build(
        &self,
        definition: &DatasetDefinition,
        target: GenerationCell,
        requested_count: u32,
        parameters: GenerationParameters,
        existing_examples: &[GeneratedCandidate],
    ) -> GenerationRequest {
        let schema = json!({
            "rows": [{
                "text": "string",
                "label": target.label,
                "dimensions": target.dimensions,
            }]
        });
        let existing =
            serde_json::to_string_pretty(existing_examples).unwrap_or_else(|_| "[]".to_owned());

        let semantic_guidance = self.guidance_for_target(&target);
        let mut system_prompt = SYSTEM_PROMPT.to_owned();
        if semantic_guidance.is_some() {
            system_prompt.push_str(
                " The supplied semantic guidance is authoritative for interpreting the target values.",
            );
        }
        let semantic_section = semantic_guidance.map_or_else(
            || "No additional semantic guidance is attached; infer meaning from the task and schema names.".to_owned(),
            |guidance| {
                format!(
                    "Attached semantic guidance (resolved and version-pinned):\n{}",
                    serde_json::to_string_pretty(&guidance).unwrap_or_default()
                )
            },
        );

        GenerationRequest {
            system_prompt,
            user_prompt: format!(
                "Task:\n{}\n\nGenerate exactly {} rows for this target:\n{}\n\n{}\n\nOutput schema:\n{}\n\nAvoid duplicating these existing examples:\n{}",
                definition.task_description,
                requested_count,
                serde_json::to_string_pretty(&target).unwrap_or_default(),
                semantic_section,
                serde_json::to_string_pretty(&schema).unwrap_or_default(),
                existing,
            ),
            target,
            requested_count,
            parameters,
            construction: None,
        }
    }

    pub fn build_hybrid(
        &self,
        definition: &DatasetDefinition,
        prepared: PreparedConstructionBatch,
        parameters: GenerationParameters,
        existing_examples: &[GeneratedCandidate],
    ) -> GenerationRequest {
        let requested_count = prepared.requested_count();
        let target = prepared.target.clone();
        let llm_fields = prepared
            .llm_fields
            .iter()
            .map(|field| {
                json!({
                    "name": field.name,
                    "value_type": field.value_type,
                    "instruction": field.instruction,
                })
            })
            .collect::<Vec<_>>();
        let mut row_shape = serde_json::Map::new();
        if prepared.llm_fields.iter().any(|field| field.name == "text") {
            row_shape.insert("text".into(), Value::String("string".into()));
        }
        let custom = prepared
            .llm_fields
            .iter()
            .filter(|field| field.name != "text")
            .map(|field| (field.name.clone(), json!(field.value_type)))
            .collect::<serde_json::Map<_, _>>();
        if !custom.is_empty() {
            row_shape.insert("fields".into(), Value::Object(custom));
        }
        let schema = json!({"rows": [Value::Object(row_shape)]});
        let existing =
            serde_json::to_string_pretty(existing_examples).unwrap_or_else(|_| "[]".to_owned());
        let semantic_guidance = self.guidance_for_target(&target);
        let mut system_prompt = HYBRID_SYSTEM_PROMPT.to_owned();
        if semantic_guidance.is_some() {
            system_prompt.push_str(
                " The supplied semantic guidance is authoritative for interpreting the target values.",
            );
        }
        let semantic_section = semantic_guidance.map_or_else(
            || "No additional semantic guidance is attached; infer meaning from the task and schema names.".to_owned(),
            |guidance| {
                format!(
                    "Attached semantic guidance (resolved and version-pinned):\n{}",
                    serde_json::to_string_pretty(&guidance).unwrap_or_default()
                )
            },
        );
        let user_prompt = HYBRID_USER_PROMPT_TEMPLATE
            .replace("{task}", &definition.task_description)
            .replace(
                "{target}",
                &serde_json::to_string_pretty(&target).unwrap_or_default(),
            )
            .replace("{semantic_section}", &semantic_section)
            .replace(
                "{row_seeds}",
                &serde_json::to_string_pretty(&prepared.rows).unwrap_or_default(),
            )
            .replace(
                "{llm_fields}",
                &serde_json::to_string_pretty(&llm_fields).unwrap_or_default(),
            )
            .replace("{requested_count}", &requested_count.to_string())
            .replace(
                "{schema}",
                &serde_json::to_string_pretty(&schema).unwrap_or_default(),
            )
            .replace("{existing_examples}", &existing);

        GenerationRequest {
            system_prompt,
            user_prompt,
            target,
            requested_count,
            parameters,
            construction: Some(prepared),
        }
    }

    fn guidance_for_target(&self, target: &GenerationCell) -> Option<serde_json::Value> {
        let context = self.semantics.as_ref()?;
        let mut targets = serde_json::Map::new();
        let label_target = SemanticTarget::Labels;
        if let Some(resolved) = context.target(&label_target) {
            targets.insert(
                label_target.key(),
                json!({
                    "description": resolved.description,
                    "selected_value": target.label,
                    "value_semantics": resolved.entries.get(&target.label),
                }),
            );
        }
        for (name, value) in &target.dimensions {
            let semantic_target = SemanticTarget::Dimension { name: name.clone() };
            if let Some(resolved) = context.target(&semantic_target) {
                targets.insert(
                    semantic_target.key(),
                    json!({
                        "description": resolved.description,
                        "selected_value": value,
                        "value_semantics": resolved.entries.get(value),
                    }),
                );
            }
        }
        if targets.is_empty() {
            return None;
        }
        Some(json!({
            "context_fingerprint": context.fingerprint,
            "targets": targets,
            "sources": context.sources,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::PromptBuilder;
    use crate::{
        construction::{FieldDefinition, FieldRecipe, FieldValueType, RowConstructionPlan},
        dimensions::expand_generation_cells,
        domain::{DatasetDefinition, DimensionDefinition, GenerationParameters},
    };

    #[test]
    fn hybrid_prompt_requests_only_unresolved_fields_and_carries_row_seeds() {
        let dataset = DatasetDefinition::new(
            "support",
            "Classify requests",
            vec!["billing".into()],
            vec![],
        )
        .expect("dataset");
        let construction = RowConstructionPlan::new(
            3,
            vec![
                FieldDefinition {
                    name: "text".into(),
                    value_type: FieldValueType::String,
                    recipe: FieldRecipe::Llm {
                        instruction: "Generate the request text.".into(),
                    },
                },
                FieldDefinition {
                    name: "ticket_id".into(),
                    value_type: FieldValueType::String,
                    recipe: FieldRecipe::Sequence {
                        prefix: "T-".into(),
                        start: 1,
                        step: 1,
                        width: Some(3),
                    },
                },
            ],
        )
        .expect("plan")
        .compile()
        .expect("compiled");
        let prepared = construction
            .prepare(expand_generation_cells(&dataset)[0].clone(), 0, 2)
            .expect("prepared");
        let request = PromptBuilder::default().build_hybrid(
            &dataset,
            prepared,
            GenerationParameters::default(),
            &[],
        );

        assert!(request.user_prompt.contains("T-001"));
        assert!(request.user_prompt.contains("Generate the request text."));
        let response_contract = request
            .user_prompt
            .split("Return exactly")
            .nth(1)
            .expect("response contract");
        assert!(response_contract.contains("\"text\""));
        assert!(!response_contract.contains("\"label\""));
        assert!(!response_contract.contains("\"dimensions\""));
        assert_eq!(
            request
                .construction
                .expect("prepared batch")
                .llm_fields
                .len(),
            1
        );
    }

    #[test]
    fn prompt_contains_task_target_and_count_without_backend_details() {
        let dataset = DatasetDefinition::new(
            "support",
            "Classify customer support requests",
            vec!["billing".into()],
            vec![DimensionDefinition::new("style", vec!["messy".into()]).expect("valid dimension")],
        )
        .expect("valid dataset");
        let request = PromptBuilder::default().build(
            &dataset,
            expand_generation_cells(&dataset)[0].clone(),
            25,
            GenerationParameters::default(),
            &[],
        );

        assert!(
            request
                .user_prompt
                .contains("Classify customer support requests")
        );
        assert!(request.user_prompt.contains("25"));
        assert!(request.user_prompt.contains("messy"));
        assert!(!request.user_prompt.contains("OpenAI"));
    }

    #[test]
    fn prompt_includes_only_semantics_relevant_to_the_cell() {
        use std::collections::BTreeMap;

        use chrono::Utc;
        use semantic_catalog::{ResolvedSemanticContext, ResolvedSemanticTarget, SemanticEntry};

        let dataset = DatasetDefinition::new(
            "support",
            "Classify requests",
            vec!["billing".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
        )
        .expect("dataset");
        let mut context = ResolvedSemanticContext {
            dataset_id: dataset.id,
            targets: BTreeMap::from([(
                "dimension:difficulty".into(),
                ResolvedSemanticTarget {
                    description: Some("Reasoning burden".into()),
                    entries: BTreeMap::from([
                        (
                            "easy".into(),
                            SemanticEntry {
                                description: Some("Direct clue".into()),
                                ..SemanticEntry::default()
                            },
                        ),
                        (
                            "hard".into(),
                            SemanticEntry {
                                description: Some("Multiple indirect clues".into()),
                                ..SemanticEntry::default()
                            },
                        ),
                    ]),
                },
            )]),
            sources: vec![],
            resolved_at: Utc::now(),
            fingerprint: String::new(),
        };
        context.fingerprint = context.reproduce_fingerprint().expect("fingerprint");
        let request = PromptBuilder::with_semantics(context).build(
            &dataset,
            expand_generation_cells(&dataset)[0].clone(),
            1,
            GenerationParameters::default(),
            &[],
        );
        assert!(request.user_prompt.contains("Reasoning burden"));
        assert!(request.user_prompt.contains("Direct clue"));
        assert!(!request.user_prompt.contains("Multiple indirect clues"));
        assert!(request.system_prompt.contains("authoritative"));
    }
}
