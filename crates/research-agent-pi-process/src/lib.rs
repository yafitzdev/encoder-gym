//! Rust host adapter for the pinned Pi JSONL sidecar.

use std::{path::PathBuf, process::Stdio, time::Duration};

use agent_runtime_core::{
    AgentAdapterError as ResearchAdapterError, AgentEvent as ResearchAgentEvent,
    AgentMessage as ResearchAgentMessage, AgentRequest as ResearchAgentRequest,
    AgentRuntime as ResearchAgentRuntime, AgentSession as ResearchAgentSession, AgentToolRequest,
    AgentToolResult, BoxFuture,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const PI_PACKAGE_VERSION: &str = "0.84.4";

#[derive(Debug, Clone)]
pub struct PiProcessRuntime {
    node_executable: PathBuf,
    sidecar_script: PathBuf,
    scripted_turns: Option<Value>,
    startup_timeout: Duration,
}

impl PiProcessRuntime {
    pub fn new(node_executable: PathBuf, sidecar_script: PathBuf) -> Self {
        Self {
            node_executable,
            sidecar_script,
            scripted_turns: None,
            startup_timeout: Duration::from_secs(15),
        }
    }

    pub fn with_scripted_turns(mut self, scripted_turns: Value) -> Self {
        self.scripted_turns = Some(scripted_turns);
        self
    }
}

impl ResearchAgentRuntime for PiProcessRuntime {
    fn start(
        &self,
        request: ResearchAgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn ResearchAgentSession>, ResearchAdapterError>> {
        let runtime = self.clone();
        Box::pin(async move {
            if request.protocol_version != PROTOCOL_VERSION {
                return Err(adapter_error(format!(
                    "unsupported Pi protocol version {}; expected {PROTOCOL_VERSION}",
                    request.protocol_version
                )));
            }
            let mut child = Command::new(&runtime.node_executable)
                .arg(&runtime.sidecar_script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(adapter_error)?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| adapter_error("Pi sidecar stdin was not piped"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| adapter_error("Pi sidecar stdout was not piped"))?;
            let mut session = PiProcessSession {
                run_id: request.run_id,
                child,
                stdin,
                lines: BufReader::new(stdout).lines(),
                completed: false,
            };
            let ready = tokio::time::timeout(runtime.startup_timeout, session.read_output())
                .await
                .map_err(|_| adapter_error("Pi sidecar startup timed out"))??;
            match ready {
                OutputMessage::Ready {
                    protocol_version,
                    pi_package_version,
                } if protocol_version == PROTOCOL_VERSION
                    && pi_package_version == PI_PACKAGE_VERSION => {}
                OutputMessage::Ready {
                    protocol_version,
                    pi_package_version,
                } => {
                    return Err(adapter_error(format!(
                        "Pi sidecar identity mismatch: protocol {protocol_version}, package {pi_package_version}"
                    )));
                }
                _ => {
                    return Err(adapter_error(
                        "Pi sidecar did not begin with a ready message",
                    ));
                }
            }
            session
                .write_input(&InputMessage::Start {
                    request: StartRequest {
                        protocol_version: request.protocol_version,
                        capability_set: request.capability_set,
                        run_id: request.run_id,
                        run_specification_fingerprint: request.run_specification_fingerprint,
                        provider: request.provider,
                        model: request.model,
                        api_key_env: request.api_key_env,
                        system_prompt: request.system_prompt,
                        initial_prompt: request.initial_prompt,
                        max_model_turns: request.max_model_turns,
                        scripted_turns: runtime.scripted_turns,
                    },
                })
                .await?;
            Ok(Box::new(session) as Box<dyn ResearchAgentSession>)
        })
    }
}

struct PiProcessSession {
    run_id: Uuid,
    child: Child,
    stdin: tokio::process::ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    completed: bool,
}

impl PiProcessSession {
    async fn write_input(&mut self, message: &InputMessage) -> Result<(), ResearchAdapterError> {
        if self.completed {
            return Err(adapter_error("Pi sidecar session is already complete"));
        }
        let mut encoded = serde_json::to_vec(message).map_err(adapter_error)?;
        encoded.push(b'\n');
        self.stdin
            .write_all(&encoded)
            .await
            .map_err(adapter_error)?;
        self.stdin.flush().await.map_err(adapter_error)
    }

    async fn read_output(&mut self) -> Result<OutputMessage, ResearchAdapterError> {
        let line = self
            .lines
            .next_line()
            .await
            .map_err(adapter_error)?
            .ok_or_else(|| adapter_error("Pi sidecar closed stdout unexpectedly"))?;
        serde_json::from_str(&line).map_err(adapter_error)
    }

    fn validate_run(&self, run_id: Uuid) -> Result<(), ResearchAdapterError> {
        if run_id == self.run_id {
            Ok(())
        } else {
            Err(adapter_error(format!(
                "Pi sidecar emitted run {run_id}, expected {}",
                self.run_id
            )))
        }
    }
}

impl Drop for PiProcessSession {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl ResearchAgentSession for PiProcessSession {
    fn next_message(
        &mut self,
    ) -> BoxFuture<'_, Result<ResearchAgentMessage, ResearchAdapterError>> {
        Box::pin(async move {
            match self.read_output().await? {
                OutputMessage::Ready { .. } => Err(adapter_error(
                    "Pi sidecar emitted a duplicate ready message",
                )),
                OutputMessage::Event { event } => {
                    let (run_id, event) = event.into_domain()?;
                    self.validate_run(run_id)?;
                    Ok(ResearchAgentMessage::Event { event })
                }
                OutputMessage::ToolRequest {
                    run_id,
                    call_id,
                    name,
                    arguments,
                } => {
                    self.validate_run(run_id)?;
                    Ok(ResearchAgentMessage::ToolRequest {
                        request: AgentToolRequest {
                            external_call_id: call_id,
                            name,
                            arguments,
                        },
                    })
                }
                OutputMessage::Completed { run_id } => {
                    self.validate_run(run_id)?;
                    self.completed = true;
                    Ok(ResearchAgentMessage::Completed)
                }
                OutputMessage::Failed { run_id, message } => {
                    if let Some(run_id) = run_id {
                        self.validate_run(run_id)?;
                    }
                    self.completed = true;
                    Ok(ResearchAgentMessage::Failed { message })
                }
            }
        })
    }

    fn send_tool_result(
        &mut self,
        external_call_id: &str,
        result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let external_call_id = external_call_id.to_owned();
        Box::pin(async move {
            self.write_input(&InputMessage::ToolResult {
                call_id: external_call_id,
                result: WireToolResult {
                    content: result.content,
                    details: result.details,
                    terminate: result.terminate,
                },
            })
            .await
        })
    }

    fn send_tool_error(
        &mut self,
        external_call_id: &str,
        message: &str,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let external_call_id = external_call_id.to_owned();
        let message = message.to_owned();
        Box::pin(async move {
            self.write_input(&InputMessage::ToolError {
                call_id: external_call_id,
                message,
            })
            .await
        })
    }

    fn cancel(&mut self, run_id: Uuid) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        Box::pin(async move {
            self.validate_run(run_id)?;
            self.write_input(&InputMessage::Cancel { run_id }).await
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputMessage {
    Start {
        request: StartRequest,
    },
    ToolResult {
        #[serde(rename = "callId")]
        call_id: String,
        result: WireToolResult,
    },
    ToolError {
        #[serde(rename = "callId")]
        call_id: String,
        message: String,
    },
    Cancel {
        #[serde(rename = "runId")]
        run_id: Uuid,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    protocol_version: u32,
    capability_set: String,
    run_id: Uuid,
    run_specification_fingerprint: String,
    provider: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key_env: Option<String>,
    system_prompt: String,
    initial_prompt: String,
    max_model_turns: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    scripted_turns: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WireToolResult {
    content: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    details: Value,
    #[serde(skip_serializing_if = "is_false")]
    terminate: bool,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputMessage {
    Ready {
        #[serde(rename = "protocolVersion")]
        protocol_version: u32,
        #[serde(rename = "piPackageVersion")]
        pi_package_version: String,
    },
    Event {
        event: WireEvent,
    },
    ToolRequest {
        #[serde(rename = "runId")]
        run_id: Uuid,
        #[serde(rename = "callId")]
        call_id: String,
        name: String,
        arguments: Value,
    },
    Completed {
        #[serde(rename = "runId")]
        run_id: Uuid,
    },
    Failed {
        #[serde(rename = "runId")]
        run_id: Option<Uuid>,
        message: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireEvent {
    AgentStarted {
        #[serde(rename = "runId")]
        run_id: Uuid,
    },
    TurnStarted {
        #[serde(rename = "runId")]
        run_id: Uuid,
        sequence: u32,
    },
    TurnCompleted {
        #[serde(rename = "runId")]
        run_id: Uuid,
        sequence: u32,
        #[serde(rename = "inputTokens")]
        input_tokens: u64,
        #[serde(rename = "outputTokens")]
        output_tokens: u64,
        #[serde(rename = "costMicrousd")]
        cost_microusd: u64,
    },
    AgentText {
        #[serde(rename = "runId")]
        run_id: Uuid,
        text: String,
    },
    ToolStarted {
        #[serde(rename = "runId")]
        run_id: Uuid,
        #[serde(rename = "callId")]
        call_id: String,
        name: String,
        #[serde(rename = "arguments")]
        _arguments: Value,
    },
    ToolCompleted {
        #[serde(rename = "runId")]
        run_id: Uuid,
        #[serde(rename = "callId")]
        call_id: String,
        name: String,
        failed: bool,
    },
    AgentFinished {
        #[serde(rename = "runId")]
        run_id: Uuid,
        turns: u32,
        aborted: bool,
    },
}

impl WireEvent {
    fn into_domain(self) -> Result<(Uuid, ResearchAgentEvent), ResearchAdapterError> {
        Ok(match self {
            Self::AgentStarted { run_id } => (run_id, ResearchAgentEvent::AgentStarted),
            Self::TurnStarted { run_id, sequence } => {
                (run_id, ResearchAgentEvent::ModelTurnStarted { sequence })
            }
            Self::TurnCompleted {
                run_id,
                sequence,
                input_tokens,
                output_tokens,
                cost_microusd,
            } => (
                run_id,
                ResearchAgentEvent::ModelTurnCompleted {
                    sequence,
                    input_tokens,
                    output_tokens,
                    cost_microusd,
                },
            ),
            Self::AgentText { run_id, text } => (run_id, ResearchAgentEvent::AgentText { text }),
            Self::ToolStarted {
                run_id,
                call_id,
                name,
                ..
            } => (
                run_id,
                ResearchAgentEvent::ToolStarted {
                    external_call_id: call_id,
                    name,
                },
            ),
            Self::ToolCompleted {
                run_id,
                call_id,
                name,
                failed,
            } => (
                run_id,
                ResearchAgentEvent::ToolCompleted {
                    external_call_id: call_id,
                    name,
                    failed,
                },
            ),
            Self::AgentFinished {
                run_id,
                turns,
                aborted,
            } => (run_id, ResearchAgentEvent::AgentFinished { turns, aborted }),
        })
    }
}

fn adapter_error(error: impl std::fmt::Display) -> ResearchAdapterError {
    ResearchAdapterError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_decodes_tool_arguments_without_exposing_pi_types() {
        let run_id = Uuid::new_v4();
        let message: OutputMessage = serde_json::from_value(serde_json::json!({
            "type": "tool_request",
            "runId": run_id,
            "callId": "call-1",
            "name": "search_web",
            "arguments": {"query": "authentic support", "maximumResults": 2}
        }))
        .expect("tool request");
        match message {
            OutputMessage::ToolRequest {
                run_id: actual,
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(actual, run_id);
                assert_eq!(call_id, "call-1");
                assert_eq!(name, "search_web");
                assert_eq!(arguments["maximumResults"], 2);
            }
            _ => panic!("wrong message variant"),
        }
    }

    #[test]
    fn host_messages_use_the_versioned_camel_case_wire_contract() {
        let encoded = serde_json::to_value(InputMessage::ToolResult {
            call_id: "call-2".into(),
            result: WireToolResult {
                content: serde_json::json!({"ok": true}),
                details: Value::Null,
                terminate: false,
            },
        })
        .expect("serialize result");
        assert_eq!(encoded["type"], "tool_result");
        assert_eq!(encoded["callId"], "call-2");
        assert!(encoded.get("call_id").is_none());
        assert!(encoded["result"].get("details").is_none());
        assert!(encoded["result"].get("terminate").is_none());

        let start = serde_json::to_value(InputMessage::Start {
            request: StartRequest {
                protocol_version: PROTOCOL_VERSION,
                capability_set: "dataset_architect_v1".into(),
                run_id: Uuid::new_v4(),
                run_specification_fingerprint: "sha256:run".into(),
                provider: "fake".into(),
                model: "scripted".into(),
                api_key_env: None,
                system_prompt: "policy".into(),
                initial_prompt: "start".into(),
                max_model_turns: 4,
                scripted_turns: None,
            },
        })
        .expect("serialize start");
        assert_eq!(start["request"]["capabilitySet"], "dataset_architect_v1");
    }
}
