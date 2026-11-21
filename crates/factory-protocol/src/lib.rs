//! Factory v1 wire contract, separate from the OpenCode API contract.
use factory_core::*;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const VERSION_HEADER: &str = "x-factory-protocol";
pub const RUNNER_HEADER: &str = "x-factory-runner-registration";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestCompaction {
    pub request_id: RequestId,
    pub expected_stream_revision: Revision,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinueAgent {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub previous_command_id: CommandId,
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseSession {
    pub request_id: RequestId,
    pub expected_stream_revision: Revision,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSupervisor {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub stream_id: StreamId,
    pub enabled: bool,
    pub evaluation_budget: u32,
    pub interval_seconds: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportCompaction {
    pub request_id: RequestId,
    pub ownership_epoch: u64,
    pub state: CompactionState,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportArchive {
    pub ownership_epoch: u64,
    pub batch: ArchiveBatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateKnowledge {
    pub request_id: RequestId,
    pub project_id: ProjectId,
    pub ticket_id: Option<TicketId>,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_command: Option<CommandId>,
    pub priority: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetireKnowledge {
    pub request_id: RequestId,
    pub expected_revision: Revision,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateEnrollment {
    pub role: DeviceRole,
    pub runner_id: Option<RunnerId>,
    pub ttl_seconds: u32,
}
// Secret-bearing DTOs deliberately do not implement Debug.
#[derive(Serialize, Deserialize)]
pub struct Enrollment {
    pub code: String,
    pub expires_unix_ms: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemEnrollment {
    pub code: String,
    pub name: String,
}
#[derive(Serialize, Deserialize)]
pub struct DeviceCredential {
    pub device: Device,
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRunner {
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub id: RunnerId,
    pub instance_id: InstanceId,
    pub name: String,
    pub location: RunnerLocation,
    pub version: String,
    pub opencode: OpenCodeStatus,
}
pub const SESSION_LIFECYCLE_CAPABILITY: &str = "session-lifecycle-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub registration_id: RegistrationId,
    pub opencode: OpenCodeStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
    pub registration_id: RegistrationId,
    pub boot_id: BootId,
    pub heartbeat_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerView {
    pub id: RunnerId,
    pub name: String,
    pub location: RunnerLocation,
    pub version: String,
    pub connection: ConnectionState,
    pub last_seen_unix_ms: u64,
    pub opencode: OpenCodeStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub protocol_version: u16,
    pub server_version: String,
    pub boot_id: BootId,
    // This is a live registry revision, NOT a durable event replay cursor.
    pub revision: Revision,
    pub runners: Vec<RunnerView>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub tickets: Vec<Ticket>,
    /// Durable project/ticket event cursor, independent of the live runner revision.
    #[serde(default)]
    pub event_cursor: u64,
    #[serde(default)]
    pub streams: Vec<Workstream>,
    #[serde(default)]
    pub commands: Vec<RunnerCommand>,
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub controls: Vec<StreamControl>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetControl {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub action: ControlAction,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportControl {
    pub control_request: RequestId,
    pub ownership_epoch: u64,
    pub state: PauseState,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionSnapshot {
    pub ownership_epoch: u64,
    pub questions: Vec<ExternalQuestion>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerQuestion {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub answer: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerDelivery {
    pub request_id: RequestId,
    pub ownership_epoch: u64,
    pub answer_request: RequestId,
    pub state: QuestionState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateStream {
    pub request_id: RequestId,
    pub project_id: ProjectId,
    pub goal: String,
    pub runner_id: RunnerId,
    pub ticket_ids: Vec<TicketId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnqueueCommand {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub kind: CommandKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProfile {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub profile: ExecutionProfile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveAgent {
    pub ownership_epoch: u64,
    pub progress: AgentProgress,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckCommand {
    pub request_id: RequestId,
    pub ownership_epoch: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    pub request_id: RequestId,
    pub ownership_epoch: u64,
    pub result: CommandOutput,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    pub request_id: RequestId,
    pub name: String,
    pub repository: String,
    pub base_branch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProject {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub name: Option<String>,
    pub repository: Option<String>,
    pub base_branch: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportExcerpt {
    pub ownership_epoch: u64,
    pub excerpt: SessionExcerpt,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTicket {
    pub request_id: RequestId,
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: String,
    pub priority: u8,
}

/// Partial update; omitted fields retain their value. Empty strings clear text fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTicket {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub priority: Option<u8>,
    pub status: Option<TicketState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactoryEvent {
    pub sequence: u64,
    pub kind: String,
    pub aggregate_id: String,
    pub created_unix_ms: u64,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventPage {
    pub events: Vec<FactoryEvent>,
    /// Last delivered sequence, or the supplied cursor if no events were available.
    pub next_cursor: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    ProtocolMismatch,
    InvalidRequest,
    RunnerConflict,
    RegistrationExpired,
    NotFound,
    RevisionConflict,
    RequestConflict,
    Internal,
    WorkConflict,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub request_id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<Revision>,
}

#[cfg(feature = "client")]
pub mod client;
