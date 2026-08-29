//! Provider-neutral prompt and generation-request construction.

use serde_json::json;

use crate::domain::{
    DatasetDefinition, GeneratedCandidate, GenerationCell, GenerationParameters, GenerationRequest,
};

#[derive(Debug, Clone, Default)]
pub struct PromptBuilder;

impl PromptBuilder {
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

        GenerationRequest {
            system_prompt: concat!(
                "You generate synthetic text-classification examples. ",
                "Return only one valid JSON object with a 'rows' array. ",
                "Every row must match the requested label and dimensions exactly. ",
                "Create diverse examples and do not include Markdown fences or commentary."
            )
            .to_owned(),
            user_prompt: format!(
                "Task:\n{}\n\nGenerate exactly {} rows for this target:\n{}\n\nOutput schema:\n{}\n\nAvoid duplicating these existing examples:\n{}",
                definition.task_description,
                requested_count,
                serde_json::to_string_pretty(&target).unwrap_or_default(),
                serde_json::to_string_pretty(&schema).unwrap_or_default(),
                existing,
            ),
            target,
            requested_count,
            parameters,
        }
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
        let request = PromptBuilder.build(
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
}
