//! Deterministic, dependency-aware construction of hybrid synthetic rows.

use std::collections::{BTreeMap, BTreeSet};

use artifact_core::{FingerprintError, fingerprint};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use thiserror::Error;

use crate::domain::{DatasetDefinition, GeneratedCandidate, GenerationCell};

pub const CONSTRUCTION_PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldValueType {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
    Any,
}

impl FieldValueType {
    pub fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
            Self::Any => !value.is_null(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightedValue {
    pub value: Value,
    #[serde(default = "default_weight")]
    pub weight: u64,
}

const fn default_weight() -> u64 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldTransform {
    Lowercase,
    Uppercase,
    Trim,
    Length,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldRecipe {
    Fixed {
        value: Value,
    },
    CellLabel,
    CellDimension {
        name: String,
    },
    Sequence {
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        start: i64,
        #[serde(default = "default_step")]
        step: i64,
        width: Option<usize>,
    },
    WeightedChoice {
        choices: Vec<WeightedValue>,
    },
    IntegerRange {
        min: i64,
        max: i64,
    },
    Pattern {
        pattern: String,
    },
    Template {
        template: String,
    },
    Lookup {
        source: String,
        cases: BTreeMap<String, Value>,
        default: Option<Value>,
    },
    Transform {
        source: String,
        operation: FieldTransform,
    },
    Llm {
        instruction: String,
    },
}

const fn default_step() -> i64 {
    1
}

impl FieldRecipe {
    pub const fn is_llm(&self) -> bool {
        matches!(self, Self::Llm { .. })
    }

    pub const fn source_kind(&self) -> FieldSourceKind {
        if self.is_llm() {
            FieldSourceKind::Llm
        } else {
            FieldSourceKind::Deterministic
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Fixed { .. } => "fixed",
            Self::CellLabel => "cell_label",
            Self::CellDimension { .. } => "cell_dimension",
            Self::Sequence { .. } => "sequence",
            Self::WeightedChoice { .. } => "weighted_choice",
            Self::IntegerRange { .. } => "integer_range",
            Self::Pattern { .. } => "pattern",
            Self::Template { .. } => "template",
            Self::Lookup { .. } => "lookup",
            Self::Transform { .. } => "transform",
            Self::Llm { .. } => "llm",
        }
    }

    fn dependencies(&self) -> Result<BTreeSet<String>, ConstructionError> {
        match self {
            Self::Template { template }
            | Self::Llm {
                instruction: template,
            } => template_dependencies(template),
            Self::Lookup { source, .. } | Self::Transform { source, .. } => {
                Ok(BTreeSet::from([source.clone()]))
            }
            _ => Ok(BTreeSet::new()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDefinition {
    pub name: String,
    pub value_type: FieldValueType,
    pub recipe: FieldRecipe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowConstructionPlan {
    pub version: u32,
    pub seed: u64,
    pub fields: Vec<FieldDefinition>,
    pub fingerprint: String,
}

impl RowConstructionPlan {
    pub fn new(seed: u64, fields: Vec<FieldDefinition>) -> Result<Self, ConstructionError> {
        let fields = validate_and_normalize_fields(fields)?;
        compile_order(&fields)?;
        let mut plan = Self {
            version: CONSTRUCTION_PLAN_VERSION,
            seed,
            fields,
            fingerprint: String::new(),
        };
        plan.fingerprint = plan.reproduce_fingerprint()?;
        Ok(plan)
    }

    pub fn llm_text_default() -> Result<Self, ConstructionError> {
        Self::new(
            0,
            vec![FieldDefinition {
                name: "text".into(),
                value_type: FieldValueType::String,
                recipe: FieldRecipe::Llm {
                    instruction: "Generate the classification input text.".into(),
                },
            }],
        )
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, ConstructionError> {
        Ok(fingerprint(&(self.version, self.seed, &self.fields))?)
    }

    pub fn compile(&self) -> Result<CompiledRowConstructionPlan, ConstructionError> {
        if self.version != CONSTRUCTION_PLAN_VERSION {
            return Err(ConstructionError::UnsupportedVersion(self.version));
        }
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(ConstructionError::FingerprintMismatch);
        }
        Ok(CompiledRowConstructionPlan {
            plan: self.clone(),
            order: compile_order(&self.fields)?,
        })
    }

    pub fn field(&self, name: &str) -> Option<&FieldDefinition> {
        self.fields.iter().find(|field| field.name == name)
    }

    pub fn validate_for_dataset(
        &self,
        dataset: &DatasetDefinition,
    ) -> Result<(), ConstructionError> {
        let dimensions = dataset
            .dimensions
            .iter()
            .map(|dimension| dimension.name.as_str())
            .collect::<BTreeSet<_>>();
        for field in &self.fields {
            if let FieldRecipe::CellDimension { name } = &field.recipe {
                if !dimensions.contains(name.as_str()) {
                    return Err(ConstructionError::UnknownDimension(name.clone()));
                }
            }
            for dependency in field.recipe.dependencies()? {
                if let Some(name) = dependency.strip_prefix("dimension.") {
                    if !dimensions.contains(name) {
                        return Err(ConstructionError::UnknownDimension(name.into()));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn verify_trace(
        &self,
        text: &str,
        fields: &BTreeMap<String, Value>,
        trace: &RowConstructionTrace,
    ) -> Result<(), ConstructionError> {
        let trace_names = trace.fields.keys().collect::<BTreeSet<_>>();
        let plan_names = self
            .fields
            .iter()
            .map(|field| &field.name)
            .collect::<BTreeSet<_>>();
        if trace.plan_fingerprint != self.fingerprint
            || trace_names != plan_names
            || trace.reproduce_fingerprint()? != trace.fingerprint
        {
            return Err(ConstructionError::InvalidTrace(
                "trace header, field set, or fingerprint differs".into(),
            ));
        }
        for field in &self.fields {
            let value = if field.name == "text" {
                Value::String(text.to_owned())
            } else {
                fields.get(&field.name).cloned().ok_or_else(|| {
                    ConstructionError::InvalidTrace(format!(
                        "row is missing constructed field {}",
                        field.name
                    ))
                })?
            };
            if !field.value_type.accepts(&value) {
                return Err(ConstructionError::ValueTypeMismatch(field.name.clone()));
            }
            let field_trace = trace.fields.get(&field.name).ok_or_else(|| {
                ConstructionError::InvalidTrace(format!(
                    "trace is missing constructed field {}",
                    field.name
                ))
            })?;
            if field_trace.source != field.recipe.source_kind()
                || field_trace.recipe != field.recipe.name()
                || fingerprint(&value)? != field_trace.value_fingerprint
            {
                return Err(ConstructionError::InvalidTrace(format!(
                    "field trace does not reproduce for {}",
                    field.name
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CompiledRowConstructionPlan {
    plan: RowConstructionPlan,
    order: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmFieldRequest {
    pub name: String,
    pub value_type: FieldValueType,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowConstructionSeed {
    pub row_index: u64,
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedConstructionBatch {
    pub plan_fingerprint: String,
    pub target: GenerationCell,
    pub start_index: u64,
    pub rows: Vec<RowConstructionSeed>,
    pub llm_fields: Vec<LlmFieldRequest>,
}

impl PreparedConstructionBatch {
    pub fn requires_llm(&self) -> bool {
        !self.llm_fields.is_empty()
    }

    pub fn requested_count(&self) -> u32 {
        self.rows.len().try_into().unwrap_or(u32::MAX)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSourceKind {
    Deterministic,
    Llm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldConstructionTrace {
    pub source: FieldSourceKind,
    pub recipe: String,
    pub value_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowConstructionTrace {
    pub plan_fingerprint: String,
    pub row_index: u64,
    pub fields: BTreeMap<String, FieldConstructionTrace>,
    pub fingerprint: String,
}

impl RowConstructionTrace {
    pub fn reproduce_fingerprint(&self) -> Result<String, ConstructionError> {
        Ok(fingerprint(&(
            &self.plan_fingerprint,
            self.row_index,
            &self.fields,
        ))?)
    }
}

impl CompiledRowConstructionPlan {
    pub fn plan(&self) -> &RowConstructionPlan {
        &self.plan
    }

    pub fn prepare(
        &self,
        target: GenerationCell,
        start_index: u64,
        count: u32,
    ) -> Result<PreparedConstructionBatch, ConstructionError> {
        if count == 0 {
            return Err(ConstructionError::EmptyBatch);
        }
        let mut rows = Vec::with_capacity(count as usize);
        for offset in 0..u64::from(count) {
            let row_index = start_index
                .checked_add(offset)
                .ok_or(ConstructionError::RowIndexOverflow)?;
            let mut values = BTreeMap::new();
            self.evaluate_available(&target, row_index, &mut values)?;
            rows.push(RowConstructionSeed {
                row_index,
                fields: values,
            });
        }
        let llm_fields = self
            .plan
            .fields
            .iter()
            .filter_map(|field| match &field.recipe {
                FieldRecipe::Llm { instruction } => Some(LlmFieldRequest {
                    name: field.name.clone(),
                    value_type: field.value_type,
                    instruction: instruction.clone(),
                }),
                _ => None,
            })
            .collect();
        Ok(PreparedConstructionBatch {
            plan_fingerprint: self.plan.fingerprint.clone(),
            target,
            start_index,
            rows,
            llm_fields,
        })
    }

    pub fn complete(
        &self,
        prepared: &PreparedConstructionBatch,
        provider_rows: Vec<GeneratedCandidate>,
    ) -> Result<Vec<GeneratedCandidate>, ConstructionError> {
        if prepared.plan_fingerprint != self.plan.fingerprint {
            return Err(ConstructionError::PreparedPlanMismatch);
        }
        let row_count = if prepared.requires_llm() {
            prepared.rows.len().min(provider_rows.len())
        } else {
            prepared.rows.len()
        };
        let mut completed = Vec::with_capacity(row_count);
        for (index, seed) in prepared.rows.iter().take(row_count).enumerate() {
            let provider = prepared
                .requires_llm()
                .then(|| provider_rows.get(index))
                .flatten();
            let mut values = seed.fields.clone();
            if let Some(provider) = provider {
                for field in &prepared.llm_fields {
                    let value = if field.name == "text" {
                        (!provider.text.is_empty()).then(|| Value::String(provider.text.clone()))
                    } else {
                        provider.fields.get(&field.name).cloned()
                    };
                    if let Some(value) = value {
                        values.insert(field.name.clone(), value);
                    }
                }
            }
            self.evaluate_available(&prepared.target, seed.row_index, &mut values)?;
            let text = values
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let fields = values
                .iter()
                .filter(|(name, _)| name.as_str() != "text")
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            let mut trace_fields = BTreeMap::new();
            for field in &self.plan.fields {
                if let Some(value) = values.get(&field.name) {
                    trace_fields.insert(
                        field.name.clone(),
                        FieldConstructionTrace {
                            source: field.recipe.source_kind(),
                            recipe: field.recipe.name().into(),
                            value_fingerprint: fingerprint(value)?,
                        },
                    );
                }
            }
            let mut trace = RowConstructionTrace {
                plan_fingerprint: self.plan.fingerprint.clone(),
                row_index: seed.row_index,
                fields: trace_fields,
                fingerprint: String::new(),
            };
            trace.fingerprint = trace.reproduce_fingerprint()?;
            completed.push(GeneratedCandidate {
                text,
                label: prepared.target.label.clone(),
                dimensions: prepared.target.dimensions.clone(),
                fields,
                construction: Some(trace),
            });
        }
        Ok(completed)
    }

    fn evaluate_available(
        &self,
        target: &GenerationCell,
        row_index: u64,
        values: &mut BTreeMap<String, Value>,
    ) -> Result<(), ConstructionError> {
        for &field_index in &self.order {
            let field = &self.plan.fields[field_index];
            if values.contains_key(&field.name) || field.recipe.is_llm() {
                continue;
            }
            if field.recipe.dependencies()?.iter().any(|dependency| {
                !is_context_reference(dependency) && !values.contains_key(dependency)
            }) {
                continue;
            }
            let value = evaluate_recipe(&self.plan, field, target, row_index, values)?;
            values.insert(field.name.clone(), value);
        }
        Ok(())
    }
}

fn validate_and_normalize_fields(
    fields: Vec<FieldDefinition>,
) -> Result<Vec<FieldDefinition>, ConstructionError> {
    if fields.is_empty() {
        return Err(ConstructionError::NoFields);
    }
    let mut names = BTreeSet::new();
    let mut normalized = Vec::with_capacity(fields.len());
    for mut field in fields {
        field.name = field.name.trim().to_owned();
        if field.name.is_empty() {
            return Err(ConstructionError::InvalidFieldName(field.name));
        }
        if matches!(field.name.as_str(), "label" | "dimensions")
            || field.name.starts_with("dimensions.")
        {
            return Err(ConstructionError::ReservedField(field.name));
        }
        if !names.insert(field.name.clone()) {
            return Err(ConstructionError::DuplicateField(field.name));
        }
        validate_recipe(&field)?;
        normalized.push(field);
    }
    let text = normalized
        .iter()
        .find(|field| field.name == "text")
        .ok_or(ConstructionError::MissingText)?;
    if text.value_type != FieldValueType::String {
        return Err(ConstructionError::TextMustBeString);
    }
    let known = normalized
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    for field in &normalized {
        for dependency in field.recipe.dependencies()? {
            if !is_context_reference(&dependency) && !known.contains(dependency.as_str()) {
                return Err(ConstructionError::UnknownDependency {
                    field: field.name.clone(),
                    dependency,
                });
            }
        }
    }
    Ok(normalized)
}

fn validate_recipe(field: &FieldDefinition) -> Result<(), ConstructionError> {
    match &field.recipe {
        FieldRecipe::Fixed { value } if !field.value_type.accepts(value) => {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::CellLabel | FieldRecipe::CellDimension { .. }
            if field.value_type != FieldValueType::String =>
        {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::Sequence { step: 0, .. } => Err(ConstructionError::InvalidRecipe(
            field.name.clone(),
            "sequence step must not be zero".into(),
        )),
        FieldRecipe::Sequence { .. } if field.value_type != FieldValueType::String => {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::WeightedChoice { choices }
            if choices.is_empty() || choices.iter().all(|choice| choice.weight == 0) =>
        {
            Err(ConstructionError::InvalidRecipe(
                field.name.clone(),
                "weighted choice needs a positive-weight value".into(),
            ))
        }
        FieldRecipe::WeightedChoice { choices }
            if choices
                .iter()
                .any(|choice| !field.value_type.accepts(&choice.value)) =>
        {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::IntegerRange { min, max } if min > max => {
            Err(ConstructionError::InvalidRecipe(
                field.name.clone(),
                "integer range min exceeds max".into(),
            ))
        }
        FieldRecipe::IntegerRange { .. } if field.value_type != FieldValueType::Integer => {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::Pattern { pattern } | FieldRecipe::Template { template: pattern }
            if pattern.trim().is_empty() =>
        {
            Err(ConstructionError::InvalidRecipe(
                field.name.clone(),
                "pattern/template must not be empty".into(),
            ))
        }
        FieldRecipe::Pattern { .. } | FieldRecipe::Template { .. }
            if field.value_type != FieldValueType::String =>
        {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::Lookup { cases, default, .. }
            if cases
                .values()
                .chain(default.iter())
                .any(|value| !field.value_type.accepts(value)) =>
        {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::Transform { operation, .. }
            if matches!(operation, FieldTransform::Length)
                && field.value_type != FieldValueType::Integer =>
        {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::Transform { operation, .. }
            if !matches!(operation, FieldTransform::Length)
                && field.value_type != FieldValueType::String =>
        {
            Err(ConstructionError::ValueTypeMismatch(field.name.clone()))
        }
        FieldRecipe::Llm { instruction } if instruction.trim().is_empty() => {
            Err(ConstructionError::InvalidRecipe(
                field.name.clone(),
                "LLM instruction must not be empty".into(),
            ))
        }
        _ => Ok(()),
    }
}

fn compile_order(fields: &[FieldDefinition]) -> Result<Vec<usize>, ConstructionError> {
    let indexes = fields
        .iter()
        .enumerate()
        .map(|(index, field)| (field.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut remaining = (0..fields.len()).collect::<BTreeSet<_>>();
    let mut resolved = BTreeSet::new();
    let mut order = Vec::with_capacity(fields.len());
    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .copied()
            .filter(|index| {
                fields[*index]
                    .recipe
                    .dependencies()
                    .is_ok_and(|dependencies| {
                        dependencies.iter().all(|dependency| {
                            is_context_reference(dependency)
                                || indexes.get(dependency.as_str()).is_some_and(
                                    |dependency_index| resolved.contains(dependency_index),
                                )
                        })
                    })
            })
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(ConstructionError::DependencyCycle(
                remaining
                    .iter()
                    .map(|index| fields[*index].name.clone())
                    .collect(),
            ));
        }
        for index in ready {
            remaining.remove(&index);
            resolved.insert(index);
            order.push(index);
        }
    }
    Ok(order)
}

fn evaluate_recipe(
    plan: &RowConstructionPlan,
    field: &FieldDefinition,
    target: &GenerationCell,
    row_index: u64,
    values: &BTreeMap<String, Value>,
) -> Result<Value, ConstructionError> {
    let value = match &field.recipe {
        FieldRecipe::Fixed { value } => value.clone(),
        FieldRecipe::CellLabel => Value::String(target.label.clone()),
        FieldRecipe::CellDimension { name } => Value::String(
            target
                .dimensions
                .get(name)
                .cloned()
                .ok_or_else(|| ConstructionError::MissingDimension(name.clone()))?,
        ),
        FieldRecipe::Sequence {
            prefix,
            start,
            step,
            width,
        } => {
            let offset =
                i64::try_from(row_index).map_err(|_| ConstructionError::RowIndexOverflow)?;
            let number = start
                .checked_add(
                    step.checked_mul(offset)
                        .ok_or(ConstructionError::SequenceOverflow)?,
                )
                .ok_or(ConstructionError::SequenceOverflow)?;
            let rendered =
                width.map_or_else(|| number.to_string(), |width| format!("{number:0width$}"));
            Value::String(format!("{prefix}{rendered}"))
        }
        FieldRecipe::WeightedChoice { choices } => {
            let total = choices
                .iter()
                .try_fold(0_u64, |sum, choice| sum.checked_add(choice.weight))
                .ok_or_else(|| {
                    ConstructionError::InvalidRecipe(
                        field.name.clone(),
                        "choice weights overflow".into(),
                    )
                })?;
            let mut selected =
                deterministic_u64(plan.seed, target, row_index, &field.name, "choice") % total;
            let choice = choices
                .iter()
                .find(|choice| {
                    if selected < choice.weight {
                        true
                    } else {
                        selected -= choice.weight;
                        false
                    }
                })
                .ok_or_else(|| {
                    ConstructionError::InvalidRecipe(
                        field.name.clone(),
                        "could not select weighted value".into(),
                    )
                })?;
            choice.value.clone()
        }
        FieldRecipe::IntegerRange { min, max } => {
            let span = u64::try_from(i128::from(*max) - i128::from(*min) + 1)
                .map_err(|_| ConstructionError::RangeTooLarge(field.name.clone()))?;
            let offset =
                deterministic_u64(plan.seed, target, row_index, &field.name, "range") % span;
            let value = i128::from(*min) + i128::from(offset);
            Value::Number(Number::from(
                i64::try_from(value)
                    .map_err(|_| ConstructionError::RangeTooLarge(field.name.clone()))?,
            ))
        }
        FieldRecipe::Pattern { pattern } => Value::String(render_pattern(
            pattern,
            plan.seed,
            target,
            row_index,
            &field.name,
        )?),
        FieldRecipe::Template { template } => {
            Value::String(render_template(template, target, row_index, values)?)
        }
        FieldRecipe::Lookup {
            source,
            cases,
            default,
        } => {
            let key = resolve_reference(source, target, row_index, values)?;
            cases
                .get(&value_to_text(&key))
                .cloned()
                .or_else(|| default.clone())
                .ok_or_else(|| ConstructionError::LookupMiss {
                    field: field.name.clone(),
                    key: value_to_text(&key),
                })?
        }
        FieldRecipe::Transform { source, operation } => {
            let source = resolve_reference(source, target, row_index, values)?;
            match operation {
                FieldTransform::Lowercase => Value::String(value_to_text(&source).to_lowercase()),
                FieldTransform::Uppercase => Value::String(value_to_text(&source).to_uppercase()),
                FieldTransform::Trim => Value::String(value_to_text(&source).trim().to_owned()),
                FieldTransform::Length => Value::Number(Number::from(
                    u64::try_from(value_to_text(&source).chars().count())
                        .map_err(|_| ConstructionError::RangeTooLarge(field.name.clone()))?,
                )),
            }
        }
        FieldRecipe::Llm { .. } => {
            return Err(ConstructionError::UnresolvedLlmField(field.name.clone()));
        }
    };
    if !field.value_type.accepts(&value) {
        return Err(ConstructionError::ValueTypeMismatch(field.name.clone()));
    }
    Ok(value)
}

fn deterministic_u64(
    seed: u64,
    target: &GenerationCell,
    row_index: u64,
    field: &str,
    purpose: &str,
) -> u64 {
    let digest = fingerprint(&(seed, target, row_index, field, purpose))
        .unwrap_or_else(|_| "sha256:0000000000000000".into());
    u64::from_str_radix(&digest[7..23], 16).unwrap_or(0)
}

fn render_pattern(
    pattern: &str,
    seed: u64,
    target: &GenerationCell,
    row_index: u64,
    field: &str,
) -> Result<String, ConstructionError> {
    let mut output = String::new();
    let mut rest = pattern;
    let mut token_index = 0_u64;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| ConstructionError::InvalidPattern(pattern.into()))?;
        let token = &after[..end];
        let (kind, count) = token
            .split_once(':')
            .map_or((token, 1_usize), |(kind, count)| {
                (kind, count.parse::<usize>().unwrap_or(0))
            });
        if count == 0 || !matches!(kind, "digit" | "alpha" | "alnum") {
            return Err(ConstructionError::InvalidPattern(pattern.into()));
        }
        let alphabet = match kind {
            "digit" => b"0123456789".as_slice(),
            "alpha" => b"ABCDEFGHIJKLMNOPQRSTUVWXYZ".as_slice(),
            _ => b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".as_slice(),
        };
        for position in 0..count {
            let value = deterministic_u64(
                seed,
                target,
                row_index,
                field,
                &format!("pattern:{token_index}:{position}"),
            );
            output.push(char::from(alphabet[value as usize % alphabet.len()]));
        }
        token_index = token_index.saturating_add(1);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

fn template_dependencies(template: &str) -> Result<BTreeSet<String>, ConstructionError> {
    let mut dependencies = BTreeSet::new();
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| ConstructionError::InvalidTemplate(template.into()))?;
        let reference = after[..end].trim();
        if reference.is_empty() {
            return Err(ConstructionError::InvalidTemplate(template.into()));
        }
        dependencies.insert(reference.to_owned());
        rest = &after[end + 1..];
    }
    Ok(dependencies)
}

fn render_template(
    template: &str,
    target: &GenerationCell,
    row_index: u64,
    values: &BTreeMap<String, Value>,
) -> Result<String, ConstructionError> {
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| ConstructionError::InvalidTemplate(template.into()))?;
        output.push_str(&value_to_text(&resolve_reference(
            after[..end].trim(),
            target,
            row_index,
            values,
        )?));
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

fn resolve_reference(
    reference: &str,
    target: &GenerationCell,
    row_index: u64,
    values: &BTreeMap<String, Value>,
) -> Result<Value, ConstructionError> {
    match reference {
        "label" => Ok(Value::String(target.label.clone())),
        "row_index" => Ok(Value::Number(Number::from(row_index))),
        reference if reference.starts_with("dimension.") => target
            .dimensions
            .get(&reference[10..])
            .cloned()
            .map(Value::String)
            .ok_or_else(|| ConstructionError::MissingDimension(reference[10..].into())),
        reference => values
            .get(reference)
            .cloned()
            .ok_or_else(|| ConstructionError::UnresolvedDependency(reference.into())),
    }
}

fn is_context_reference(reference: &str) -> bool {
    matches!(reference, "label" | "row_index") || reference.starts_with("dimension.")
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

#[derive(Debug, Error)]
pub enum ConstructionError {
    #[error("row construction plan contains no fields")]
    NoFields,
    #[error("row construction plan must define text")]
    MissingText,
    #[error("the text field must have string type")]
    TextMustBeString,
    #[error("invalid construction field name: {0:?}")]
    InvalidFieldName(String),
    #[error("reserved construction field: {0}")]
    ReservedField(String),
    #[error("duplicate construction field: {0}")]
    DuplicateField(String),
    #[error("construction field {0} produced or declares the wrong value type")]
    ValueTypeMismatch(String),
    #[error("invalid recipe for construction field {0}: {1}")]
    InvalidRecipe(String, String),
    #[error("construction field {field} references unknown dependency {dependency}")]
    UnknownDependency { field: String, dependency: String },
    #[error("construction field dependency cycle: {0:?}")]
    DependencyCycle(Vec<String>),
    #[error("unsupported row construction plan version: {0}")]
    UnsupportedVersion(u32),
    #[error("row construction plan fingerprint does not reproduce")]
    FingerprintMismatch,
    #[error("row construction batch must not be empty")]
    EmptyBatch,
    #[error("row construction index overflowed")]
    RowIndexOverflow,
    #[error("row construction sequence overflowed")]
    SequenceOverflow,
    #[error("integer range for field {0} is too large")]
    RangeTooLarge(String),
    #[error("generation cell is missing dimension {0}")]
    MissingDimension(String),
    #[error("construction plan references unknown dataset dimension {0}")]
    UnknownDimension(String),
    #[error("invalid deterministic pattern: {0}")]
    InvalidPattern(String),
    #[error("invalid construction template: {0}")]
    InvalidTemplate(String),
    #[error("unresolved construction dependency: {0}")]
    UnresolvedDependency(String),
    #[error("lookup for field {field} has no case for {key:?}")]
    LookupMiss { field: String, key: String },
    #[error("LLM field remained unresolved: {0}")]
    UnresolvedLlmField(String),
    #[error("prepared construction batch belongs to another plan")]
    PreparedPlanMismatch,
    #[error("invalid row construction trace: {0}")]
    InvalidTrace(String),
    #[error(transparent)]
    Fingerprint(#[from] FingerprintError),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{
        FieldDefinition, FieldRecipe, FieldSourceKind, FieldTransform, FieldValueType,
        RowConstructionPlan, WeightedValue,
    };
    use crate::domain::{GeneratedCandidate, GenerationCell};

    fn target() -> GenerationCell {
        GenerationCell {
            label: "billing".into(),
            dimensions: BTreeMap::from([("style".into(), "messy".into())]),
        }
    }

    #[test]
    fn compiles_dependencies_and_constructs_deterministic_fields() {
        let plan = RowConstructionPlan::new(
            7,
            vec![
                field(
                    "text",
                    FieldValueType::String,
                    FieldRecipe::Template {
                        template: "ticket ${ticket_id} is ${category}".into(),
                    },
                ),
                field(
                    "ticket_id",
                    FieldValueType::String,
                    FieldRecipe::Sequence {
                        prefix: "T-".into(),
                        start: 10,
                        step: 2,
                        width: Some(4),
                    },
                ),
                field("category", FieldValueType::String, FieldRecipe::CellLabel),
                field(
                    "style_copy",
                    FieldValueType::String,
                    FieldRecipe::CellDimension {
                        name: "style".into(),
                    },
                ),
                field(
                    "country",
                    FieldValueType::String,
                    FieldRecipe::WeightedChoice {
                        choices: vec![
                            WeightedValue {
                                value: json!("DE"),
                                weight: 3,
                            },
                            WeightedValue {
                                value: json!("US"),
                                weight: 1,
                            },
                        ],
                    },
                ),
                field(
                    "age",
                    FieldValueType::Integer,
                    FieldRecipe::IntegerRange { min: 1, max: 20 },
                ),
                field(
                    "code",
                    FieldValueType::String,
                    FieldRecipe::Pattern {
                        pattern: "C-{digit:4}-{alpha:2}".into(),
                    },
                ),
                field(
                    "upper",
                    FieldValueType::String,
                    FieldRecipe::Transform {
                        source: "country".into(),
                        operation: FieldTransform::Uppercase,
                    },
                ),
            ],
        )
        .expect("plan");
        let compiled = plan.compile().expect("compile");
        let prepared = compiled.prepare(target(), 3, 2).expect("prepare");
        assert!(!prepared.requires_llm());
        let rows = compiled.complete(&prepared, vec![]).expect("complete");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "ticket T-0016 is billing");
        assert_eq!(rows[1].fields["ticket_id"], json!("T-0018"));
        assert_eq!(rows[0].label, "billing");
        assert_eq!(rows[0].dimensions["style"], "messy");
        assert!(
            rows[0]
                .construction
                .as_ref()
                .expect("trace")
                .fingerprint
                .starts_with("sha256:")
        );
        plan.verify_trace(
            &rows[0].text,
            &rows[0].fields,
            rows[0].construction.as_ref().expect("trace"),
        )
        .expect("trace reproduces from row values");
        let mut tampered = rows[0].clone();
        tampered.fields.insert("country".into(), json!("FR"));
        assert!(
            plan.verify_trace(
                &tampered.text,
                &tampered.fields,
                tampered.construction.as_ref().expect("trace"),
            )
            .is_err()
        );
    }

    #[test]
    fn merges_only_requested_llm_fields_then_resolves_dependents() {
        let plan = RowConstructionPlan::new(
            11,
            vec![
                field(
                    "text",
                    FieldValueType::String,
                    FieldRecipe::Llm {
                        instruction: "Write a support message using the deterministic context."
                            .into(),
                    },
                ),
                field(
                    "ticket_id",
                    FieldValueType::String,
                    FieldRecipe::Sequence {
                        prefix: "T".into(),
                        start: 1,
                        step: 1,
                        width: None,
                    },
                ),
                field(
                    "normalized",
                    FieldValueType::String,
                    FieldRecipe::Transform {
                        source: "text".into(),
                        operation: FieldTransform::Lowercase,
                    },
                ),
            ],
        )
        .expect("plan");
        let compiled = plan.compile().expect("compile");
        let prepared = compiled.prepare(target(), 0, 1).expect("prepare");
        assert!(prepared.requires_llm());
        assert_eq!(prepared.rows[0].fields["ticket_id"], json!("T1"));
        let rows = compiled
            .complete(
                &prepared,
                vec![GeneratedCandidate {
                    text: "WHY CHARGED?".into(),
                    label: "wrong and ignored".into(),
                    dimensions: BTreeMap::new(),
                    fields: BTreeMap::new(),
                    construction: None,
                }],
            )
            .expect("complete");
        assert_eq!(rows[0].text, "WHY CHARGED?");
        assert_eq!(rows[0].fields["normalized"], json!("why charged?"));
        assert_eq!(
            rows[0].construction.as_ref().expect("trace").fields["text"].source,
            FieldSourceKind::Llm
        );
    }

    #[test]
    fn rejects_cycles_unknown_dependencies_and_wrong_types() {
        let cycle = RowConstructionPlan::new(
            0,
            vec![
                field(
                    "text",
                    FieldValueType::String,
                    FieldRecipe::Template {
                        template: "${other}".into(),
                    },
                ),
                field(
                    "other",
                    FieldValueType::String,
                    FieldRecipe::Template {
                        template: "${text}".into(),
                    },
                ),
            ],
        );
        assert!(cycle.is_err());
        let unknown = RowConstructionPlan::new(
            0,
            vec![field(
                "text",
                FieldValueType::String,
                FieldRecipe::Template {
                    template: "${missing}".into(),
                },
            )],
        );
        assert!(unknown.is_err());
        let wrong = RowConstructionPlan::new(
            0,
            vec![field(
                "text",
                FieldValueType::String,
                FieldRecipe::Fixed { value: json!(42) },
            )],
        );
        assert!(wrong.is_err());
    }

    fn field(name: &str, value_type: FieldValueType, recipe: FieldRecipe) -> FieldDefinition {
        FieldDefinition {
            name: name.into(),
            value_type,
            recipe,
        }
    }
}
