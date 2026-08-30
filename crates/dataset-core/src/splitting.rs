use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use uuid::Uuid;

use crate::domain::{
    DatasetError, DatasetSnapshot, SnapshotMember, SnapshotSplit, SourceRow, SplitConfiguration,
};

pub fn build_snapshot(
    source_dataset_id: Uuid,
    name: impl Into<String>,
    description: Option<String>,
    configuration: SplitConfiguration,
    source_rows: Vec<SourceRow>,
) -> Result<(DatasetSnapshot, Vec<SnapshotMember>), DatasetError> {
    if source_rows.is_empty() {
        return Err(DatasetError::EmptySnapshot);
    }
    let mut seen = BTreeSet::new();
    for row in &source_rows {
        if row.dataset_id != source_dataset_id {
            return Err(DatasetError::SourceDatasetMismatch {
                row_id: row.id,
                expected: source_dataset_id,
                actual: row.dataset_id,
            });
        }
        if !seen.insert(row.id) {
            return Err(DatasetError::DuplicateSourceRow(row.id));
        }
    }

    let mut snapshot = DatasetSnapshot::new(
        source_dataset_id,
        name,
        description,
        configuration.clone(),
        source_rows.len() as u64,
    )?;
    let mut members = Vec::with_capacity(snapshot.member_count as usize);
    for (row, split) in split_rows(source_rows, configuration)? {
        members.push(SnapshotMember {
            id: Uuid::new_v4(),
            snapshot_id: snapshot.id,
            source_row_id: row.id,
            split,
            text: row.text,
            label: row.label,
            dimensions: row.dimensions,
            fields: row.fields,
            source_provenance: row.provenance,
            source_created_at: row.created_at,
        });
    }
    members.sort_by_key(|member| member.source_row_id);
    snapshot.fingerprint = snapshot_fingerprint(&snapshot, &members)?;
    Ok((snapshot, members))
}

fn split_rows(
    source_rows: Vec<SourceRow>,
    configuration: SplitConfiguration,
) -> Result<Vec<(SourceRow, SnapshotSplit)>, DatasetError> {
    let Some(group_dimension) = configuration.group_dimension.as_deref() else {
        let mut strata = BTreeMap::<String, Vec<SourceRow>>::new();
        for row in source_rows {
            strata.entry(row.label.clone()).or_default().push(row);
        }
        let mut assignments = Vec::new();
        for rows in strata.values_mut() {
            rows.sort_by_key(|row| (stable_score(configuration.seed, row.id), row.id));
            let counts = allocate_counts(rows.len(), &configuration);
            for (index, row) in rows.drain(..).enumerate() {
                assignments.push((row, split_for_index(index, counts)));
            }
        }
        return Ok(assignments);
    };

    let mut groups = BTreeMap::<String, Vec<SourceRow>>::new();
    for row in source_rows {
        let group = row.dimensions.get(group_dimension).ok_or_else(|| {
            DatasetError::MissingGroupDimension {
                row_id: row.id,
                dimension: group_dimension.to_owned(),
            }
        })?;
        groups.entry(group.clone()).or_default().push(row);
    }
    let total = groups.values().map(Vec::len).sum::<usize>();
    let targets = allocate_counts(total, &configuration);
    let mut ordered = groups.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|(left, left_rows), (right, right_rows)| {
        right_rows.len().cmp(&left_rows.len()).then_with(|| {
            stable_text_score(configuration.seed, left)
                .cmp(&stable_text_score(configuration.seed, right))
                .then_with(|| left.cmp(right))
        })
    });
    let mut assigned = [0_usize; 3];
    let mut assignments = Vec::with_capacity(total);
    for (_, rows) in ordered {
        let index = (0..3)
            .min_by(|left, right| {
                group_split_cost(assigned, targets, *left, rows.len())
                    .cmp(&group_split_cost(assigned, targets, *right, rows.len()))
                    .then_with(|| left.cmp(right))
            })
            .expect("three candidate splits");
        assigned[index] += rows.len();
        let split = split_from_index(index);
        assignments.extend(rows.into_iter().map(|row| (row, split)));
    }
    Ok(assignments)
}

