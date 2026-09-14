use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Invalid, require};

pub const ACTIVITY_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
    Desktop,
    Cli,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEventState {
    Started,
    Progress,
    Succeeded,
    Failed,
}

/// A concise, reviewable explanation of an automated choice. This is an
/// operational summary, never private model chain-of-thought or native row
/// content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityNarrativeOrigin {
    Agent,
    Generation,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityNarrativeKind {
    Intent,
    Reasoning,
    Action,
    Observation,
    Decision,
    NextStep,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityNarrative {
    pub origin: ActivityNarrativeOrigin,
    pub kind: ActivityNarrativeKind,
    pub summary: String,
}

impl ActivityNarrative {
    pub fn new(
        origin: ActivityNarrativeOrigin,
        kind: ActivityNarrativeKind,
        summary: impl Into<String>,
    ) -> Result<Self, Invalid> {
        let value = Self {
            origin,
            kind,
            summary: summary.into(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), Invalid> {
        require(
            !self.summary.trim().is_empty()
                && self.summary.trim() == self.summary
                && self.summary.chars().count() <= 400
                && !self.summary.chars().any(char::is_control),
            "Activity narrative must contain 1–400 canonical printable characters.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityReference {
    pub kind: String,
    pub id: String,
}

impl ActivityReference {
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Result<Self, Invalid> {
        let value = Self {
            kind: kind.into(),
            id: id.into(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), Invalid> {
        validate_key(&self.kind, "Activity reference kind")?;
        require(
            !self.id.trim().is_empty()
                && self.id.chars().count() <= 200
                && !self.id.chars().any(char::is_control),
            "Activity reference identity must contain 1–200 printable characters.",
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityFailure {
    pub code: String,
    pub message: String,
}

impl ActivityFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Result<Self, Invalid> {
        let value = Self {
            code: code.into(),
            message: message.into(),
        };
        validate_key(&value.code, "Activity failure code")?;
        require(
            !value.message.trim().is_empty()
                && value.message.chars().count() <= 1_000
                && !value.message.chars().any(char::is_control),
            "Activity failure message must contain 1–1000 printable characters.",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectActivityEvent {
    pub schema_version: u32,
    pub id: Uuid,
    pub action_id: Uuid,
    pub project_id: Uuid,
    pub sequence: u32,
    pub operation: String,
    pub source: ActivitySource,
    pub state: ActivityEventState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narrative: Option<ActivityNarrative>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ActivityReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<ActivityFailure>,
    pub created_at: DateTime<Utc>,
    pub previous_event_fingerprint: Option<String>,
    pub fingerprint: String,
}

#[allow(clippy::too_many_arguments)]
impl ProjectActivityEvent {
    pub fn create(
        action_id: Uuid,
        project_id: Uuid,
        sequence: u32,
        operation: impl Into<String>,
        source: ActivitySource,
        state: ActivityEventState,
        stage: Option<String>,
        completed: Option<u64>,
        total: Option<u64>,
        narrative: Option<ActivityNarrative>,
        references: Vec<ActivityReference>,
        failure: Option<ActivityFailure>,
        created_at: DateTime<Utc>,
        previous_event_fingerprint: Option<String>,
    ) -> Result<Self, Invalid> {
        let mut value = Self {
            schema_version: ACTIVITY_EVENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            action_id,
            project_id,
            sequence,
            operation: operation.into(),
            source,
            state,
            stage,
            completed,
            total,
            narrative,
            references,
            failure,
            created_at,
            previous_event_fingerprint,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), Invalid> {
        self.validate_fields()?;
        require(
            self.reproduce_fingerprint()? == self.fingerprint,
            "Activity event fingerprint changed.",
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            ActivityEventState::Succeeded | ActivityEventState::Failed
        )
    }

    fn reproduce_fingerprint(&self) -> Result<String, Invalid> {
        let mut value = self.clone();
        value.fingerprint.clear();
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| Invalid(format!("Could not fingerprint activity event: {error}")))?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    fn validate_fields(&self) -> Result<(), Invalid> {
        require(
            self.schema_version == ACTIVITY_EVENT_SCHEMA_VERSION,
            "Unsupported activity event schema.",
        )?;
        require(
            !self.id.is_nil() && !self.action_id.is_nil() && !self.project_id.is_nil(),
            "Activity identities cannot be empty.",
        )?;
        validate_key(&self.operation, "Activity operation")?;
        require(
            self.sequence > 0 && (self.sequence == 1) == self.previous_event_fingerprint.is_none(),
            "Activity sequence and predecessor are invalid.",
        )?;
        if let Some(previous) = &self.previous_event_fingerprint {
            crate::validate_hash(previous)?;
        }
        let start = self.sequence == 1;
        require(
            start == (self.state == ActivityEventState::Started),
            "An action must have exactly one first started event.",
        )?;
        require(
            (self.completed.is_none() && self.total.is_none())
                || matches!((self.completed, self.total), (Some(done), Some(total)) if total > 0 && done <= total),
            "Activity progress counters must be a complete bounded pair.",
        )?;
        match self.state {
            ActivityEventState::Started => require(
                self.stage.is_none()
                    && self.failure.is_none()
                    && self.completed.is_none()
                    && self.narrative.is_none(),
                "Started events cannot contain progress or failure.",
            )?,
            ActivityEventState::Progress => {
                require(
                    self.stage.is_some() && self.failure.is_none(),
                    "Progress events require a stage and cannot contain failure.",
                )?;
                validate_key(self.stage.as_deref().unwrap_or_default(), "Activity stage")?;
                if let Some(narrative) = &self.narrative {
                    narrative.validate()?;
                }
            }
            ActivityEventState::Succeeded => require(
                self.stage.is_none()
                    && self.failure.is_none()
                    && self.completed.is_none()
                    && self.narrative.is_none(),
                "Succeeded events cannot contain progress or failure.",
            )?,
            ActivityEventState::Failed => require(
                self.stage.is_none()
                    && self.failure.is_some()
                    && self.completed.is_none()
                    && self.narrative.is_none(),
                "Failed events require a failure and cannot contain progress.",
            )?,
        }
        require(
            self.references.len() <= 32,
            "Activity events may contain at most 32 references.",
        )?;
        let mut seen = BTreeSet::new();
        for reference in &self.references {
            reference.validate()?;
            require(
                seen.insert((&reference.kind, &reference.id)),
                "Activity references must be unique.",
            )?;
        }
        if let Some(failure) = &self.failure {
            validate_key(&failure.code, "Activity failure code")?;
            require(
                !failure.message.trim().is_empty()
                    && failure.message.chars().count() <= 1_000
                    && !failure.message.chars().any(char::is_control),
                "Invalid activity failure message.",
            )?;
        }
        if !self.fingerprint.is_empty() {
            crate::validate_hash(&self.fingerprint)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectAction {
    pub action_id: Uuid,
    pub project_id: Uuid,
    pub operation: String,
    pub source: ActivitySource,
    pub state: ActivityEventState,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub references: Vec<ActivityReference>,
    pub events: Vec<ProjectActivityEvent>,
}

impl ProjectAction {
    pub fn replay(events: Vec<ProjectActivityEvent>) -> Result<Self, Invalid> {
        let first = events
            .first()
            .ok_or_else(|| Invalid("Activity action has no events.".into()))?;
        let mut previous = None;
        let mut references = vec![];
        let mut keys = BTreeSet::new();
        for (index, event) in events.iter().enumerate() {
            event.validate_integrity()?;
            require(
                event.action_id == first.action_id
                    && event.project_id == first.project_id
                    && event.operation == first.operation
                    && event.source == first.source
                    && event.sequence == index as u32 + 1
                    && event.previous_event_fingerprint == previous,
                "Activity action chain is inconsistent.",
            )?;
            require(
                index + 1 == events.len() || !event.is_terminal(),
                "Activity action contains events after a terminal outcome.",
            )?;
            for reference in &event.references {
                if keys.insert((reference.kind.clone(), reference.id.clone())) {
                    references.push(reference.clone());
                }
            }
            previous = Some(event.fingerprint.clone());
        }
        let last = events.last().expect("nonempty activity");
        Ok(Self {
            action_id: first.action_id,
            project_id: first.project_id,
            operation: first.operation.clone(),
            source: first.source,
            state: last.state,
            started_at: first.created_at,
            finished_at: last.is_terminal().then_some(last.created_at),
            references,
            events,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectActivityLog {
    pub project_id: Uuid,
    pub actions: Vec<ProjectAction>,
}

fn validate_key(value: &str, label: &str) -> Result<(), Invalid> {
    require(
        !value.is_empty()
            && value.len() <= 80
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            }),
        &format!("{label} must be a lowercase stable key."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_events_have_unique_ids_and_a_verified_chain() {
        let action = Uuid::new_v4();
        let project = Uuid::new_v4();
        let first = ProjectActivityEvent::create(
            action,
            project,
            1,
            "optimization.resume",
            ActivitySource::Desktop,
            ActivityEventState::Started,
            None,
            None,
            None,
            None,
            vec![ActivityReference::new("run", Uuid::new_v4().to_string()).unwrap()],
            None,
            Utc::now(),
            None,
        )
        .unwrap();
        let second = ProjectActivityEvent::create(
            action,
            project,
            2,
            "optimization.resume",
            ActivitySource::Desktop,
            ActivityEventState::Progress,
            Some("training".into()),
            Some(2),
            Some(10),
            None,
            vec![],
            None,
            Utc::now(),
            Some(first.fingerprint.clone()),
        )
        .unwrap();
        let third = ProjectActivityEvent::create(
            action,
            project,
            3,
            "optimization.resume",
            ActivitySource::Desktop,
            ActivityEventState::Succeeded,
            None,
            None,
            None,
            None,
            vec![],
            None,
            Utc::now(),
            Some(second.fingerprint.clone()),
        )
        .unwrap();
        let replay = ProjectAction::replay(vec![first.clone(), second, third]).unwrap();
        assert_eq!(replay.state, ActivityEventState::Succeeded);
        assert_ne!(replay.events[0].id, replay.events[1].id);
        let mut tampered = first;
        tampered.operation = "optimization.cancel".into();
        assert!(tampered.validate_integrity().is_err());
    }

    #[test]
    fn event_shape_rejects_secrets_and_invalid_terminal_sequences() {
        assert!(ActivityReference::new("api_key", "secret\nvalue").is_err());
        assert!(ActivityFailure::new("provider", "Bearer\nsecret").is_err());
        assert!(
            ProjectActivityEvent::create(
                Uuid::new_v4(),
                Uuid::new_v4(),
                1,
                "Optimization Resume",
                ActivitySource::Desktop,
                ActivityEventState::Started,
                None,
                None,
                None,
                None,
                vec![],
                None,
                Utc::now(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn narrative_is_bounded_and_part_of_the_verified_event() {
        let first = ProjectActivityEvent::create(
            Uuid::new_v4(),
            Uuid::new_v4(),
            2,
            "optimization.run",
            ActivitySource::Desktop,
            ActivityEventState::Progress,
            Some("creating_candidate".into()),
            None,
            None,
            Some(
                ActivityNarrative::new(
                    ActivityNarrativeOrigin::System,
                    ActivityNarrativeKind::Reasoning,
                    "Use one conservative candidate before expanding the search.",
                )
                .unwrap(),
            ),
            vec![],
            None,
            Utc::now(),
            Some(format!("sha256:{}", "a".repeat(64))),
        )
        .unwrap();
        assert!(first.validate_integrity().is_ok());
        let mut tampered = first;
        tampered.narrative.as_mut().unwrap().summary = "Different explanation.".into();
        assert!(tampered.validate_integrity().is_err());
        assert!(
            ActivityNarrative::new(
                ActivityNarrativeOrigin::Agent,
                ActivityNarrativeKind::Decision,
                "contains\na hidden payload"
            )
            .is_err()
        );
    }
}
