//! Resolve only a run's pinned roles. No model discovery, mutable project
//! defaults, provider probes or credential values enter the execution record.
use agent_runtime_core::AgentRuntime;
use anyhow::{Context, Result, ensure};
use encoder_optimization_runner::AgentSelection;
use generation_core::structured::StructuredGenerationBackend;
use generation_openai_compatible::OpenAICompatibleBackend;
use project_workspace_core::{
    ProviderAuthentication, ProviderConfiguration, ProviderKind, ProviderLimits,
};
use research_agent_pi_process::{PiProcessRuntime, ProjectProvider};
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct SelectedAgent {
    pub runtime: Arc<dyn AgentRuntime>,
    pub selection: AgentSelection,
}

pub(super) fn output_limit(limits: &ProviderLimits) -> u32 {
    limits.maximum_output_tokens.min(8192) as u32
}

pub(super) fn cost_limit(limits: &ProviderLimits) -> u64 {
    limits.maximum_cost_microusd / u64::from(limits.maximum_requests)
}

fn credential_environment(
    project: Uuid,
    provider: &ProviderConfiguration,
) -> Result<Option<String>> {
    provider.validate(project)?;
    ensure!(
        provider.kind == ProviderKind::OpenaiCompatible,
        "This execution route requires a configured OpenAI-compatible connection"
    );
    if provider.authentication == ProviderAuthentication::None {
        return Ok(None);
    }
    let name = provider.secret.as_ref().context("Pinned provider credential reference is missing")?
        .execution_environment(project, provider.role)?
        .context("Pinned legacy credential has no execution environment; save its connection in Settings")?;
    ensure!(
        std::env::var(&name).is_ok_and(|value| !value.trim().is_empty()),
        "Pinned {} credential is unavailable; supply its exact saved connection key before resuming",
        provider.role.key()
    );
    Ok(Some(name))
}

pub(super) fn agent(
    project: Uuid,
    provider: &ProviderConfiguration,
    limits: &ProviderLimits,
    args: crate::cli::ResearchRuntimeArgs,
) -> Result<SelectedAgent> {
    let api_key_env = credential_environment(project, provider)?;
    let sidecar = crate::commands::research::resolve_sidecar(&args)?;
    let maximum_output_tokens_per_turn = output_limit(limits);
    let runtime =
        PiProcessRuntime::new(args.node, sidecar).with_project_provider(ProjectProvider {
            base_url: provider
                .endpoint
                .clone()
                .context("Pinned Agent endpoint is missing")?,
            maximum_output_tokens: maximum_output_tokens_per_turn,
        });
    Ok(SelectedAgent {
        runtime: Arc::new(runtime),
        selection: AgentSelection {
            provider: "project-connection".into(),
            model: provider.model.clone(),
            api_key_env,
            maximum_output_tokens_per_turn,
            maximum_cost_microusd_per_turn: cost_limit(limits),
            runtime_cost_is_known: false,
        },
    })
}

pub(super) fn generation(
    project: Uuid,
    provider: &ProviderConfiguration,
) -> Result<Arc<dyn StructuredGenerationBackend>> {
    let environment = credential_environment(project, provider)?;
    let key = environment
        .map(|name| std::env::var(name).context("Pinned generator credential became unavailable"))
        .transpose()?;
    Ok(Arc::new(OpenAICompatibleBackend::new(
        provider
            .endpoint
            .as_deref()
            .context("Pinned generator endpoint is missing")?,
        key,
        provider.model.clone(),
    )?))
}