fn group_split_cost(
    assigned: [usize; 3],
    targets: [usize; 3],
    candidate: usize,
    group_size: usize,
) -> (usize, usize) {
    let projected = assigned[candidate] + group_size;
    (
        projected.saturating_sub(targets[candidate]),
        targets[candidate].abs_diff(projected),
    )
}

fn split_for_index(index: usize, counts: [usize; 3]) -> SnapshotSplit {
    if index < counts[0] {
        SnapshotSplit::Train
    } else if index < counts[0] + counts[1] {
        SnapshotSplit::Validation
    } else {
        SnapshotSplit::Test
    }
}

const fn split_from_index(index: usize) -> SnapshotSplit {
    match index {
        0 => SnapshotSplit::Train,
        1 => SnapshotSplit::Validation,
        _ => SnapshotSplit::Test,
    }
}

#[derive(Serialize)]
struct SnapshotFingerprintInput<'a> {
    source_dataset_id: Uuid,
    name: &'a str,
    description: &'a Option<String>,
    split_configuration: SplitConfiguration,
    members: Vec<SnapshotMemberFingerprintInput<'a>>,
}

#[derive(Serialize)]
struct LegacySnapshotFingerprintInput<'a> {
    source_dataset_id: Uuid,
    name: &'a str,
    description: &'a Option<String>,
    split_configuration: SplitConfiguration,
    members: Vec<LegacySnapshotMemberFingerprintInput<'a>>,
}

#[derive(Serialize)]
struct LegacySnapshotMemberFingerprintInput<'a> {
    source_row_id: Uuid,
    split: SnapshotSplit,
    text: &'a str,
    label: &'a str,
    dimensions: &'a BTreeMap<String, String>,
    provenance: &'a crate::domain::SourceProvenance,
    source_created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
struct SnapshotMemberFingerprintInput<'a> {
    source_row_id: Uuid,
    split: SnapshotSplit,
    text: &'a str,
    label: &'a str,
    dimensions: &'a BTreeMap<String, String>,
    fields: &'a BTreeMap<String, serde_json::Value>,
    provenance: &'a crate::domain::SourceProvenance,
    source_created_at: chrono::DateTime<chrono::Utc>,
}

fn snapshot_fingerprint(
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
) -> Result<String, DatasetError> {
    if members.iter().all(|member| member.fields.is_empty()) {
        return artifact_core::fingerprint(&LegacySnapshotFingerprintInput {
            source_dataset_id: snapshot.source_dataset_id,
            name: &snapshot.name,
            description: &snapshot.description,
            split_configuration: snapshot.split_configuration.clone(),
            members: members
                .iter()
                .map(|member| LegacySnapshotMemberFingerprintInput {
                    source_row_id: member.source_row_id,
                    split: member.split,
                    text: &member.text,
                    label: &member.label,
                    dimensions: &member.dimensions,
                    provenance: &member.source_provenance,
                    source_created_at: member.source_created_at,
                })
                .collect(),
        })
        .map_err(|error| DatasetError::Fingerprint(error.to_string()));
    }
    artifact_core::fingerprint(&SnapshotFingerprintInput {
        source_dataset_id: snapshot.source_dataset_id,
        name: &snapshot.name,
        description: &snapshot.description,
        split_configuration: snapshot.split_configuration.clone(),
        members: members
            .iter()
            .map(|member| SnapshotMemberFingerprintInput {
                source_row_id: member.source_row_id,
                split: member.split,
                text: &member.text,
                label: &member.label,
                dimensions: &member.dimensions,
                fields: &member.fields,
                provenance: &member.source_provenance,
                source_created_at: member.source_created_at,
            })
            .collect(),
    })
    .map_err(|error| DatasetError::Fingerprint(error.to_string()))
}

