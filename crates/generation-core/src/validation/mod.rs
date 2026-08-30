//! Small, composable generated-row validators.

use crate::{
    construction::RowConstructionPlan,
    domain::{DatasetDefinition, GeneratedCandidate, GenerationCell},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub issues: Vec<ValidationIssue>,
}

pub struct ValidationContext<'a> {
    pub dataset: &'a DatasetDefinition,
    pub target: &'a GenerationCell,
    pub construction_plan: Option<&'a RowConstructionPlan>,
}

pub trait RowValidator: Send + Sync {
    fn validate(
        &self,
        context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> Vec<ValidationIssue>;
}

#[derive(Default)]
pub struct ValidationPipeline {
    validators: Vec<Box<dyn RowValidator>>,
}

impl ValidationPipeline {
    pub fn new(validators: Vec<Box<dyn RowValidator>>) -> Self {
        Self { validators }
    }

    pub fn standard(text_length: Option<TextLengthValidator>) -> Self {
        let mut validators: Vec<Box<dyn RowValidator>> = vec![
            Box::new(NonEmptyTextValidator),
            Box::new(ValidLabelValidator),
            Box::new(TargetDimensionsValidator),
            Box::new(ConstructionFieldsValidator),
        ];
        if let Some(validator) = text_length {
            validators.push(Box::new(validator));
        }
        Self::new(validators)
    }

