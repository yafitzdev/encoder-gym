//! Product-neutral contract for a bounded interactive model/tool runtime.

use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("agent runtime adapter failed: {0}")]
pub struct AgentAdapterError(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRequest {
    pub protocol_version: u32,
    pub capability_set: String,
    pub run_id: Uuid,
    pub run_specification_fingerprint: String,
    pub provider: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub system_prompt: String,
    pub initial_prompt: String,
    pub max_model_turns: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStarted,
    ModelTurnStarted {
        sequence: u32,
    },
    ModelTurnCompleted {
        sequence: u32,
        input_tokens: u64,
        output_tokens: u64,
        cost_microusd: u64,
    },
    AgentText {
        text: String,
    },
    ToolStarted {
        external_call_id: String,
        name: String,
    },
    ToolCompleted {
        external_call_id: String,
        name: String,
        failed: bool,
    },
    AgentFinished {
        turns: u32,
        aborted: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolRequest {
    pub external_call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Event { event: AgentEvent },
    ToolRequest { request: AgentToolRequest },
    Completed,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolResult {
    pub content: serde_json::Value,
    #[serde(default)]
    pub details: serde_json::Value,
    #[serde(default)]
    pub terminate: bool,
}

pub trait AgentSession: Send {
    fn next_message(&mut self) -> BoxFuture<'_, Result<AgentMessage, AgentAdapterError>>;

    fn send_tool_result(
        &mut self,
        external_call_id: &str,
        result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), AgentAdapterError>>;

    fn send_tool_error(
        &mut self,
        external_call_id: &str,
        message: &str,
    ) -> BoxFuture<'_, Result<(), AgentAdapterError>>;

    fn cancel(&mut self, run_id: Uuid) -> BoxFuture<'_, Result<(), AgentAdapterError>>;
}

pub trait AgentRuntime: Send + Sync {
    fn start(
        &self,
        request: AgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn AgentSession>, AgentAdapterError>>;
}
