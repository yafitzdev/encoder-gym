//! Provider-neutral prompt and generation-request construction.

use semantic_catalog::{ResolvedSemanticContext, SemanticTarget};
use serde_json::json;

use crate::domain::{
    DatasetDefinition, GeneratedCandidate, GenerationCell, GenerationParameters, GenerationRequest,
};

#[derive(Debug, Clone, Default)]
pub struct PromptBuilder {
    semantics: Option<ResolvedSemanticContext>,
}

impl PromptBuilder {
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
        let mut system_prompt = concat!(
            "You generate synthetic text-classification examples. ",
            "Return only one valid JSON object with a 'rows' array. ",
            "Every row must match the requested label and dimensions exactly. ",
            "Create diverse examples and do not include Markdown fences or commentary."
        )
        .to_owned();
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
        dimensions::expand_generation_cells,
        domain::{DatasetDefinition, DimensionDefinition, GenerationParameters},
    };

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
