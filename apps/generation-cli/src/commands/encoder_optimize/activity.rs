//! Operational observations alongside (never in place of) the scientific journal.
use super::*;
use encoder_experiment_nomos::{NativeProgress, ProgressObserver};

#[derive(Debug)]
pub(crate) struct ProgressOutput;
impl ProgressObserver for ProgressOutput {
    fn observe(&self, progress: NativeProgress) {
        // This closed schema carries no native output, model text, paths, or scores.
        if let Ok(json) = serde_json::to_string(&progress) {
            eprintln!("ENCODER_GYM_PROGRESS {json}");
        }
    }
}

pub(super) fn worker_status(database: &Path, run_id: Uuid) -> serde_json::Value {
    let Some(parent) = database.parent() else {
        return serde_json::json!({"state":"unavailable"});
    };
    let directory = parent.join(format!(".encoder-optimization-{run_id}.lease"));
    match directory.try_exists() {
        Ok(false) => return serde_json::json!({"state":"idle"}),
        Err(_) => return serde_json::json!({"state":"unavailable"}),
        Ok(true) => {}
    }
    let Ok(owner) = read_execution_lease(&directory) else {
        return serde_json::json!({"state":"unavailable"});
    };
    let system = System::new_all();
    let pid = Pid::from_u32(owner.process_id);
    let Some(process) = system
        .process(pid)
        .filter(|process| process.start_time() == owner.process_started_at)
    else {
        let Ok(children) = read_execution_children(&directory, &owner) else {
            return serde_json::json!({"state":"unavailable"});
        };
        if children.iter().any(|child| {
            system
                .process(Pid::from_u32(child.process_id))
                .is_some_and(|process| process.start_time() == child.process_started_at)
        }) {
            return serde_json::json!({"state":"orphaned"});
        }
        return serde_json::json!({"state":"interrupted"});
    };
    let mut ids = std::collections::HashSet::from([pid]);
    loop {
        let before = ids.len();
        for (child_id, child) in system.processes() {
            if child.start_time() >= owner.process_started_at
                && child.parent().is_some_and(|id| ids.contains(&id))
            {
                ids.insert(*child_id);
            }
        }
        if before == ids.len() {
            break;
        }
    }
    let memory_bytes = ids
        .iter()
        .filter_map(|id| system.process(*id))
        .map(|p| p.memory())
        .sum::<u64>();
    let cpu_milliseconds = ids
        .iter()
        .filter_map(|id| system.process(*id))
        .map(|p| p.accumulated_cpu_time())
        .sum::<u64>();
    serde_json::json!({"state":"running", "started_at": chrono::DateTime::from_timestamp(process.start_time() as i64, 0), "memory_bytes":memory_bytes, "cpu_milliseconds":cpu_milliseconds})
}

pub(super) async fn timeline(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
    campaign: Option<&CampaignContext>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut items = vec![serde_json::json!({"at":context.run.created_at,"label":"Run created"})];
    for event in store.list_optimization_events(context.run.id).await? {
        if matches!(
            event.event,
            OptimizationEventKind::AutomaticExecutionAuthorized { .. }
        ) {
            items.push(
                serde_json::json!({"at":event.created_at,"label":"Automatic execution authorized"}),
            );
        }
    }
    if campaign.and_then(|c| c.experiment.as_ref()).is_none() {
        return Ok(items);
    }
    for event in store
        .load_events(context.run.reserved_experiment_run_id)
        .await?
    {
        let label = match event.event {
            ExperimentEventKind::RunCreated => "Run prepared".to_owned(),
            ExperimentEventKind::CandidateTrainingStarted { .. } => {
                "Candidate training requested".to_owned()
            }
            ExperimentEventKind::CandidateTrainingCompleted { .. } => "Candidate saved".to_owned(),
            ExperimentEventKind::CandidateDevelopmentSuiteCompleted { suite_key, .. } => {
                format!("Evaluation complete: {suite_key}")
            }
            ExperimentEventKind::CandidateDevelopmentCompleted { .. } => {
                "Evaluation complete".to_owned()
            }
            ExperimentEventKind::CandidateFailed { .. } => "Candidate failed".to_owned(),
            ExperimentEventKind::DevelopmentSelected { .. } => {
                "Candidate comparison complete".to_owned()
            }
            ExperimentEventKind::SealedAuthorized { .. } => "Final evaluation approved".to_owned(),
            ExperimentEventKind::SealedStarted { .. } => "Final evaluation started".to_owned(),
            ExperimentEventKind::SealedCompleted { .. } => "Final evaluation complete".to_owned(),
            ExperimentEventKind::Finalized { .. } => "Decision saved".to_owned(),
            ExperimentEventKind::RunFailed { .. } => "Run failed".to_owned(),
        };
        items.push(serde_json::json!({"at":event.created_at,"label":label}));
    }
    items.sort_by_cached_key(|item| {
        item["at"]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
    });
    if items.len() > 20 {
        items.drain(..items.len() - 20);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_follows_the_exact_lease_and_never_claims_a_dead_process_is_running() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("scientific.sqlite");
        let url = format!("sqlite://{}", database.to_string_lossy().replace('\\', "/"));
        let id = Uuid::new_v4();
        assert_eq!(worker_status(&database, id)["state"], "idle");
        let lease = OptimizationExecutionLease::acquire(&url, id).unwrap();
        assert_eq!(worker_status(&database, id)["state"], "running");
        assert_eq!(worker_status(&database, Uuid::new_v4())["state"], "idle");
        let mut stale = lease.owner.clone();
        stale.process_started_at = stale.process_started_at.saturating_sub(1);
        fs::write(
            lease.directory.join("owner.json"),
            serde_json::to_vec(&stale).unwrap(),
        )
        .unwrap();
        assert_eq!(worker_status(&database, id)["state"], "interrupted");
    }
}
