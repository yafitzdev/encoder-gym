//! Actual CLI proof: on-demand case reads do not execute native work or mutate
//! either database, and missing saved samples do not become claimed fixes.
use super::*;

pub(super) fn verify(
    root: &Path,
    folder: &Path,
    run_id: Uuid,
    history: &Value,
    baseline_path: &Path,
) {
    let project_before = fs::read(folder.join("project.sqlite")).unwrap();
    let scientific_before = fs::read(folder.join("runs/scientific.sqlite")).unwrap();
    let native_before = fs::read(root.join("runtime/native-invocations.log")).unwrap();
    let read = || {
        run(
            root,
            &[
                "optimization-run",
                "project",
                "cases",
                &run_id.to_string(),
                "--iteration",
                "1",
            ],
        )
    };
    let value = read();
    assert_eq!(value["runId"], run_id.to_string());
    assert_eq!(value["iterationId"], history["iterations"][0]["id"]);
    assert_eq!(
        value["baselineModelId"],
        history["iterations"][0]["startingModelId"]
    );
    assert_eq!(
        value["candidateModelId"],
        history["iterations"][0]["modelId"]
    );
    let comparisons = value["comparisons"].as_array().unwrap();
    assert_eq!(comparisons.len(), 2);
    for comparison in comparisons {
        assert_eq!(comparison["sampleLimit"], 50);
        assert_eq!(comparison["baseline"]["sampleSize"], 1);
        assert_eq!(comparison["candidate"]["sampleSize"], 1);
        let cases = comparison["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 2);
        assert!(cases.iter().all(|pair| pair["change"] == "not_comparable"));
        assert!(
            cases
                .iter()
                .any(|pair| pair["baseline"]["expectedRank"] == 2 && pair["candidate"].is_null())
        );
        assert!(
            cases
                .iter()
                .any(|pair| pair["candidate"]["expectedRank"] == 3 && pair["baseline"].is_null())
        );
    }
    assert!(!value.to_string().contains("NEVER_DISCLOSE_HOLDOUT"));
    assert!(!value.to_string().contains("99999.125"));
    assert_eq!(value, read());
    let missing = baseline_path.with_extension("unavailable.json");
    fs::rename(baseline_path, &missing).unwrap();
    let partial = read();
    assert_eq!(
        partial["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["baseline"]["sampleSize"].is_null())
            .count(),
        1
    );
    assert!(
        partial["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["candidate"]["sampleSize"] == 1)
    );
    fs::rename(missing, baseline_path).unwrap();
    assert_eq!(value, read());
    assert_eq!(
        fs::read(folder.join("project.sqlite")).unwrap(),
        project_before
    );
    assert_eq!(
        fs::read(folder.join("runs/scientific.sqlite")).unwrap(),
        scientific_before
    );
    assert_eq!(
        fs::read(root.join("runtime/native-invocations.log")).unwrap(),
        native_before
    );
}
