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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionState {
    Requested,
    Admitted,
    Running,
    Succeeded,
    Failed,
    Rejected,
    Unknown,
    Aborted,
}
impl CompactionState {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Rejected | Self::Aborted
        )
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Compaction {
    pub request_id: RequestId,
    pub command_id: CommandId,
    pub session_id: String,
    pub message_id: String,
    pub reason: String,
    pub revision: Revision,
    pub state: CompactionState,
    pub summary: String,
}
pub fn compaction_message_id(id: RequestId) -> String {
    format!("msg_factory_compact_{}", id.0.simple())
}

/// Untruncated projected OpenCode messages. Binary objects referenced by messages are not embedded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveBatch {
    pub session_id: String,
    pub revision: u64,
    pub captured_unix_ms: u64,
    pub final_scan_complete: bool,
    pub messages: Vec<serde_json::Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveStatus {
    pub session_id: String,
    pub revision: u64,
    pub captured_unix_ms: u64,
    pub final_scan_complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivedMessage {
    pub sequence: u64,
    pub sample_revision: u64,
    pub captured_unix_ms: u64,
    pub message: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivePage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination: Option<RuntimeTermination>,
    pub status: Option<ArchiveStatus>,
    pub entries: Vec<ArchivedMessage>,
    pub next_after: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeNote {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<KnowledgeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<KnowledgeId>,
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
pub const CONTAINER_FREEZE_CAPABILITY: &str = "container-freeze-v1";
pub const RUNTIME_TERMINATION_CAPABILITY: &str = "runtime-termination-v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRef {
    pub container_id: String,
    pub started_at: String,
}
/// Durable runner-observed identity, saved centrally before the first agent write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeBinding {
    pub command_id: CommandId,
    pub stream_id: StreamId,
    pub runner_id: RunnerId,
    pub ownership_epoch: u64,
    pub runtime: RuntimeRef,
    pub daemon_id: String,
    pub endpoint: String,
    pub workspace: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationState {
    Requested,
    Unknown,
    Stopped,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemovalProof {
    pub runtime: RuntimeRef,
    pub daemon_id: String,
    pub observed_unix_ms: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeTermination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<RuntimeBinding>,
    pub request_id: RequestId,
    pub command_id: CommandId,
    pub stream_id: StreamId,
    pub runner_id: RunnerId,
    pub ownership_epoch: u64,
    pub runtime: RuntimeRef,
    pub previous_request: Option<RequestId>,
    pub reason: String,
    pub state: TerminationState,
    pub summary: String,
    pub proof: Option<RemovalProof>,
    pub result: Option<CommandOutput>,
    pub archive_may_be_incomplete: bool,
}
fn pause_action() -> ControlAction {
    ControlAction::Pause
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContainmentProof {
    pub container_id: String,
    pub started_at: String,
    pub frozen: bool,
    pub observed_unix_ms: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseState {
    Requested,
    HeldBeforeExecution,
    InterruptUnverified,
    Frozen,
    ResumeRequested,
    Released,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<ControlRetry>,
    #[serde(default = "pause_action")]
    pub action: ControlAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<ContainmentProof>,
    pub stream_id: StreamId,
    pub request_id: RequestId,
    pub command_id: Option<CommandId>,
    pub state: PauseState,
    pub summary: String,
    pub previous_status: StreamState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlRetry {
    pub previous_request: RequestId,
    pub reason: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<CheckpointImport>,
    pub id: StreamId,
    pub project_id: ProjectId,
    pub revision: Revision,
    pub goal: String,
    pub runner_id: RunnerId,
    /// Incremented atomically by checkpoint-based handoff; fences former owners.
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
    Checkpoint,
    ReviewCheckpoint,
    IntegrateReview,
}
pub const REVIEW_CAPABILITY: &str = "git-review-v1";
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewJob {
    Review {
        checkpoint: CheckpointImport,
        target_branch: String,
        target_commit: String,
        test_command: String,
    },
    Integrate {
        review_command: CommandId,
        target_branch: String,
        target_commit: String,
        candidate_commit: String,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewResult {
    pub checkpoint: CommandId,
    pub target_commit: String,
    pub candidate_commit: String,
    pub tree: String,
    pub image: String,
    pub test_exit_code: i32,
    pub removal: RemovalProof,
    pub log_path: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IntegrationResult {
    pub review_command: CommandId,
    pub target_branch: String,
    pub previous_commit: String,
    pub commit: String,
}
pub const CHECKPOINT_CAPABILITY: &str = "git-checkpoint-v1";
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckpointSpec {
    pub command_id: CommandId,
    pub source_commands: Vec<CommandId>,
    pub stopped_runtimes: Vec<RemovalProof>,
    pub context: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitCheckpoint {
    pub commit: String,
    pub parent: String,
    pub tree: String,
    pub bundle_path: String,
    pub bundle_sha256: String,
    pub bundle_bytes: u64,
    pub manifest_path: String,
}
pub const CHECKPOINT_BUNDLE_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckpointImport {
    pub command_id: CommandId,
    pub commit: String,
    pub bundle_sha256: String,
    pub bundle_bytes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BundleReceipt {
    pub sha256: String,
    pub bytes: u64,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<SessionContinuation>,
    pub prompt: String,
    pub profile: ExecutionProfile,
}
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionContinuation {
    pub root_command_id: CommandId,
    pub previous_command_id: CommandId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionClosure {
    pub request_id: RequestId,
    pub root_command_id: CommandId,
    pub last_command_id: CommandId,
    pub session_id: String,
    pub reason: String,
    pub closed_unix_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SupervisorPolicy {
    pub project_id: ProjectId,
    pub stream_id: StreamId,
    pub revision: Revision,
    pub enabled: bool,
    pub remaining_evaluations: u32,
    pub interval_seconds: u32,
    pub next_unix_ms: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorAction {
    Wait,
    Compact,
    Close,
    Terminate,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorDecision {
    pub action: SupervisorAction,
    pub reason: String,
    pub memory: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SupervisorEvaluation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeRef>,
    pub command_id: CommandId,
    pub project_id: ProjectId,
    pub target_command_id: CommandId,
    pub target_stream_revision: Revision,
    pub policy_revision: Revision,
    pub created_unix_ms: u64,
    pub resolved: bool,
    pub outcome: String,
    pub decision: Option<SupervisorDecision>,
    pub action_request_id: Option<RequestId>,
    pub knowledge_id: Option<KnowledgeId>,
}
impl RunnerCommand {
    pub fn requires_checkpoint(&self) -> bool {
        self.kind == CommandKind::Checkpoint
            || self
                .workspace_spec
                .as_ref()
                .is_some_and(|s| s.checkpoint.is_some())
    }
    pub fn requires_session_lifecycle(&self) -> bool {
        self.agent_spec
            .as_ref()
            .is_some_and(|s| s.read_only || s.continuation.is_some())
    }
    pub fn session_root(&self) -> CommandId {
        self.agent_spec
            .as_ref()
            .and_then(|s| s.continuation.as_ref())
            .map_or(self.id, |c| c.root_command_id)
    }
    pub fn session_id(&self) -> String {
        agent_session_id(self.session_root())
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<CheckpointImport>,
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
    Review { review: ReviewResult },
    Integration { integration: IntegrationResult },
    ReviewError { review_error: String },
    Checkpoint { checkpoint: GitCheckpoint },
    CheckpointError { checkpoint_error: String },
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TicketId>,
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SchedulerPolicy {
    pub project_id: ProjectId,
    pub revision: Revision,
    pub enabled: bool,
    pub runner_ids: Vec<RunnerId>,
    pub max_parallel: u8,
    pub remaining_starts: u32,
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
