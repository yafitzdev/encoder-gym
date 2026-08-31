//! Provider-neutral prompt and generation-request construction.

use std::collections::BTreeSet;

use artifact_core::{FingerprintError, fingerprint};
use research_core::profile::ResolvedAuthenticityContext;
use semantic_catalog::{ResolvedSemanticContext, SemanticTarget};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::construction::PreparedConstructionBatch;
use crate::domain::{
    DatasetDefinition, GeneratedCandidate, GenerationCell, GenerationParameters, GenerationRequest,
};
use crate::jobs::PromptTemplateIdentity;
use crate::strategy::ResolvedGenerationStrategyContext;

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
const AUTHENTICITY_PROMPT_TEMPLATE_VERSION: u32 = 3;
const STRATEGY_PROMPT_TEMPLATE_VERSION: u32 = 4;
const STRATEGY_PROMPT_SECTION: &str = "Approved per-cell generation strategy (apply proportionally across this cell):\n{strategy_section}";
const SUPERVISED_PROMPT_TEMPLATE_VERSION: u32 = 5;
const SUPERVISED_PROMPT_SECTION: &str = concat!(
    "Supervisor-approved generation guidance for only these row positions:\n",
    "{supervised_section}"
);
const AUTHENTICITY_SYSTEM_PROMPT: &str = concat!(
    "You fill only the explicitly requested semantic fields in synthetic rows. ",
    "Return one valid JSON object with a 'rows' array in exactly the supplied row order. ",
    "Do not repeat, modify, or infer fields already supplied by the deterministic construction engine. ",
    "Do not include Markdown fences or commentary. ",
    "Treat the approved authenticity profile as abstract distributional guidance. ",
    "Never reproduce source wording, quoted examples, personal data, or a recognizable source document."
);
const AUTHENTICITY_USER_PROMPT_TEMPLATE: &str = concat!(
    "Task:\n{task}\n\nTarget:\n{target}\n\n{semantic_section}",
    "\n\nApproved authenticity profile (abstract guidance only):\n{authenticity_section}",
    "\n\nDeterministic row seeds:\n{row_seeds}",
    "\n\nFill only these unresolved fields:\n{llm_fields}",
    "\n\nReturn exactly {requested_count} rows using this response shape:\n{schema}",
    "\n\nAvoid duplicating these existing examples:\n{existing_examples}"
);

