use chrono::Utc;
use encoder_experiment_core::{
    benchmark::{BenchmarkDefinition, BenchmarkResult},
    domain::{
        BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
        ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
        TrainingCandidate,
    },
    metrics::{
        EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
        MetricGateCondition,
    },
    protocol::ExperimentProtocol,
};
use serde_json::json;
use std::collections::BTreeMap;

fn fp(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
fn project(model: char) -> ExternalProjectSnapshot {
    ExternalProjectSnapshot::create(
        "Fixture",
        EncoderTaskKind::RetrievalRanking,
        "revision",
        fp('1'),
        BackendIdentity::new("fixture", "v1", fp('2')).unwrap(),
        [
            ("train", EvidenceRole::Training),
            ("development", EvidenceRole::Development),
            ("holdout", EvidenceRole::SealedAcceptance),
        ]
        .into_iter()
        .map(|(key, role)| ExternalArtifactIdentity::new(key, role, 100, fp('3')).unwrap())
        .collect(),
        ModelArtifactIdentity::new("model", "fixture", 1, fp(model)).unwrap(),
        json!({"training":"unused by benchmark"}),
        Utc::now(),
    )
    .unwrap()
}
fn contract(tolerance: f64) -> MetricContract {
    MetricContract::create(
        vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
        "mrr",
        [EvidenceRole::Development, EvidenceRole::SealedAcceptance]
            .into_iter()
            .map(|role| {
                MetricGate::new(
                    "mrr",
                    role,
                    MetricGateCondition::MaximumRegression { value: tolerance },
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap()
}
fn protocol(project: &ExternalProjectSnapshot, sequence: u32) -> ExperimentProtocol {
    let contract = contract(0.0);
    let report = |role, key, score| {
        EvaluationReport::create(
            project,
            project.baseline_model.clone(),
            role,
            key,
            fp('4'),
            &contract,
            BTreeMap::from([("mrr".into(), score)]),
            50,
            Utc::now(),
        )
        .unwrap()
    };
    ExperimentProtocol::create(
        project,
        contract.clone(),
        report(EvidenceRole::Development, "development", 0.7),
        report(EvidenceRole::SealedAcceptance, "holdout", 99999.125),
        OptimizationBudget {
            maximum_candidates: 1,
            maximum_training_seconds: 60,
            maximum_development_evaluations: 1,
            maximum_sealed_evaluations: 1,
        },
        60,
        "development",
        "holdout",
        vec![
            TrainingCandidate::create(
                project,
                1,
                60,
                BTreeMap::from([("seed".into(), ParameterValue::Integer(i64::from(sequence)))]),
            )
            .unwrap(),
        ],
        Utc::now(),
    )
    .unwrap()
}

fn project_with_changed_training(source: &ExternalProjectSnapshot) -> ExternalProjectSnapshot {
    let copied_baseline = ModelArtifactIdentity::new(
        "materialized/baseline",
        source.baseline_model.format.clone(),
        source.baseline_model.bytes,
        source.baseline_model.fingerprint.clone(),
    )
    .unwrap();
    ExternalProjectSnapshot::create(
        "Fixture with selected training data",
        source.task,
        source.source_revision.clone(),
        fp('9'),
        source.backend.clone(),
        source
            .inputs
            .iter()
            .cloned()
            .map(|mut input| {
                if input.role == EvidenceRole::Training {
                    input.fingerprint = fp('8');
                }
                input
            })
            .collect(),
        copied_baseline,
        json!({"training":"selected immutable version"}),
        Utc::now(),
    )
    .unwrap()
}

#[test]
fn models_training_settings_and_run_identity_do_not_define_the_benchmark() {
    let first = project('a');
    let second = project('b');
    let baseline_protocol = protocol(&first, 1);
    let another = protocol(&second, 2);
    assert_ne!(first.fingerprint, second.fingerprint);
    assert_ne!(baseline_protocol.fingerprint, another.fingerprint);
    let benchmark =
        BenchmarkDefinition::from_protocol(&first, &baseline_protocol, fp('5')).unwrap();
    assert_eq!(
        benchmark,
        BenchmarkDefinition::from_protocol(&second, &another, fp('5')).unwrap()
    );
    let result = BenchmarkResult::from_development(
        &benchmark,
        &second,
        &fp('5'),
        &another.baseline_development_report,
    )
    .unwrap();
    assert_eq!(result.model, second.baseline_model);
    assert_eq!(result.report_id, another.baseline_development_report.id);
    let encoded = serde_json::to_string(&benchmark).unwrap();
    assert!(!encoded.contains("99999.125"));
    assert!(!encoded.contains(&first.baseline_model.fingerprint));
    assert!(!encoded.contains("training"));
}

#[test]
fn every_evaluation_input_changes_identity_and_rejects_incompatible_results() {
    let project = project('a');
    let protocol = protocol(&project, 1);
    let baseline = BenchmarkDefinition::from_protocol(&project, &protocol, fp('5')).unwrap();
    let mut changed = Vec::new();
    let mut value = baseline.clone();
    value.suites[0].fingerprint = fp('6');
    changed.push(value);
    let mut value = baseline.clone();
    value.suites[0].support += 1;
    changed.push(value);
    let mut value = baseline.clone();
    value.evaluation_configuration_fingerprint = fp('6');
    changed.push(value);
    let mut value = baseline.clone();
    value.source_revision = "other-revision".into();
    changed.push(value);
    let mut value = baseline.clone();
    value.backend.protocol_version = "v2".into();
    changed.push(value);
    let mut value = baseline.clone();
    value.metric_contract = contract(0.1);
    changed.push(value);
    for mut value in changed {
        value.fingerprint = value.reproduce_fingerprint().unwrap();
        value.validate_integrity().unwrap();
        assert_ne!(value.fingerprint, baseline.fingerprint);
        assert!(
            value
                .validate_development_report(
                    &project,
                    &fp('5'),
                    &protocol.baseline_development_report
                )
                .is_err()
        );
    }
}

#[test]
fn sealed_reports_and_invalid_or_tampered_definitions_never_enter_comparison() {
    let project = project('a');
    let protocol = protocol(&project, 1);
    let baseline = BenchmarkDefinition::from_protocol(&project, &protocol, fp('5')).unwrap();
    assert!(
        BenchmarkResult::from_development(
            &baseline,
            &project,
            &fp('5'),
            &protocol.baseline_sealed_report
        )
        .is_err()
    );
    let mut changed = baseline.clone();
    changed.suites[0].support += 1;
    assert!(changed.validate_integrity().is_err());
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    changed.suites[0].role = EvidenceRole::Training;
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    assert!(changed.validate_integrity().is_err());
    let mut repeated = baseline.clone();
    repeated.suites.push(repeated.suites[0].clone());
    repeated.fingerprint = repeated.reproduce_fingerprint().unwrap();
    assert!(repeated.validate_integrity().is_err());
    let mut wrong = protocol.baseline_development_report.clone();
    wrong.metrics.insert("mrr".into(), 100.0);
    assert!(
        baseline
            .validate_development_report(&project, &fp('5'), &wrong)
            .is_err()
    );
    assert!(
        baseline
            .validate_development_report(&project, &fp('6'), &protocol.baseline_development_report)
            .is_err()
    );
}

#[test]
fn unchanged_shared_baseline_evidence_is_referenced_with_exact_provenance() {
    let source = project('a');
    let source_protocol = protocol(&source, 1);
    let target = project_with_changed_training(&source);
    let benchmark = BenchmarkDefinition::from_protocol(&source, &source_protocol, fp('5')).unwrap();
    let referenced = benchmark
        .reference_baseline_report(
            uuid::Uuid::new_v4(),
            &source,
            &source_protocol,
            &target,
            &source_protocol.baseline_development_report,
            Utc::now(),
        )
        .unwrap();

    benchmark
        .validate_baseline_reference(
            &source,
            &source_protocol,
            &target,
            &source_protocol.baseline_development_report,
            &referenced,
        )
        .unwrap();
    assert_eq!(referenced.project_snapshot_id, target.id);
    assert_eq!(referenced.model, target.baseline_model);
    assert_ne!(referenced.model.id, source.baseline_model.id);
    assert!(referenced.model.has_same_content(&source.baseline_model));
    assert_eq!(
        referenced.metrics,
        source_protocol.baseline_development_report.metrics
    );
    assert_eq!(
        referenced.reference.as_ref().unwrap().source_report_id,
        source_protocol.baseline_development_report.id
    );
    assert!(
        serde_json::to_string(&referenced)
            .unwrap()
            .contains("source_report_id")
    );

    let mut changed = referenced.clone();
    changed.metrics.insert("mrr".into(), 0.99);
    changed.fingerprint = changed.reproduce_fingerprint().unwrap();
    assert!(
        benchmark
            .validate_baseline_reference(
                &source,
                &source_protocol,
                &target,
                &source_protocol.baseline_development_report,
                &changed,
            )
            .is_err()
    );

    let different_baseline = project('b');
    let incompatible_target = project_with_changed_training(&different_baseline);
    assert!(
        benchmark
            .reference_baseline_report(
                uuid::Uuid::new_v4(),
                &source,
                &source_protocol,
                &incompatible_target,
                &source_protocol.baseline_development_report,
                Utc::now(),
            )
            .is_err()
    );

    let old_json = serde_json::to_string(&source_protocol.baseline_development_report).unwrap();
    assert!(!old_json.contains("reference"));
    source_protocol
        .baseline_development_report
        .validate_integrity(&source, &source_protocol.metric_contract)
        .unwrap();
}
