use generation_supervisor_core::ports::GenerationSupervisorStore;

use super::super::*;

pub(in crate::commands::doctor) async fn supervisor_facts_check(
    store: &SqliteStore,
) -> DoctorCheck {
    match store.verify_supervisor_integrity().await {
        Ok(report) if report.healthy() => pass(
            "generation_supervisor_facts",
            format!(
                "verified {} contract(s), {} run(s), {} prompt version(s), {} row observation(s), {} quality window(s), {} decision(s), {} advisor session(s), {} revision proposal(s), {} activation(s), {} qualification handoff(s), and {} qualification application(s)",
                report.contracts,
                report.runs,
                report.prompt_versions,
                report.row_observations,
                report.quality_windows,
                report.decisions,
                report.advisor_sessions,
                report.revision_proposals,
                report.activations,
                report.qualification_handoffs,
                report.qualification_applications,
            ),
        ),
        Ok(report) => fail("generation_supervisor_facts", report.errors.join("; ")),
        Err(error) => fail("generation_supervisor_facts", error.to_string()),
    }
}
