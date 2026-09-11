//! Drives only the existing immutable, finite run; it never chooses a recipe.
use super::*;
use encoder_campaign_core::optimization::automatic_stage_rank;

pub(super) async fn drive<B: EncoderTaskBackend>(
    store: &SqliteExperimentStore,
    mut backend_factory: impl FnMut() -> anyhow::Result<B>,
    args: EncoderOptimizeAuthorizeArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if matches!(
        context.view.state,
        OptimizationRunState::Planned | OptimizationRunState::CampaignActive
    ) {
        if let Some(event) = context.view.authorize_automatic_execution(
            &context.run,
            &args.authorized_by,
            Utc::now(),
        )? {
            store.append_optimization_event(event).await?;
        }
    }
    let mut previous_rank = None;
    loop {
        // Reload after every stage. In particular, never run another child
        // from an in-memory state that predates operator cancellation.
        let context = load_launch(store, args.run_id).await?;
        let campaign = if context.view.state == OptimizationRunState::CampaignActive {
            Some(
                load_optimization_campaign(store, context.run.reserved_campaign_id)
                    .await?
                    .view
                    .state,
            )
        } else {
            None
        };
        let Some(rank) = automatic_stage_rank(context.view.state, campaign)? else {
            return print_current_status(store, &context, true).await;
        };
        anyhow::ensure!(
            context.view.automatic_execution_authorized_by.as_deref() == Some(&args.authorized_by),
            "automatic execution has no matching persisted authorization"
        );
        anyhow::ensure!(
            previous_rank.is_none_or(|previous| rank < previous),
            "automatic execution made no forward progress; inspect the run before retrying"
        );
        previous_rank = Some(rank);
        // Errors stop this invocation. Only an explicit retry may recover the
        // same reserved child through its existing idempotent slice contract.
        resume_stage(store, &mut backend_factory, args.run_id).await?;
    }
}
