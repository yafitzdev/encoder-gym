use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{OptimizationError, fingerprint, require};

/// An immutable development-only input binding; no sealed evidence role exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentAnalysisScope {
    pub run_id: Uuid,
    pub iteration: u32,
    pub launch_fingerprint: String,
    pub dataset_version_id: Uuid,
    pub dataset_fingerprint: String,
    pub development_evidence_fingerprint: String,
    pub objective: String,
    pub maximum_turns: u32,
    pub maximum_row_changes: u32,
}

impl AgentAnalysisScope {
    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(
            !self.run_id.is_nil() && !self.dataset_version_id.is_nil(),
            "Run and dataset identities must be non-nil",
        )?;
        require(
            (1..=10).contains(&self.iteration),
            "Iteration is outside the finite envelope",
        )?;
        require(
            (1..=32).contains(&self.maximum_turns),
            "Agent turn ceiling must be 1–32",
        )?;
        require(
            self.maximum_row_changes <= 5000,
            "Row-change ceiling exceeds 5000",
        )?;
        require(
            self.objective.chars().count() <= 4000,
            "Agent objective exceeds 4000 characters",
        )?;
        for value in [
            &self.launch_fingerprint,
            &self.dataset_fingerprint,
            &self.development_evidence_fingerprint,
        ] {
            require(
                value.strip_prefix("sha256:").is_some_and(|v| {
                    v.len() == 64
                        && v.bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                }),
                "Agent inputs require canonical artifact fingerprints",
            )?;
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String, OptimizationError> {
        self.validate()?;
        fingerprint(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowRemoval {
    pub row_id: String,
    pub reason: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerationTarget {
    pub template_row_id: String,
    pub instruction: String,
    pub count: u32,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DatasetEditProposal {
    pub summary: String,
    pub stop: bool,
    pub removals: Vec<RowRemoval>,
    pub additions: Vec<GenerationTarget>,
}

impl DatasetEditProposal {
    pub fn validate(
        &self,
        scope: &AgentAnalysisScope,
        inspected_rows: &BTreeSet<String>,
        inspected_evidence: &BTreeSet<String>,
    ) -> Result<(), OptimizationError> {
        scope.validate()?;
        text(&self.summary)?;
        require(
            self.summary.chars().count() <= 400,
            "Public decision summary exceeds 400 characters",
        )?;
        require(
            self.removals.len() <= 5000 && self.additions.len() <= 5000,
            "Too many edit targets",
        )?;
        let changes =
            self.additions
                .iter()
                .try_fold(self.removals.len() as u64, |total, target| {
                    require(target.count > 0, "Generation target must request rows")?;
                    total.checked_add(u64::from(target.count)).ok_or_else(|| {
                        OptimizationError::Validation("Row-change count overflow".into())
                    })
                })?;
        require(
            changes <= u64::from(scope.maximum_row_changes),
            "Proposal exceeds the remaining row-change budget",
        )?;
        require(
            if self.stop { changes == 0 } else { changes > 0 },
            "Stop requires no edits; continuation requires an explicit edit",
        )?;
        let mut removed = BTreeSet::new();
        for row in &self.removals {
            require(removed.insert(&row.row_id), "Repeated row removal")?;
            require(
                inspected_rows.contains(&row.row_id),
                "Removal references an uninspected training row",
            )?;
            text(&row.reason)?;
            evidence(&row.evidence_ids, inspected_evidence)?;
        }
        for target in &self.additions {
            require(
                inspected_rows.contains(&target.template_row_id),
                "Generation references an uninspected training template",
            )?;
            text(&target.instruction)?;
            evidence(&target.evidence_ids, inspected_evidence)?;
        }
        Ok(())
    }
}

fn text(value: &str) -> Result<(), OptimizationError> {
    require(
        !value.trim().is_empty() && value.chars().count() <= 2000,
        "A concise non-empty public explanation is required",
    )
}

fn evidence(ids: &[String], inspected: &BTreeSet<String>) -> Result<(), OptimizationError> {
    require(
        !ids.is_empty() && ids.len() <= 20 && ids.iter().all(|id| inspected.contains(id)),
        "Edits must reference inspected development evidence",
    )?;
    require(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        "Repeated development evidence identity",
    )
}

/// Payload is native-task owned and goes only to the authorized Agent, never
/// the project activity stream. Adapters must fence both dataset and evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectionItem {
    pub id: String,
    pub fingerprint: String,
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectionPage {
    pub items: Vec<InspectionItem>,
    pub next_offset: Option<u64>,
}

impl InspectionPage {
    pub fn validate(&self, limit: u32) -> Result<(), OptimizationError> {
        require(
            self.items.len() <= limit as usize,
            "Inspection returned more than the requested page",
        )?;
        require(
            serde_json::to_vec(self)?.len() <= 262144,
            "Agent inspection page exceeds 256 KiB",
        )?;
        let mut ids = BTreeSet::new();
        for item in &self.items {
            require(
                !item.id.is_empty() && item.id.len() <= 128 && ids.insert(&item.id),
                "Invalid or repeated inspection identity",
            )?;
            require(
                item.fingerprint == fingerprint(&item.content)?,
                "Inspection content fingerprint does not match",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordedAgentTool {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    pub result: Value,
    pub failed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_microusd: Option<u64>,
}

impl AgentTokenUsage {
    pub fn exceeds(&self, input: u64, output: u64, cost: u64) -> bool {
        self.input_tokens.is_some_and(|v| v > input)
            || self.output_tokens.is_some_and(|v| v > output)
            || self.cost_microusd.is_some_and(|v| v > cost)
    }
}

/// Unknown/partial outcomes retain at least their reservation; a reported
/// overrun is still charged in full even when the operation was interrupted.
pub fn conservative_charge(reported: Option<u64>, ceiling: u64, interrupted: bool) -> u64 {
    if interrupted {
        reported.unwrap_or(ceiling).max(ceiling)
    } else {
        reported.unwrap_or(ceiling)
    }
}

/// One reserved remote call. The same completed call must never be dispatched
/// again. Interrupted calls remain charged at their reserved token ceilings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentCallReservation {
    pub id: Uuid,
    pub scope_fingerprint: String,
    pub sequence: u32,
    pub request_fingerprint: String,
    pub input_token_ceiling: u64,
    pub output_token_ceiling: u64,
    pub cost_ceiling_microusd: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTurnRecord {
    pub call: AgentCallReservation,
    pub explanations: Vec<String>,
    pub tools: Vec<RecordedAgentTool>,
    pub usage: AgentTokenUsage,
    pub proposal: Option<DatasetEditProposal>,
    pub interrupted: bool,
}

impl AgentTurnRecord {
    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(
            !self.call.id.is_nil() && (1..=32).contains(&self.call.sequence),
            "Invalid Agent call identity",
        )?;
        require(
            self.tools.len() <= 16 && self.explanations.len() <= 16,
            "Agent turn exceeds its response bounds",
        )?;
        require(
            serde_json::to_vec(self)?.len() <= 2_097_152,
            "Agent turn record exceeds 2 MiB",
        )?;
        require(
            !self.interrupted || self.proposal.is_none(),
            "An interrupted Agent call cannot authorize edits",
        )?;
        require(
            self.interrupted
                || !self.usage.exceeds(
                    self.call.input_token_ceiling,
                    self.call.output_token_ceiling,
                    self.call.cost_ceiling_microusd,
                ),
            "An Agent overrun cannot authorize edits",
        )?;
        let accepted: Vec<_> = self
            .tools
            .iter()
            .filter(|tool| tool.name == "propose_dataset_edits" && !tool.failed)
            .collect();
        require(
            accepted.len() <= 1,
            "Multiple accepted proposals in one Agent turn",
        )?;
        if let Some(proposal) = &self.proposal {
            require(
                accepted.len() == 1
                    && serde_json::from_value::<DatasetEditProposal>(
                        accepted[0].arguments.clone(),
                    )? == *proposal
                    && accepted[0].result.get("accepted").and_then(Value::as_bool) == Some(true),
                "Proposal has no matching accepted Agent tool call",
            )?;
        } else if !self.interrupted {
            require(
                accepted.is_empty(),
                "Accepted Agent tool proposal is missing from its turn",
            )?;
        }
        Ok(())
    }
}
