use super::*;

pub(super) fn assert_lineage(
    evidence: project_workspace_core::optimization_final_promotion::AgentFinalPromotionEvidence<'_>,
) {
    use project_workspace_core::optimization_final_promotion::AgentFinalPromotionEvidence;
    let decision = |model: &project_workspace_core::ModelArtifact,
                    training: &project_workspace_core::optimization_iteration_execution::IterationTrainingBinding,
                    baseline: &project_workspace_core::BaselineRevision| {
        AgentFinalPromotionEvidence {
            model,
            training,
            baseline,
            ..evidence
        }
        .decision()
    };
    assert!(decision(evidence.model, evidence.training, evidence.baseline).is_ok());
    // Cached fingerprints cannot conceal mutated training or baseline metadata.
    let mut training = evidence.training.clone();
    training.native_materialization_fingerprint = format!("sha256:{}", "b".repeat(64));
    assert!(decision(evidence.model, &training, evidence.baseline).is_err());
    training.fingerprint = training.reproduce().unwrap();
    assert!(decision(evidence.model, &training, evidence.baseline).is_err());
    let mut baseline = evidence.baseline.clone();
    baseline.actor = "substituted-actor".into();
    assert!(decision(evidence.model, evidence.training, &baseline).is_err());
    for change in 0..10 {
        let mut model = evidence.model.clone();
        match change {
            0 => model.project_id = Uuid::new_v4(),
            1 => model.parent_model_id = Some(Uuid::new_v4()),
            2 => model.source_model.as_mut().unwrap().id = Uuid::new_v4().to_string(),
            3 => {
                model.producing_run.as_mut().unwrap().fingerprint =
                    format!("sha256:{}", "b".repeat(64))
            }
            4 => model.training_snapshot.as_mut().unwrap().id = Uuid::new_v4().to_string(),
            5 => model.trainer.as_mut().unwrap().id = "another-trainer".into(),
            6 => {
                model.effective_configuration_fingerprint =
                    Some(format!("sha256:{}", "b".repeat(64)))
            }
            7 => model.source_revision = Some("another-revision".into()),
            8 => model.bytes += 1,
            _ => model.format = "another-format".into(),
        }
        assert!(
            decision(&model, evidence.training, evidence.baseline).is_err(),
            "Altered model field {change}"
        );
    }
}

pub(super) async fn assert_promotion(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    grant: &AgentFinalAuthorization,
    receipt: &AgentFinalReceipt,
) {
    eprintln!(
        "Checking explicit final promotion (accepted={})",
        receipt.accepted
    );
    let before = project_workspace_local::open_workspace(folder, true)
        .await
        .unwrap();
    let catalog = before.model_catalog.as_ref().unwrap();
    let baseline = &grant.scope.comparison_baseline_revision.id;
    let execute = |baseline: &str, fingerprint: &str, extra: &[&str]| {
        let mut args = vec![
            "--expected-baseline-revision",
            baseline,
            "--expected-final-receipt",
            fingerprint,
        ];
        args.extend(extra);
        command(root, run_id, "promote-final-agent", &args)
    };
    let native = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let calls = fs::read(root.join("agent-calls.jsonl")).unwrap();
    let scientific = fs::read(folder.join("runs/scientific.sqlite")).unwrap();
    assert!(
        !execute(&Uuid::new_v4().to_string(), &receipt.fingerprint, &[])
            .status
            .success()
    );
    assert!(
        !execute(baseline, &format!("sha256:{}", "0".repeat(64)), &[])
            .status
            .success()
    );
    if !receipt.accepted {
        assert!(root.join("runtime/runs/fixture-final-rejected").is_file());
        assert_injected(
            execute(baseline, &receipt.fingerprint, &[]),
            "accepted final evidence",
        );
        assert_eq!(
            project_workspace_local::open_workspace(folder, true)
                .await
                .unwrap()
                .model_catalog,
            before.model_catalog
        );
    } else {
        let promoted = execute(baseline, &receipt.fingerprint, &[]);
        assert!(
            promoted.status.success(),
            "{}",
            String::from_utf8_lossy(&promoted.stderr)
        );
        let promoted: Value = serde_json::from_slice(&promoted.stdout).unwrap();
        let again = execute(baseline, &receipt.fingerprint, &[]);
        assert!(
            again.status.success(),
            "{}",
            String::from_utf8_lossy(&again.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&again.stdout).unwrap(),
            promoted
        );
        assert!(
            !execute(
                baseline,
                &receipt.fingerprint,
                &["--actor", "another-operator"]
            )
            .status
            .success()
        );
        let after = project_workspace_local::open_workspace(folder, true)
            .await
            .unwrap();
        let next = after.model_catalog.as_ref().unwrap();
        assert_eq!(
            next.artifacts, catalog.artifacts,
            "Promotion reuses registered custody"
        );
        assert_eq!(after.model_dataset_links, before.model_dataset_links);
        assert_eq!(
            next.baseline_revisions.len(),
            catalog.baseline_revisions.len() + 1
        );
        assert_eq!(
            next.active_model().source_model.as_ref(),
            Some(&grant.scope.model)
        );
        assert_ne!(next.active_model().id, catalog.active_model().id);
        // The newly active checkpoint must be usable by the ordinary next-run binding.
        // Preview resolves the exact Agent decision without creating another project.
        let preview = run(
            root,
            &[
                "preview-nomos-binding",
                "project",
                "--runtime",
                "runtime",
                "--python",
                env!("CARGO_BIN_EXE_synth-benchmark-fixture"),
            ],
        );
        assert_eq!(
            preview["baselineRevisionId"],
            next.active_baseline_revision_id.to_string()
        );
    }
    assert_eq!(
        fs::read(folder.join("runs/scientific.sqlite")).unwrap(),
        scientific
    );
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native
    );
    assert_eq!(fs::read(root.join("agent-calls.jsonl")).unwrap(), calls);
}