    pub fn validate(
        &self,
        context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> ValidationResult {
        let issues = self
            .validators
            .iter()
            .flat_map(|validator| validator.validate(context, candidate))
            .collect::<Vec<_>>();
        ValidationResult {
            is_valid: issues.is_empty(),
            issues,
        }
    }
}

pub struct ConstructionFieldsValidator;

impl RowValidator for ConstructionFieldsValidator {
    fn validate(
        &self,
        context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> Vec<ValidationIssue> {
        let Some(plan) = context.construction_plan else {
            return vec![];
        };
        let mut issues = Vec::new();
        let expected = plan
            .fields
            .iter()
            .filter(|field| field.name != "text")
            .map(|field| field.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let actual = candidate
            .fields
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        for missing in expected.difference(&actual) {
            issues.push(issue(
                "missing_constructed_field",
                format!("missing constructed field {missing}"),
            ));
        }
        for unexpected in actual.difference(&expected) {
            issues.push(issue(
                "unexpected_constructed_field",
                format!("unexpected constructed field {unexpected}"),
            ));
        }
        for field in &plan.fields {
            let value = if field.name == "text" {
                serde_json::Value::String(candidate.text.clone())
            } else if let Some(value) = candidate.fields.get(&field.name) {
                value.clone()
            } else {
                continue;
            };
            if !field.value_type.accepts(&value) {
                issues.push(issue(
                    "constructed_field_type",
                    format!("constructed field {} has the wrong value type", field.name),
                ));
            }
        }
        match &candidate.construction {
            Some(trace) => {
                if plan
                    .verify_trace(&candidate.text, &candidate.fields, trace)
                    .is_err()
                {
                    issues.push(issue(
                        "invalid_construction_trace",
                        "row construction trace does not reproduce from the pinned plan and row values",
                    ));
                }
            }
            None => issues.push(issue(
                "missing_construction_trace",
                "row is missing construction provenance",
            )),
        }
        issues
    }
}

pub struct NonEmptyTextValidator;

impl RowValidator for NonEmptyTextValidator {
    fn validate(
        &self,
        _context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> Vec<ValidationIssue> {
        if candidate.text.trim().is_empty() {
            vec![issue("empty_text", "text must not be empty")]
        } else {
            vec![]
        }
    }
}

pub struct ValidLabelValidator;

impl RowValidator for ValidLabelValidator {
    fn validate(
        &self,
        context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> Vec<ValidationIssue> {
        if !context.dataset.labels.contains(&candidate.label) {
            vec![issue(
                "invalid_label",
                format!("unknown label: {}", candidate.label),
            )]
        } else if candidate.label != context.target.label {
            vec![issue(
                "wrong_target_label",
                format!(
                    "expected label {}, got {}",
                    context.target.label, candidate.label
                ),
            )]
        } else {
            vec![]
        }
    }
}

pub struct TargetDimensionsValidator;

impl RowValidator for TargetDimensionsValidator {
    fn validate(
        &self,
        context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        if candidate.dimensions != context.target.dimensions {
            issues.push(issue(
                "wrong_target_dimensions",
                "dimensions do not exactly match the requested generation cell",
            ));
        }

        for definition in &context.dataset.dimensions {
            match candidate.dimensions.get(&definition.name) {
                Some(value) if definition.values.contains(value) => {}
                Some(value) => issues.push(issue(
                    "invalid_dimension_value",
                    format!("invalid value {value} for dimension {}", definition.name),
                )),
                None => issues.push(issue(
                    "missing_dimension",
                    format!("missing dimension {}", definition.name),
                )),
            }
        }

        issues
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TextLengthValidator {
    pub min_chars: Option<usize>,
    pub max_chars: Option<usize>,
}

impl RowValidator for TextLengthValidator {
    fn validate(
        &self,
        _context: &ValidationContext<'_>,
        candidate: &GeneratedCandidate,
    ) -> Vec<ValidationIssue> {
        let length = candidate.text.chars().count();
        if self.min_chars.is_some_and(|minimum| length < minimum) {
            return vec![issue(
                "text_too_short",
                format!("text contains {length} characters"),
            )];
        }
        if self.max_chars.is_some_and(|maximum| length > maximum) {
            return vec![issue(
                "text_too_long",
                format!("text contains {length} characters"),
            )];
        }
        vec![]
    }
}

fn issue(code: &'static str, message: impl Into<String>) -> ValidationIssue {
    ValidationIssue {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{TextLengthValidator, ValidationContext, ValidationPipeline};
    use crate::domain::{
        DatasetDefinition, DimensionDefinition, GeneratedCandidate, GenerationCell,
    };

    fn fixtures() -> (DatasetDefinition, GenerationCell) {
        let dataset = DatasetDefinition::new(
            "support",
            "classify",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("style", vec!["clean".into(), "messy".into()])
                    .expect("valid dimension"),
            ],
        )
        .expect("valid dataset");
        let target = GenerationCell {
            label: "billing".into(),
            dimensions: BTreeMap::from([("style".into(), "clean".into())]),
        };
        (dataset, target)
    }

    #[test]
    fn pipeline_composes_multiple_independent_issues() {
        let (dataset, target) = fixtures();
        let candidate = GeneratedCandidate {
            text: " ".into(),
            label: "unknown".into(),
            dimensions: BTreeMap::from([("style".into(), "other".into())]),
            fields: BTreeMap::new(),
            construction: None,
        };
        let pipeline = ValidationPipeline::standard(Some(TextLengthValidator {
            min_chars: Some(5),
            max_chars: None,
        }));
        let result = pipeline.validate(
            &ValidationContext {
                dataset: &dataset,
                target: &target,
                construction_plan: None,
            },
            &candidate,
        );

        assert!(!result.is_valid);
        assert!(result.issues.len() >= 4);
        assert!(result.issues.iter().any(|issue| issue.code == "empty_text"));
        assert!(
            result
                .issues
                .iter()
                .any(|issue| issue.code == "invalid_label")
        );
    }

    #[test]
    fn valid_target_row_is_accepted() {
        let (dataset, target) = fixtures();
        let candidate = GeneratedCandidate {
            text: "Why was I charged twice?".into(),
            label: "billing".into(),
            dimensions: target.dimensions.clone(),
            fields: BTreeMap::new(),
            construction: None,
        };
        let result = ValidationPipeline::standard(None).validate(
            &ValidationContext {
                dataset: &dataset,
                target: &target,
                construction_plan: None,
            },
            &candidate,
        );
        assert!(result.is_valid);
    }
}