#[derive(Debug, Error)]
pub enum PromptBuildError {
    #[error("supervised prompt schedule is invalid: {0}")]
    InvalidSchedule(String),
    #[error(transparent)]
    Fingerprint(#[from] FingerprintError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisedRowGuidance {
    pub cell_key: String,
    pub row_sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_assignment_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_directive_id: Option<Uuid>,
    #[serde(default)]
    pub strategy_instructions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisedGenerationSchedule {
    pub supervisor_run_id: Uuid,
    pub prompt_version_id: Uuid,
    pub prompt_version_fingerprint: String,
    #[serde(default)]
    pub prompt_guidance: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_assignment_set_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_assignment_set_fingerprint: Option<String>,
    pub rows: Vec<SupervisedRowGuidance>,
    pub fingerprint: String,
}

impl SupervisedGenerationSchedule {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        supervisor_run_id: Uuid,
        prompt_version_id: Uuid,
        prompt_version_fingerprint: impl Into<String>,
        prompt_guidance: Vec<String>,
        strategy_assignment_set: Option<(Uuid, String)>,
        rows: Vec<SupervisedRowGuidance>,
    ) -> Result<Self, PromptBuildError> {
        let (strategy_assignment_set_id, strategy_assignment_set_fingerprint) =
            strategy_assignment_set.unzip();
        let mut value = Self {
            supervisor_run_id,
            prompt_version_id,
            prompt_version_fingerprint: prompt_version_fingerprint.into(),
            prompt_guidance: normalize_guidance(prompt_guidance),
            strategy_assignment_set_id,
            strategy_assignment_set_fingerprint,
            rows,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, PromptBuildError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        Ok(fingerprint(&value)?)
    }

    pub fn validate(&self) -> Result<(), PromptBuildError> {
        self.validate_fields()?;
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(PromptBuildError::InvalidSchedule(
                "schedule fingerprint does not reproduce".into(),
            ));
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), PromptBuildError> {
        if self.supervisor_run_id.is_nil()
            || self.prompt_version_id.is_nil()
            || self.prompt_version_fingerprint.trim().is_empty()
            || self.rows.is_empty()
            || self.strategy_assignment_set_id.is_some()
                != self.strategy_assignment_set_fingerprint.is_some()
        {
            return Err(PromptBuildError::InvalidSchedule(
                "run, prompt, row, and optional strategy identities must be complete".into(),
            ));
        }
        if self
            .strategy_assignment_set_fingerprint
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(PromptBuildError::InvalidSchedule(
                "strategy assignment-set fingerprint must not be empty".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        for row in &self.rows {
            if row.cell_key.trim().is_empty()
                || !keys.insert((row.cell_key.clone(), row.row_sequence))
                || row
                    .strategy_assignment_fingerprint
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                || row
                    .strategy_instructions
                    .iter()
                    .any(|value| value.trim().is_empty())
                || (row.strategy_directive_id.is_some()
                    && row.strategy_assignment_fingerprint.is_none())
                || (row.strategy_directive_id.is_none() && !row.strategy_instructions.is_empty())
            {
                return Err(PromptBuildError::InvalidSchedule(
                    "row guidance must be unique, normalized, and fully bound".into(),
                ));
            }
        }
        Ok(())
    }

    fn for_batch(
        &self,
        target: &GenerationCell,
        start_index: u64,
        requested_count: u32,
    ) -> Result<Vec<&SupervisedRowGuidance>, PromptBuildError> {
        self.validate()?;
        let start = u32::try_from(start_index)
            .map_err(|_| PromptBuildError::InvalidSchedule("row sequence exceeds u32".into()))?;
        let mut rows = Vec::with_capacity(requested_count as usize);
        for offset in 0..requested_count {
            let sequence = start
                .checked_add(offset)
                .ok_or_else(|| PromptBuildError::InvalidSchedule("row sequence overflow".into()))?;
            rows.push(
                self.rows
                    .iter()
                    .find(|row| row.cell_key == target.key() && row.row_sequence == sequence)
                    .ok_or_else(|| {
                        PromptBuildError::InvalidSchedule(format!(
                            "no guidance for {} row {sequence}",
                            target.key()
                        ))
                    })?,
            );
        }
        Ok(rows)
    }
}

fn normalize_guidance(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct PromptBuilder {
    semantics: Option<ResolvedSemanticContext>,
    authenticity: Option<ResolvedAuthenticityContext>,
    strategy: Option<ResolvedGenerationStrategyContext>,
    supervision: Option<SupervisedGenerationSchedule>,
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

    pub fn authenticity_template_identity() -> Result<PromptTemplateIdentity, FingerprintError> {
        Ok(PromptTemplateIdentity {
            name: HYBRID_PROMPT_TEMPLATE_NAME.into(),
            version: AUTHENTICITY_PROMPT_TEMPLATE_VERSION,
            fingerprint: fingerprint(&(
                AUTHENTICITY_SYSTEM_PROMPT,
                AUTHENTICITY_USER_PROMPT_TEMPLATE,
            ))?,
        })
    }

    pub fn strategy_template_identity(
        with_authenticity: bool,
    ) -> Result<PromptTemplateIdentity, FingerprintError> {
        let base = if with_authenticity {
            (
                AUTHENTICITY_SYSTEM_PROMPT,
                AUTHENTICITY_USER_PROMPT_TEMPLATE,
            )
        } else {
            (HYBRID_SYSTEM_PROMPT, HYBRID_USER_PROMPT_TEMPLATE)
        };
        Ok(PromptTemplateIdentity {
            name: HYBRID_PROMPT_TEMPLATE_NAME.into(),
            version: STRATEGY_PROMPT_TEMPLATE_VERSION,
            fingerprint: fingerprint(&(base, STRATEGY_PROMPT_SECTION))?,
        })
    }

    pub fn supervision_template_identity(
        base: &PromptTemplateIdentity,
    ) -> Result<PromptTemplateIdentity, FingerprintError> {
        Ok(PromptTemplateIdentity {
            name: HYBRID_PROMPT_TEMPLATE_NAME.into(),
            version: SUPERVISED_PROMPT_TEMPLATE_VERSION,
            fingerprint: fingerprint(&(base, SUPERVISED_PROMPT_SECTION))?,
        })
    }

    pub fn with_semantics(semantics: ResolvedSemanticContext) -> Self {
        Self {
            semantics: Some(semantics),
            authenticity: None,
            strategy: None,
            supervision: None,
        }
    }

    pub fn attach_semantics(mut self, semantics: ResolvedSemanticContext) -> Self {
        self.semantics = Some(semantics);
        self
    }

    pub fn attach_authenticity(mut self, authenticity: ResolvedAuthenticityContext) -> Self {
        self.authenticity = Some(authenticity);
        self
    }

    pub fn attach_strategy(mut self, strategy: ResolvedGenerationStrategyContext) -> Self {
        self.strategy = Some(strategy);
        self
    }

    pub fn attach_supervision(mut self, schedule: SupervisedGenerationSchedule) -> Self {
        self.supervision = Some(schedule);
        self
    }

    pub fn semantic_context(&self) -> Option<&ResolvedSemanticContext> {
        self.semantics.as_ref()
    }

    pub fn authenticity_context(&self) -> Option<&ResolvedAuthenticityContext> {
        self.authenticity.as_ref()
    }

    pub fn strategy_context(&self) -> Option<&ResolvedGenerationStrategyContext> {
        self.strategy.as_ref()
    }

    pub fn supervision_schedule(&self) -> Option<&SupervisedGenerationSchedule> {
        self.supervision.as_ref()
    }

    pub fn supervised_row(
        &self,
        target: &GenerationCell,
        row_sequence: u64,
    ) -> Result<Option<&SupervisedRowGuidance>, PromptBuildError> {
        let Some(schedule) = &self.supervision else {
            return Ok(None);
        };
        Ok(Some(schedule.for_batch(target, row_sequence, 1)?[0]))
    }

    pub fn build(
        &self,
        definition: &DatasetDefinition,
        target: GenerationCell,
        requested_count: u32,
        parameters: GenerationParameters,
        existing_examples: &[GeneratedCandidate],
    ) -> GenerationRequest {
        self.build_at(
            definition,
            target,
            requested_count,
            0,
            parameters,
            existing_examples,
        )
        .expect("an unsupervised prompt cannot have a missing schedule row")
    }

    pub fn build_at(
        &self,
        definition: &DatasetDefinition,
        target: GenerationCell,
        requested_count: u32,
        start_index: u64,
        parameters: GenerationParameters,
        existing_examples: &[GeneratedCandidate],
    ) -> Result<GenerationRequest, PromptBuildError> {
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

        let strategy_section = self.strategy_guidance(&target);
        if strategy_section.is_some() {
            system_prompt.push_str(" Follow the approved per-cell strategy while preserving the exact target label and dimensions.");
        }
        let strategy_section = strategy_section.map_or_else(
            || "No approved per-cell generation strategy is attached.".to_owned(),
            |guidance| {
                STRATEGY_PROMPT_SECTION.replace(
                    "{strategy_section}",
                    &serde_json::to_string_pretty(&guidance).unwrap_or_default(),
                )
            },
        );

        let supervised_section = self.supervised_section(&target, start_index, requested_count)?;
        if supervised_section.is_some() {
            system_prompt.push_str(
                " Follow only the supervisor guidance assigned to each exact row position.",
            );
        }
        let supervised_section = supervised_section.map_or_else(String::new, |guidance| {
            format!(
                "\n\n{}",
                SUPERVISED_PROMPT_SECTION.replace(
                    "{supervised_section}",
                    &serde_json::to_string_pretty(&guidance).unwrap_or_default(),
                )
            )
        });

        Ok(GenerationRequest {
            system_prompt,
            user_prompt: format!(
                "Task:\n{}\n\nGenerate exactly {} rows for this target:\n{}\n\n{}\n\n{}{}\n\nOutput schema:\n{}\n\nAvoid duplicating these existing examples:\n{}",
                definition.task_description,
                requested_count,
                serde_json::to_string_pretty(&target).unwrap_or_default(),
                semantic_section,
                strategy_section,
                supervised_section,
                serde_json::to_string_pretty(&schema).unwrap_or_default(),
                existing,
            ),
            target,
            requested_count,
            parameters,
            construction: None,
        })
    }

    pub fn build_hybrid(
        &self,
        definition: &DatasetDefinition,
        prepared: PreparedConstructionBatch,
        parameters: GenerationParameters,
        existing_examples: &[GeneratedCandidate],
    ) -> GenerationRequest {
        self.build_hybrid_at(definition, prepared, 0, parameters, existing_examples)
            .expect("an unsupervised prompt cannot have a missing schedule row")
    }

    pub fn build_hybrid_at(
        &self,
        definition: &DatasetDefinition,
        prepared: PreparedConstructionBatch,
        start_index: u64,
        parameters: GenerationParameters,
        existing_examples: &[GeneratedCandidate],
    ) -> Result<GenerationRequest, PromptBuildError> {
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
        let authenticity_guidance = self.authenticity_guidance();
        let mut system_prompt = if authenticity_guidance.is_some() {
            AUTHENTICITY_SYSTEM_PROMPT.to_owned()
        } else {
            HYBRID_SYSTEM_PROMPT.to_owned()
        };
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
        let authenticity_section = authenticity_guidance
            .map(|guidance| serde_json::to_string_pretty(&guidance).unwrap_or_default());
        let strategy_section = self.strategy_guidance(&target);
        if strategy_section.is_some() {
            system_prompt.push_str(" Follow the approved per-cell strategy while preserving the exact target label and dimensions.");
        }
        let template = if authenticity_section.is_some() {
            AUTHENTICITY_USER_PROMPT_TEMPLATE
        } else {
            HYBRID_USER_PROMPT_TEMPLATE
        };
        let user_prompt = template
            .replace("{task}", &definition.task_description)
            .replace(
                "{target}",
                &serde_json::to_string_pretty(&target).unwrap_or_default(),
            )
            .replace("{semantic_section}", &semantic_section)
            .replace(
                "{authenticity_section}",
                authenticity_section.as_deref().unwrap_or(""),
            )
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
        let mut user_prompt = match strategy_section {
            Some(guidance) => format!(
                "{user_prompt}\n\n{}",
                STRATEGY_PROMPT_SECTION.replace(
                    "{strategy_section}",
                    &serde_json::to_string_pretty(&guidance).unwrap_or_default(),
                )
            ),
            None => user_prompt,
        };
        if let Some(guidance) = self.supervised_section(&target, start_index, requested_count)? {
            system_prompt.push_str(
                " Follow only the supervisor guidance assigned to each exact row position.",
            );
            user_prompt.push_str("\n\n");
            user_prompt.push_str(&SUPERVISED_PROMPT_SECTION.replace(
                "{supervised_section}",
                &serde_json::to_string_pretty(&guidance).unwrap_or_default(),
            ));
        }

        Ok(GenerationRequest {
            system_prompt,
            user_prompt,
            target,
            requested_count,
            parameters,
            construction: Some(prepared),
        })
    }

    fn supervised_section(
        &self,
        target: &GenerationCell,
        start_index: u64,
        requested_count: u32,
    ) -> Result<Option<serde_json::Value>, PromptBuildError> {
        let Some(schedule) = &self.supervision else {
            return Ok(None);
        };
        let rows = schedule.for_batch(target, start_index, requested_count)?;
        Ok(Some(json!({
            "supervisor_run_id": schedule.supervisor_run_id,
            "prompt_version_id": schedule.prompt_version_id,
            "prompt_version_fingerprint": schedule.prompt_version_fingerprint,
            "prompt_guidance": schedule.prompt_guidance,
            "strategy_assignment_set_id": schedule.strategy_assignment_set_id,
            "strategy_assignment_set_fingerprint": schedule.strategy_assignment_set_fingerprint,
            "rows": rows,
        })))
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

    fn authenticity_guidance(&self) -> Option<serde_json::Value> {
        let context = self.authenticity.as_ref()?;
        let sections = context
            .sections
            .iter()
            .map(|(name, section)| {
                (
                    name.clone(),
                    json!({
                        "observations": section.observations,
                        "generation_instructions": section.generation_instructions,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        Some(json!({
            "profile_id": context.profile_id,
            "profile_version": context.profile_version,
            "profile_fingerprint": context.profile_fingerprint,
            "summary": context.summary,
            "sections": sections,
            "generation_instructions": context.generation_instructions,
            "caveats": context.caveats,
        }))
    }

    fn strategy_guidance(&self, target: &GenerationCell) -> Option<serde_json::Value> {
        let context = self.strategy.as_ref()?;
        let directives = context.for_cell(target);
        (!directives.is_empty()).then(|| {
            json!({
                "context_id": context.id,
                "context_fingerprint": context.fingerprint,
                "directives": directives,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use research_core::profile::{AuthenticitySection, ResolvedAuthenticityContext};
    use uuid::Uuid;

    use super::{PromptBuilder, SupervisedGenerationSchedule, SupervisedRowGuidance};
    use crate::{
        construction::{FieldDefinition, FieldRecipe, FieldValueType, RowConstructionPlan},
        dimensions::expand_generation_cells,
        domain::{
            DatasetDefinition, DimensionDefinition, GenerationParameters, GenerationPlan,
            PlannedCell,
        },
        strategy::{GenerationStrategyDirective, ResolvedGenerationStrategyContext},
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
    fn approved_strategy_is_scoped_to_its_exact_generation_cell() {
        let dataset = DatasetDefinition::new(
            "support",
            "Classify requests",
            vec!["billing".into(), "fraud".into()],
            vec![],
        )
        .unwrap();
        let cells = expand_generation_cells(&dataset);
        let plan = GenerationPlan::new(
            dataset.id,
            cells
                .iter()
                .cloned()
                .map(|cell| PlannedCell {
                    cell,
                    target_count: 5,
                })
                .collect(),
        )
        .unwrap();
        let context = ResolvedGenerationStrategyContext::create(
            &dataset,
            &plan,
            Uuid::new_v4(),
            "sha256:proposal".into(),
            Uuid::new_v4(),
            "sha256:approval".into(),
            BTreeMap::from([(
                cells[0].key(),
                vec![GenerationStrategyDirective {
                    source_directive_id: Uuid::new_v4(),
                    kind: "boundary_case".into(),
                    share_basis_points: 2_500,
                    instructions: vec!["Use indirect but decisive billing clues.".into()],
                    related_labels: vec!["fraud".into()],
                    rationale: "Strengthen the class boundary.".into(),
                    confidence: "high".into(),
                }],
            )]),
        )
        .unwrap();
        let builder = PromptBuilder::default().attach_strategy(context);
        let matching = builder.build(
            &dataset,
            cells[0].clone(),
            1,
            GenerationParameters::default(),
            &[],
        );
        let other = builder.build(
            &dataset,
            cells[1].clone(),
            1,
            GenerationParameters::default(),
            &[],
        );
        assert!(
            matching
                .user_prompt
                .contains("indirect but decisive billing clues")
        );
        assert!(
            !other
                .user_prompt
                .contains("indirect but decisive billing clues")
        );
    }

    #[test]
    fn prompt_includes_only_semantics_relevant_to_the_cell() {
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

    #[test]
    fn approved_authenticity_context_changes_the_versioned_prompt_without_source_text() {
        let dataset = DatasetDefinition::new(
            "support",
            "Classify requests",
            vec!["billing".into()],
            vec![],
        )
        .expect("dataset");
        let mut context = ResolvedAuthenticityContext {
            dataset_id: dataset.id,
            dataset_fingerprint: "sha256:dataset".into(),
            binding_id: Uuid::new_v4(),
            binding_fingerprint: "sha256:binding".into(),
            profile_id: Uuid::new_v4(),
            profile_version: 2,
            profile_fingerprint: "sha256:profile".into(),
            summary: "Messages are terse and context-dependent.".into(),
            sections: BTreeMap::from([(
                "language".into(),
                AuthenticitySection {
                    observations: vec!["Fragments are common.".into()],
                    generation_instructions: vec!["Vary sentence completeness.".into()],
                    claim_ids: vec![Uuid::new_v4()],
                },
            )]),
            generation_instructions: vec!["Vary length.".into()],
            caveats: vec!["Small corpus.".into()],
            resolved_at: Utc::now(),
            fingerprint: String::new(),
        };
        context.fingerprint = context.reproduce_fingerprint().expect("fingerprint");
        let construction = RowConstructionPlan::llm_text_default()
            .expect("plan")
            .compile()
            .expect("compiled");
        let prepared = construction
            .prepare(expand_generation_cells(&dataset)[0].clone(), 0, 1)
            .expect("prepared");
        let request = PromptBuilder::default()
            .attach_authenticity(context)
            .build_hybrid(&dataset, prepared, GenerationParameters::default(), &[]);

        assert!(request.user_prompt.contains("Fragments are common."));
        assert!(request.user_prompt.contains("Vary sentence completeness."));
        assert!(!request.user_prompt.contains("claim_ids"));
        assert!(
            request
                .system_prompt
                .contains("Never reproduce source wording")
        );
        assert_ne!(
            PromptBuilder::authenticity_template_identity()
                .expect("authenticity identity")
                .fingerprint,
            PromptBuilder::template_identity()
                .expect("ordinary identity")
                .fingerprint
        );
    }

    #[test]
    fn supervised_prompt_uses_only_the_exact_assigned_row_guidance() {
        let dataset = DatasetDefinition::new(
            "support",
            "Classify requests",
            vec!["billing".into()],
            vec![],
        )
        .expect("dataset");
        let cell = expand_generation_cells(&dataset)[0].clone();
        let directive_id = Uuid::new_v4();
        let schedule = SupervisedGenerationSchedule::create(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "prompt-v2",
            vec!["Write a concrete first-person request.".into()],
            Some((Uuid::new_v4(), "assignment-set".into())),
            vec![
                SupervisedRowGuidance {
                    cell_key: cell.key(),
                    row_sequence: 0,
                    strategy_assignment_fingerprint: Some("assignment-0".into()),
                    strategy_directive_id: Some(directive_id),
                    strategy_instructions: vec!["Use a boundary case.".into()],
                },
                SupervisedRowGuidance {
                    cell_key: cell.key(),
                    row_sequence: 1,
                    strategy_assignment_fingerprint: Some("assignment-1".into()),
                    strategy_directive_id: Some(directive_id),
                    strategy_instructions: vec!["Use plausible input noise.".into()],
                },
            ],
        )
        .expect("schedule");
        let builder = PromptBuilder::default().attach_supervision(schedule);
        let first = builder
            .build_at(
                &dataset,
                cell.clone(),
                1,
                0,
                GenerationParameters::default(),
                &[],
            )
            .expect("first prompt");
        assert!(first.user_prompt.contains("boundary case"));
        assert!(!first.user_prompt.contains("plausible input noise"));
        let second = builder
            .build_at(&dataset, cell, 1, 1, GenerationParameters::default(), &[])
            .expect("second prompt");
        assert!(!second.user_prompt.contains("boundary case"));
        assert!(second.user_prompt.contains("plausible input noise"));
    }
}
