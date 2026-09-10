use chrono::{TimeZone, Utc};
use dataset_core::domain::SnapshotSplit;
use dataset_core::versions::{
    DatasetBranch, DatasetChanges, DatasetMember, DatasetVersion, SourceRecord,
};
use uuid::Uuid;

fn at() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_750_000_000, 0).unwrap()
}

fn row(n: u64) -> DatasetMember {
    DatasetMember::imported(
        SourceRecord {
            import_id: Uuid::from_u128(100),
            artifact_fingerprint: artifact_core::fingerprint(&"source").unwrap(),
            record: n,
        },
        artifact_core::fingerprint(&format!("row {n}")).unwrap(),
        SnapshotSplit::Train,
    )
    .unwrap()
}

fn base() -> (DatasetBranch, DatasetVersion) {
    let branch = DatasetBranch::new(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "Base data".into(),
        None,
        at(),
    )
    .unwrap();
    let version =
        DatasetVersion::initial(Uuid::from_u128(3), &branch, vec![row(1), row(2)], at()).unwrap();
    (branch, version)
}

#[test]
fn variants_reconstruct_exact_membership_without_changing_the_base() {
    let (base_branch, base_version) = base();
    let branch = DatasetBranch::new(
        Uuid::from_u128(4),
        base_branch.project_id,
        "Variant A".into(),
        Some(base_version.reference()),
        at(),
    )
    .unwrap();
    let first = DatasetVersion::fork(Uuid::from_u128(5), &branch, &base_version, at()).unwrap();
    let delta = DatasetChanges {
        added: vec![row(3)],
        removed: vec![row(1).id],
        replaced: vec![],
    };
    let next = DatasetVersion::revise(Uuid::from_u128(6), &branch, &first, delta, at()).unwrap();
    assert_eq!(next.members, vec![row(2), row(3)]);
    assert_eq!(next.number, 2);
    assert_eq!(base_version.members, vec![row(1), row(2)]);
    assert_eq!(first.members, base_version.members);
    next.verify(&branch, Some(&first)).unwrap();
    let restored: DatasetVersion =
        serde_json::from_str(&serde_json::to_string(&next).unwrap()).unwrap();
    assert_eq!(restored, next);
    restored.verify(&branch, Some(&first)).unwrap();
    assert!(restored.verify(&branch, Some(&base_version)).is_err());
}

#[test]
fn replacing_a_row_preserves_its_logical_id_and_records_new_source() {
    let (branch, first) = base();
    let mut replacement = row(3);
    replacement.id = row(1).id;
    let changes = DatasetChanges {
        added: vec![],
        removed: vec![],
        replaced: vec![replacement.clone()],
    };
    let next = DatasetVersion::revise(Uuid::from_u128(7), &branch, &first, changes, at()).unwrap();
    assert_eq!(next.members, vec![replacement, row(2)]);
    assert_eq!(next.members[0].id, first.members[0].id);
    assert_ne!(
        next.members[0].content_fingerprint,
        first.members[0].content_fingerprint
    );
    next.verify(&branch, Some(&first)).unwrap();
}

#[test]
fn malformed_changes_and_tampered_versions_are_rejected() {
    let (branch, first) = base();
    for changes in [
        DatasetChanges {
            added: vec![row(1)],
            ..Default::default()
        },
        DatasetChanges {
            removed: vec![row(3).id],
            ..Default::default()
        },
        DatasetChanges {
            removed: vec![row(1).id.clone(), row(1).id],
            ..Default::default()
        },
        DatasetChanges {
            replaced: vec![row(3)],
            ..Default::default()
        },
        DatasetChanges {
            replaced: vec![row(1)],
            ..Default::default()
        },
        DatasetChanges {
            removed: vec![row(1).id, row(2).id],
            ..Default::default()
        },
        DatasetChanges::default(),
    ] {
        assert!(DatasetVersion::revise(Uuid::new_v4(), &branch, &first, changes, at()).is_err());
    }
    let mut tampered = first.clone();
    tampered.members.reverse();
    assert!(tampered.verify(&branch, None).is_err());
    tampered = first.clone();
    tampered.changes.added.clear();
    assert!(tampered.verify(&branch, None).is_err());
    tampered = first.clone();
    tampered.fingerprint = artifact_core::fingerprint(&"fake").unwrap();
    assert!(tampered.verify(&branch, None).is_err());
    for field in [
        "id",
        "datasetId",
        "projectId",
        "number",
        "createdAt",
        "parent",
        "members",
    ] {
        let mut changed = serde_json::to_value(&first).unwrap();
        changed[field] = match field {
            "id" | "datasetId" | "projectId" => serde_json::json!(Uuid::nil()),
            "number" => serde_json::json!(99),
            "createdAt" => serde_json::json!(at() - chrono::Duration::seconds(1)),
            "parent" => serde_json::to_value(first.reference()).unwrap(),
            "members" => serde_json::json!([]),
            _ => unreachable!(),
        };
        let changed: DatasetVersion = serde_json::from_value(changed).unwrap();
        assert!(
            changed.verify(&branch, None).is_err(),
            "accepted changed {field}"
        );
    }
}

#[test]
fn project_ancestry_and_dataset_sequence_cannot_be_forged() {
    let (branch, first) = base();
    assert!(
        DatasetBranch::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Foreign".into(),
            Some(first.reference()),
            at()
        )
        .is_err()
    );
    let unrelated = DatasetBranch::new(
        Uuid::new_v4(),
        branch.project_id,
        "Other".into(),
        None,
        at(),
    )
    .unwrap();
    assert!(DatasetVersion::fork(Uuid::new_v4(), &unrelated, &first, at()).is_err());
    assert!(
        DatasetVersion::revise(
            Uuid::new_v4(),
            &unrelated,
            &first,
            DatasetChanges {
                added: vec![row(3)],
                ..Default::default()
            },
            at()
        )
        .is_err()
    );
    assert!(DatasetVersion::initial(Uuid::new_v4(), &branch, vec![row(1), row(1)], at()).is_err());
    assert!(DatasetVersion::initial(Uuid::new_v4(), &branch, vec![], at()).is_err());
}
