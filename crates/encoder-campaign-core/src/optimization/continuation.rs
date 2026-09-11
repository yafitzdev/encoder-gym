//! Finite continuation policy for the single-iteration optimization parent.
use super::{OptimizationError, OptimizationRunState};
use crate::CampaignState;

/// A strictly decreasing rank for routine execution. None means stop at a
/// terminal state or an explicit approval boundary. The rank is a termination
/// proof, not a progress percentage or permission to invoke a backend.
pub fn automatic_stage_rank(
    state: OptimizationRunState,
    campaign: Option<CampaignState>,
) -> Result<Option<u8>, OptimizationError> {
    use CampaignState as C;
    use OptimizationRunState as O;
    match (state, campaign) {
        (O::Completed | O::Cancelled | O::Failed, _) => Ok(None),
        (O::Planned, None) => Ok(Some(9)),
        (O::CampaignActive, Some(campaign)) => Ok(match campaign {
            C::AwaitingGeneration => Some(8),
            C::ReadyToPrepare => Some(7),
            C::ReadyToStart => Some(6),
            C::RunningDevelopment => Some(5),
            C::AwaitingFinalization => Some(4),
            C::AwaitingSealedAuthorization => None,
            C::SealedAuthorized => Some(3),
            C::RenewalRequired => Some(2),
            C::Completed => Some(1),
        }),
        _ => Err(OptimizationError::Integrity(
            "optimization and campaign continuation states disagree".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimization::{
        OptimizationEventKind, ProductionOptimizationRun, first_optimization_event,
        replay_optimization,
    };

    #[test]
    fn authorization_is_versioned_exact_idempotent_and_replay_validated() {
        let now = chrono::Utc::now();
        let mut run = ProductionOptimizationRun {
            schema_version: 1,
            id: uuid::Uuid::new_v4(),
            definition_id: uuid::Uuid::new_v4(),
            definition_fingerprint: format!("sha256:{}", "1".repeat(64)),
            reserved_campaign_id: uuid::Uuid::new_v4(),
            reserved_protocol_id: uuid::Uuid::new_v4(),
            reserved_experiment_run_id: uuid::Uuid::new_v4(),
            created_at: now,
            fingerprint: String::new(),
        };
        run.fingerprint = run.reproduce_fingerprint().unwrap();
        let first = first_optimization_event(&run, now).unwrap();
        assert_eq!(first.schema_version, 1);
        let mut events = vec![first.clone()];
        let view = replay_optimization(&run, &events).unwrap();
        assert!(
            serde_json::to_value(&view)
                .unwrap()
                .get("automatic_execution_authorized_by")
                .is_none()
        );
        for actor in [
            "",
            " operator",
            "operator ",
            "line\nbreak",
            "a\0b",
            &"x".repeat(121),
        ] {
            assert!(
                view.authorize_automatic_execution(&run, actor, now)
                    .is_err()
            );
        }
        let authorized = view
            .authorize_automatic_execution(&run, "operator", now)
            .unwrap()
            .unwrap();
        assert_eq!(authorized.schema_version, 2);
        let mut unversioned = authorized.clone();
        unversioned.schema_version = 1;
        unversioned.fingerprint = unversioned.reproduce_fingerprint().unwrap();
        assert!(unversioned.validate_integrity(&run).is_err());
        let mut value = serde_json::to_value(&authorized).unwrap();
        value["event"]["approve_sealed"] = serde_json::json!(true);
        assert!(serde_json::from_value::<crate::optimization::OptimizationEvent>(value).is_err());
        events.push(authorized.clone());
        let view = replay_optimization(&run, &events).unwrap();
        assert_eq!(view.state, OptimizationRunState::Planned);
        assert!(
            view.authorize_automatic_execution(&run, "operator", now)
                .unwrap()
                .is_none()
        );
        assert!(
            view.authorize_automatic_execution(&run, "another", now)
                .is_err()
        );
        let mut foreign = run.clone();
        foreign.id = uuid::Uuid::new_v4();
        assert!(
            view.authorize_automatic_execution(&foreign, "operator", now)
                .is_err()
        );
        assert!(
            view.next_event(&run, authorized.event.clone(), now)
                .is_err()
        );
        let mut duplicate = authorized;
        duplicate.id = uuid::Uuid::new_v4();
        duplicate.sequence += 1;
        duplicate.previous_event_fingerprint = Some(view.last_event_fingerprint.clone());
        duplicate.fingerprint = duplicate.reproduce_fingerprint().unwrap();
        let mut duplicated = events.clone();
        duplicated.push(duplicate);
        assert!(replay_optimization(&run, &duplicated).is_err());
        let cancelled = view
            .next_event(
                &run,
                OptimizationEventKind::Cancelled {
                    reason: "operator cancelled".into(),
                },
                now,
            )
            .unwrap();
        assert_eq!(cancelled.schema_version, 1);
        events.push(cancelled);
        assert_eq!(
            replay_optimization(&run, &events).unwrap().state,
            OptimizationRunState::Cancelled
        );
        assert_eq!(
            events[0], first,
            "authorizing continuation must not change historical fingerprints"
        );
    }

    #[test]
    fn routine_paths_are_strictly_finite_and_approval_is_a_stop() {
        use CampaignState as C;
        use OptimizationRunState as O;
        assert_eq!(automatic_stage_rank(O::Planned, None).unwrap(), Some(9));
        for path in [
            vec![
                C::AwaitingGeneration,
                C::ReadyToPrepare,
                C::ReadyToStart,
                C::RunningDevelopment,
                C::AwaitingFinalization,
                C::RenewalRequired,
                C::Completed,
            ],
            vec![C::SealedAuthorized, C::RenewalRequired, C::Completed],
        ] {
            let mut previous = 9;
            for stage in path {
                let rank = automatic_stage_rank(O::CampaignActive, Some(stage))
                    .unwrap()
                    .unwrap();
                assert!(rank < previous);
                previous = rank;
            }
        }
        assert_eq!(
            automatic_stage_rank(O::CampaignActive, Some(C::AwaitingSealedAuthorization)).unwrap(),
            None
        );
        for state in [O::Cancelled, O::Completed, O::Failed] {
            assert_eq!(
                automatic_stage_rank(state, Some(C::RunningDevelopment)).unwrap(),
                None
            );
            assert_eq!(automatic_stage_rank(state, None).unwrap(), None);
        }
        assert!(automatic_stage_rank(O::CampaignActive, None).is_err());
        assert!(automatic_stage_rank(O::Planned, Some(C::ReadyToPrepare)).is_err());
    }
}
