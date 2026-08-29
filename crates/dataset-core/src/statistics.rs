use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::domain::{SnapshotMember, SnapshotSplit};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotStatistics {
    pub total: u64,
    pub by_split: BTreeMap<SnapshotSplit, u64>,
    pub by_label: BTreeMap<String, u64>,
    pub by_cell: BTreeMap<String, u64>,
}

pub fn calculate_statistics(members: &[SnapshotMember]) -> SnapshotStatistics {
    let mut by_split = SnapshotSplit::ALL
        .into_iter()
        .map(|split| (split, 0))
        .collect::<BTreeMap<_, _>>();
    let mut by_label = BTreeMap::new();
    let mut by_cell = BTreeMap::new();
    for member in members {
        *by_split.entry(member.split).or_default() += 1;
        *by_label.entry(member.label.clone()).or_default() += 1;
        *by_cell.entry(cell_key(member)).or_default() += 1;
    }
    SnapshotStatistics {
        total: members.len() as u64,
        by_split,
        by_label,
        by_cell,
    }
}

pub fn cell_key(member: &SnapshotMember) -> String {
    serde_json::to_string(&json!({
        "label": member.label,
        "dimensions": member.dimensions,
    }))
    .expect("strings and maps always serialize")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use uuid::Uuid;

    use super::calculate_statistics;
    use crate::domain::{SnapshotMember, SnapshotSplit};

    #[test]
    fn statistics_are_derived_from_members() {
        let snapshot_id = Uuid::new_v4();
        let members = [
            member(snapshot_id, "billing", SnapshotSplit::Train, "clean"),
            member(snapshot_id, "billing", SnapshotSplit::Test, "clean"),
            member(snapshot_id, "fraud", SnapshotSplit::Train, "messy"),
        ];
        let statistics = calculate_statistics(&members);
        assert_eq!(statistics.total, 3);
        assert_eq!(statistics.by_split[&SnapshotSplit::Train], 2);
        assert_eq!(statistics.by_split[&SnapshotSplit::Validation], 0);
        assert_eq!(statistics.by_label["billing"], 2);
        assert_eq!(statistics.by_cell.len(), 2);
    }

    fn member(snapshot_id: Uuid, label: &str, split: SnapshotSplit, style: &str) -> SnapshotMember {
        SnapshotMember {
            id: Uuid::new_v4(),
            snapshot_id,
            source_row_id: Uuid::new_v4(),
            split,
            text: "example".into(),
            label: label.into(),
            dimensions: BTreeMap::from([("style".into(), style.into())]),
            source_provenance: crate::domain::SourceProvenance::Generated {
                generation_job_id: Uuid::nil(),
                backend: "fixture".into(),
                model: "fixture-v1".into(),
            },
            source_created_at: Utc::now(),
        }
    }
}