fn allocate_counts(size: usize, configuration: &SplitConfiguration) -> [usize; 3] {
    let ratios = [
        configuration.ratios.train,
        configuration.ratios.validation,
        configuration.ratios.test,
    ];
    let exact = ratios.map(|ratio| ratio * size as f64);
    let mut counts = exact.map(|value| value.floor() as usize);
    let assigned = counts.iter().sum::<usize>();
    let mut order = [0_usize, 1, 2];
    order.sort_by(|left, right| {
        let left_remainder = exact[*left] - counts[*left] as f64;
        let right_remainder = exact[*right] - counts[*right] as f64;
        right_remainder
            .total_cmp(&left_remainder)
            .then_with(|| left.cmp(right))
    });
    for index in order.into_iter().take(size - assigned) {
        counts[index] += 1;
    }
    counts
}

fn stable_score(seed: u64, id: Uuid) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for byte in id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn stable_text_score(seed: u64, value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ seed;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::Utc;
    use uuid::Uuid;

    use super::{
        LegacySnapshotFingerprintInput, LegacySnapshotMemberFingerprintInput, build_snapshot,
    };
    use crate::domain::{SnapshotSplit, SourceRow, SplitConfiguration, SplitRatios};

    fn rows(dataset_id: Uuid) -> Vec<SourceRow> {
        (0..20)
            .map(|index| SourceRow {
                id: Uuid::from_u128(index + 1),
                dataset_id,
                text: format!("example {index}"),
                label: if index < 10 { "billing" } else { "fraud" }.into(),
                dimensions: BTreeMap::from([(
                    "style".into(),
                    if index % 2 == 0 { "clean" } else { "messy" }.into(),
                )]),
                fields: BTreeMap::new(),
                provenance: crate::domain::SourceProvenance::Generated {
                    generation_job_id: Uuid::nil(),
                    backend: "fixture".into(),
                    model: "fixture-v1".into(),
                    construction_plan_fingerprint: None,
                },
                created_at: Utc::now(),
            })
            .collect()
    }

    #[test]
    fn split_is_stable_regardless_of_source_order() {
        let dataset_id = Uuid::new_v4();
        let source = rows(dataset_id);
        let mut reversed = source.clone();
        reversed.reverse();
        let configuration =
            SplitConfiguration::new(SplitRatios::new(0.6, 0.2, 0.2).expect("ratios"), 42);
        let (first_snapshot, first) =
            build_snapshot(dataset_id, "stable", None, configuration.clone(), source)
                .expect("snapshot");
        let (second_snapshot, second) =
            build_snapshot(dataset_id, "stable", None, configuration, reversed).expect("snapshot");

        let assignments = |members: &[crate::domain::SnapshotMember]| {
            members
                .iter()
                .map(|member| (member.source_row_id, member.split))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(assignments(&first), assignments(&second));
        assert_eq!(first_snapshot.fingerprint, second_snapshot.fingerprint);
    }

    #[test]
    fn empty_fields_keep_the_legacy_fingerprint_while_typed_fields_are_preserved() {
        let dataset_id = Uuid::new_v4();
        let configuration =
            SplitConfiguration::new(SplitRatios::new(0.6, 0.2, 0.2).expect("ratios"), 42);
        let (legacy_snapshot, legacy_members) = build_snapshot(
            dataset_id,
            "legacy-compatible",
            None,
            configuration.clone(),
            rows(dataset_id),
        )
        .expect("legacy snapshot");
        let expected = artifact_core::fingerprint(&LegacySnapshotFingerprintInput {
            source_dataset_id: legacy_snapshot.source_dataset_id,
            name: &legacy_snapshot.name,
            description: &legacy_snapshot.description,
            split_configuration: legacy_snapshot.split_configuration.clone(),
            members: legacy_members
                .iter()
                .map(|member| LegacySnapshotMemberFingerprintInput {
                    source_row_id: member.source_row_id,
                    split: member.split,
                    text: &member.text,
                    label: &member.label,
                    dimensions: &member.dimensions,
                    provenance: &member.source_provenance,
                    source_created_at: member.source_created_at,
                })
                .collect(),
        })
        .expect("legacy fingerprint");
        assert_eq!(legacy_snapshot.fingerprint, expected);

        let mut enriched = rows(dataset_id);
        for row in &mut enriched {
            row.fields
                .insert("channel".into(), serde_json::json!("chat"));
        }
        let (enriched_snapshot, enriched_members) = build_snapshot(
            dataset_id,
            "legacy-compatible",
            None,
            configuration,
            enriched,
        )
        .expect("enriched snapshot");
        assert_ne!(enriched_snapshot.fingerprint, legacy_snapshot.fingerprint);
        assert!(
            enriched_members
                .iter()
                .all(|member| member.fields["channel"] == "chat")
        );
    }

    #[test]
    fn stratified_split_conserves_every_row_exactly_once() {
        let dataset_id = Uuid::new_v4();
        let (_, members) = build_snapshot(
            dataset_id,
            "snapshot",
            None,
            SplitConfiguration::new(SplitRatios::new(0.6, 0.2, 0.2).expect("ratios"), 7),
            rows(dataset_id),
        )
        .expect("snapshot");

        assert_eq!(members.len(), 20);
        assert_eq!(
            members
                .iter()
                .map(|member| member.source_row_id)
                .collect::<BTreeSet<_>>()
                .len(),
            20
        );
        for label in ["billing", "fraud"] {
            let label_members = members.iter().filter(|member| member.label == label);
            let counts = label_members.fold(BTreeMap::new(), |mut counts, member| {
                *counts.entry(member.split).or_insert(0) += 1;
                counts
            });
            assert_eq!(counts[&SnapshotSplit::Train], 6);
            assert_eq!(counts[&SnapshotSplit::Validation], 2);
            assert_eq!(counts[&SnapshotSplit::Test], 2);
        }
    }

    #[test]
    fn rejects_duplicate_source_membership() {
        let dataset_id = Uuid::new_v4();
        let mut source = rows(dataset_id);
        source.push(source[0].clone());
        assert!(
            build_snapshot(
                dataset_id,
                "snapshot",
                None,
                SplitConfiguration::new(SplitRatios::default(), 1),
                source,
            )
            .is_err()
        );
    }

    #[test]
    fn group_aware_split_keeps_related_rows_together_and_is_deterministic() {
        let dataset_id = Uuid::new_v4();
        let mut source = rows(dataset_id);
        for (index, row) in source.iter_mut().enumerate() {
            row.dimensions
                .insert("account".into(), format!("account-{}", index / 2));
        }
        let configuration =
            SplitConfiguration::new(SplitRatios::new(0.6, 0.2, 0.2).expect("ratios"), 42)
                .with_group_dimension(Some("account".into()))
                .expect("group configuration");
        let mut reversed = source.clone();
        reversed.reverse();
        let (_, first) = build_snapshot(dataset_id, "grouped", None, configuration.clone(), source)
            .expect("snapshot");
        let (_, second) =
            build_snapshot(dataset_id, "grouped", None, configuration, reversed).expect("snapshot");

        let assignments = |members: &[crate::domain::SnapshotMember]| {
            members
                .iter()
                .map(|member| (member.source_row_id, member.split))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(assignments(&first), assignments(&second));
        let mut group_splits = BTreeMap::<String, BTreeSet<SnapshotSplit>>::new();
        for member in first {
            group_splits
                .entry(member.dimensions["account"].clone())
                .or_default()
                .insert(member.split);
        }
        assert!(group_splits.values().all(|splits| splits.len() == 1));
    }

    #[test]
    fn group_aware_split_rejects_missing_group_values() {
        let dataset_id = Uuid::new_v4();
        let result = build_snapshot(
            dataset_id,
            "grouped",
            None,
            SplitConfiguration::new(SplitRatios::default(), 42)
                .with_group_dimension(Some("account".into()))
                .expect("group configuration"),
            rows(dataset_id),
        );
        assert!(matches!(
            result,
            Err(crate::domain::DatasetError::MissingGroupDimension { .. })
        ));
    }
}
