use crate::models::ModelSelection;
use factory_core::*;
use serde::{Deserialize, Serialize};

pub const ROLE: &str = "direct";
pub const CAPABILITY: &str = "direct-sessions-v1";
pub const INTENT_APPROVAL: &str = "factory_direct_intent";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectIntent {
    pub title: String,
    pub prompt: String,
    pub enqueue: bool,
}

impl DirectIntent {
    pub fn from_permission(question: &ExternalQuestion) -> Option<Self> {
        if question.kind != QuestionKind::Permission || question.payload["action"] != INTENT_APPROVAL {
            return None;
        }
        serde_json::from_value(question.payload["metadata"]["direct_intent"].clone()).ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectState {
    Open,
    Archiving,
    Archived,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Direct {
    pub id: CommandId,
    pub project_id: ProjectId,
    pub state: DirectState,
    pub created_unix_ms: u64,
    pub stream_id: Option<StreamId>,
    pub selection: Option<DirectSelection>,
    pub root_command_id: Option<CommandId>,
    pub current_command_id: Option<CommandId>,
    pub preparation_failed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectSelection {
    pub runner_id: RunnerId,
    pub model: Option<ModelSelection>,
    /// None uses the project's optional MCP defaults.
    pub optional_mcps: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDirect {
    pub request_id: RequestId,
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptDirect {
    pub request_id: RequestId,
    pub prompt: String,
    pub selection: Option<DirectSelection>,
    #[serde(default)]
    pub images: Vec<PromptImage>,
    #[serde(default)]
    pub files: Vec<PromptFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDirectArchived {
    pub request_id: RequestId,
    pub archived: bool,
}
