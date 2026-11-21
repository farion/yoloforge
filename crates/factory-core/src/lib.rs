//! Domain vocabulary. No network, database or OpenCode dependencies.
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
use uuid::Uuid;

macro_rules! id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse().map(Self)
            }
        }
    };
}
id!(ProjectId);
id!(TicketId);
id!(StreamId);
id!(RunnerId);
id!(InstanceId);
id!(RegistrationId);
id!(RequestId);
id!(BootId);
id!(CommandId);
id!(QuestionId);
id!(DeviceId);
id!(KnowledgeId);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeNote {
    pub id: KnowledgeId,
    pub project_id: ProjectId,
    pub ticket_id: Option<TicketId>,
    pub revision: Revision,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_command: Option<CommandId>,
    pub priority: u8,
    pub retired: bool,
    pub created_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeContext {
    pub notes: Vec<KnowledgeNote>,
    pub text: String,
    pub budget_bytes: usize,
    pub omitted: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    Admin,
    Operator,
    Runner,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub role: DeviceRole,
    pub runner_id: Option<RunnerId>,
    pub created_unix_ms: u64,
    pub revoked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionExcerpt {
    pub session_id: String,
    pub revision: Revision,
    pub captured_unix_ms: u64,
    pub older_omitted: bool,
    pub messages: Vec<ExcerptMessage>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExcerptMessage {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Pause,
    Resume,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseState {
    Requested,
    HeldBeforeExecution,
    InterruptUnverified,
    Released,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamControl {
    pub stream_id: StreamId,
    pub request_id: RequestId,
    pub command_id: Option<CommandId>,
    pub state: PauseState,
    pub summary: String,
    pub previous_status: StreamState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    Form,
    Permission,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Pending,
    AnswerQueued,
    Answered,
    Unknown,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExternalQuestion {
    pub external_id: String,
    pub kind: QuestionKind,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: QuestionId,
    pub command_id: CommandId,
    pub session_id: String,
    pub revision: Revision,
    pub external: ExternalQuestion,
    pub state: QuestionState,
    pub answer: Option<serde_json::Value>,
    pub answer_request: Option<RequestId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Workstream {
    pub id: StreamId,
    pub project_id: ProjectId,
    pub revision: Revision,
    pub goal: String,
    pub runner_id: RunnerId,
    /// Immutable in this first queue implementation; no automatic handoff.
    pub ownership_epoch: u64,
    /// Stream branch; created by PrepareWorkspace on the owning runner.
    pub branch: String,
    pub ticket_ids: Vec<TicketId>,
    pub status: StreamState,
    pub created_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    CheckOpenCode,
    PrepareWorkspace,
    RunAgent,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfile {
    pub toolchain: String,
    pub start_command: String,
    pub test_command: String,
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub prompt: String,
    pub profile: ExecutionProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Starting,
    Running,
    WaitingInput,
    Unknown,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentProgress {
    pub session_id: String,
    pub message_id: String,
    pub phase: AgentPhase,
    pub summary: String,
}

pub fn agent_session_id(id: CommandId) -> String {
    format!("ses_factory_{}", id.0.simple())
}
pub fn agent_message_id(id: CommandId) -> String {
    format!("msg_factory_{}", id.0.simple())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    pub project_id: ProjectId,
    pub branch: String,
    pub base_branch: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub path: String,
    pub branch: String,
    pub base_commit: String,
}

/// Untagged for compatibility with previously persisted OpenCode status results.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandOutput {
    OpenCode(OpenCodeStatus),
    Workspace { workspace: Workspace },
    WorkspaceError { workspace_error: String },
    Agent { agent: AgentProgress },
}
impl From<OpenCodeStatus> for CommandOutput {
    fn from(value: OpenCodeStatus) -> Self {
        Self::OpenCode(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandState {
    Pending,
    Accepted,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunnerCommand {
    pub id: CommandId,
    pub stream_id: StreamId,
    pub runner_id: RunnerId,
    pub ownership_epoch: u64,
    pub kind: CommandKind,
    pub state: CommandState,
    pub result: Option<CommandOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_spec: Option<WorkspaceSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_spec: Option<AgentSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_progress: Option<AgentProgress>,
    pub created_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(pub u64);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub revision: Revision,
    pub name: String,
    pub repository: String,
    pub base_branch: String,
    pub created_unix_ms: u64,
    #[serde(default)]
    pub profile: ExecutionProfile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: TicketId,
    pub project_id: ProjectId,
    pub revision: Revision,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: String,
    /// 0 = low, 1 = normal, 2 = high, 3 = urgent.
    pub priority: u8,
    pub status: TicketState,
    pub created_unix_ms: u64,
    pub updated_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerLocation {
    Local,
    Server,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Online,
    Suspect,
    Unreachable,
}

pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
pub const SUSPECT_AFTER: Duration = Duration::from_secs(30);
pub const UNREACHABLE_AFTER: Duration = Duration::from_secs(60);

impl ConnectionState {
    pub fn from_age(age: Duration) -> Self {
        if age >= UNREACHABLE_AFTER {
            Self::Unreachable
        } else if age >= SUSPECT_AFTER {
            Self::Suspect
        } else {
            Self::Online
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamState {
    Idle,
    Queued,
    Running,
    WaitingInput,
    Pausing,
    Paused,
    Blocked,
    Review,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Backlog,
    Ready,
    InProgress,
    Blocked,
    Review,
    Done,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenCodeStatus {
    NotConfigured,
    Ready { version: String },
    Unsupported { version: String },
    Unavailable,
}
