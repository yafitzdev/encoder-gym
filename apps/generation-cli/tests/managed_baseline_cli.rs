use chrono::Utc;
use project_workspace_core::{BoundIdentity, ModelArtifact, ModelCatalog};
use serde_json::Value;
use sqlx::{Connection, SqliteConnection};
use std::{fs, path::Path, process::Command};
use uuid::Uuid;

fn digest(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn invoke(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_synth"))
        .current_dir(root)
        .args(["--output", "json", "workspace"])
        .args(args)
        .output()
        .unwrap()
}

fn run(root: &Path, args: &[&str]) -> Value {
    let output = invoke(root, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn previous_baseline_can_be_restored_through_the_real_cli_and_activity_log() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        training_transformer::fixture::write_tiny_bert_bundle(&root.join("checkpoint")).unwrap();
        let model = run(root, &["inspect-model", "checkpoint"]);
        let created = run(
            root,
            &[
                "create",
                "project",
                "--name",
                "Baseline history",
                "--model",
                "checkpoint",
                "--expected-fingerprint",
                model["fingerprint"].as_str().unwrap(),
            ],
        );
        let catalog: ModelCatalog =
            serde_json::from_value(created["modelCatalog"].clone()).unwrap();
        let initial_revision = catalog.active_baseline_revision_id;

        let relative = "models/candidates/accepted";
        let candidate_path = root.join("project").join(relative);
        training_transformer::fixture::write_tiny_bert_bundle(&candidate_path).unwrap();
        fs::write(
            candidate_path.join("candidate.json"),
            b"{\"candidate\":true}\n",
        )
        .unwrap();
        let inspected = project_workspace_local::inspect_model(&candidate_path).unwrap();
        let candidate = ModelArtifact::trained(
            catalog.project_id,
            Uuid::new_v4(),
            "Accepted candidate",
            relative,
            &inspected,
            catalog.active_model().id,
            BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('1'),
            },
            BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('2'),
            },
            BoundIdentity {
                id: Uuid::new_v4().to_string(),
                fingerprint: digest('3'),
            },
            BoundIdentity {
                id: "fixture:v1".into(),
                fingerprint: digest('4'),
            },
            digest('5'),
            "fixture-revision".into(),
            Utc::now(),
        )
        .unwrap();
        let promoted = catalog
            .with_promotion(
                candidate.clone(),
                Uuid::new_v4(),
                "accepted-decision".into(),
                digest('6'),
                "fixture",
                "Accepted final result",
                Utc::now(),
            )
            .unwrap();
        let promotion = promoted.active_revision();
        let database_url = format!(
            "sqlite://{}",
            root.join("project/project.sqlite").display()
        );
        let mut database = SqliteConnection::connect(&database_url).await.unwrap();
        sqlx::query("INSERT INTO model_artifacts (id, project_id, content_fingerprint, origin, metadata_json) VALUES (?, ?, ?, ?, ?)")
            .bind(candidate.id.to_string()).bind(candidate.project_id.to_string())
            .bind(&candidate.fingerprint).bind("trained")
            .bind(serde_json::to_string(&candidate).unwrap()).execute(&mut database).await.unwrap();
        sqlx::query("INSERT INTO baseline_revisions (id, project_id, sequence, model_artifact_id, previous_revision_id, change_kind, fingerprint, metadata_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(promotion.id.to_string()).bind(promotion.project_id.to_string())
            .bind(i64::try_from(promotion.sequence).unwrap()).bind(promotion.model_artifact_id.to_string())
            .bind(promotion.previous_revision_id.unwrap().to_string()).bind("promotion")
            .bind(&promotion.fingerprint).bind(serde_json::to_string(promotion).unwrap())
            .execute(&mut database).await.unwrap();
        sqlx::query("UPDATE model_catalog_state SET active_baseline_revision_id=? WHERE singleton=1")
            .bind(promotion.id.to_string()).execute(&mut database).await.unwrap();
        database.close().await.unwrap();

        let restoration = Uuid::new_v4();
        let args = [
            "restore-baseline",
            "project",
            "--revision-id",
            &restoration.to_string(),
            "--target-revision-id",
            &initial_revision.to_string(),
            "--expected-baseline-revision-id",
            &promotion.id.to_string(),
        ];
        let restored = run(root, &args);
        assert_eq!(
            restored["modelCatalog"]["activeBaselineRevisionId"],
            restoration.to_string()
        );
        assert_eq!(
            restored["modelCatalog"]["baselineRevisions"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            restored["modelCatalog"]["baselineRevisions"][2]["change"]["kind"],
            "restoration"
        );
        assert_eq!(
            restored["modelCatalog"]["baselineRevisions"][2]["change"]["target_revision_id"],
            initial_revision.to_string()
        );
        assert_eq!(run(root, &args)["modelCatalog"], restored["modelCatalog"]);

        let activity = run(root, &["activity", "project", "list"]);
        let actions = activity["actions"].as_array().unwrap();
        let action = actions
            .iter()
            .find(|action| action["operation"] == "model.restore_baseline")
            .unwrap();
        assert_eq!(action["state"], "succeeded");
        assert!(action["events"].as_array().unwrap().len() >= 2);
        let encoded = serde_json::to_string(action).unwrap();
        assert!(encoded.contains(&restoration.to_string()));
        assert!(!encoded.contains("candidate.json"));

        let stale = invoke(
            root,
            &[
                "restore-baseline",
                "project",
                "--revision-id",
                &Uuid::new_v4().to_string(),
                "--target-revision-id",
                &initial_revision.to_string(),
                "--expected-baseline-revision-id",
                &promotion.id.to_string(),
            ],
        );
        assert!(!stale.status.success());
        assert!(
            String::from_utf8_lossy(&stale.stderr).contains("active baseline changed")
        );
        assert_eq!(
            run(root, &["verify", "project"])["modelCatalog"],
            restored["modelCatalog"]
        );
    });
}
