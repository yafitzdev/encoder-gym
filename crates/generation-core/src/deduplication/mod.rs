//! Replaceable exact and normalized-text duplicate detection.

use std::collections::HashSet;

pub fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[derive(Debug, Clone, Default)]
pub struct NormalizedTextDeduplicator {
    seen: HashSet<String>,
}

impl NormalizedTextDeduplicator {
    pub fn from_normalized<I>(existing: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        Self {
            seen: existing.into_iter().collect(),
        }
    }

    pub fn check_and_record(&mut self, text: &str) -> bool {
        self.seen.insert(normalize_text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::{NormalizedTextDeduplicator, normalize_text};

    #[test]
    fn normalization_ignores_case_and_whitespace_runs() {
        assert_eq!(normalize_text("  Charged   TWICE\n"), "charged twice");
    }

    #[test]
    fn deduplicator_records_first_occurrence_only() {
        let mut deduplicator = NormalizedTextDeduplicator::default();
        assert!(deduplicator.check_and_record("Charged twice"));
        assert!(!deduplicator.check_and_record(" charged   TWICE "));
    }
}
