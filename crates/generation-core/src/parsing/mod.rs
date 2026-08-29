//! Parsing normalized generated rows from backend text output.

use crate::domain::GeneratedCandidate;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("backend returned no content")]
    Empty,
    #[error("backend output was not valid generated-row JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RowEnvelope {
    Object { rows: Vec<GeneratedCandidate> },
    Array(Vec<GeneratedCandidate>),
}

pub fn parse_generated_candidates(raw: &str) -> Result<Vec<GeneratedCandidate>, ParseError> {
    let raw = strip_optional_fence(raw.trim());
    if raw.is_empty() {
        return Err(ParseError::Empty);
    }

    match serde_json::from_str::<RowEnvelope>(raw)? {
        RowEnvelope::Object { rows } | RowEnvelope::Array(rows) => Ok(rows),
    }
}

fn strip_optional_fence(raw: &str) -> &str {
    let Some(after_open) = raw
        .strip_prefix("```json")
        .or_else(|| raw.strip_prefix("```"))
    else {
        return raw;
    };
    after_open.strip_suffix("```").unwrap_or(after_open).trim()
}

#[cfg(test)]
mod tests {
    use super::parse_generated_candidates;

    #[test]
    fn parses_object_envelope() {
        let rows = parse_generated_candidates(
            r#"{"rows":[{"text":"hello","label":"billing","dimensions":{"style":"clean"}}]}"#,
        )
        .expect("valid response");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].dimensions["style"], "clean");
    }

    #[test]
    fn tolerates_a_markdown_fence_from_compatible_backends() {
        let rows = parse_generated_candidates(
            "```json\n[{\"text\":\"hello\",\"label\":\"billing\",\"dimensions\":{}}]\n```",
        )
        .expect("valid response");
        assert_eq!(rows.len(), 1);
    }
}
