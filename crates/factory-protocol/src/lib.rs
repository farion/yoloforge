//! Factory v1 wire contract, separate from the OpenCode API contract.
use factory_core::*;
pub mod directs;

pub const DELETE_TASK_CAPABILITY: &str = "delete-intent-v2";

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteTask {
    pub request_id: RequestId,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationRecoveryClaim {
    pub grant_id: RequestId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportRecoveredPublication {
    pub request_id: RequestId,
    pub grant_id: RequestId,
    pub ownership_epoch: u64,
    pub base_branch: String,
    pub snapshot_commit: String,
    pub target_commit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportRecoveredReviewSnapshot {
    pub request_id: RequestId,
    pub grant_id: RequestId,
    pub ownership_epoch: u64,
    pub snapshot_commit: String,
    pub unchanged_tree: bool,
}
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const VERSION_HEADER: &str = "x-factory-protocol";
pub const RUNNER_HEADER: &str = "x-factory-runner-registration";
mod desktop;
pub use desktop::*;

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
pub struct ReportArchiveBlob {
    pub ownership_epoch: u64,
    pub chunk: ArchiveBlobChunk,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateKnowledge {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<SupersedeKnowledge>,
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
pub struct SupersedeKnowledge {
    pub id: KnowledgeId,
    pub expected_revision: Revision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetireKnowledge {
    pub request_id: RequestId,
    pub expected_revision: Revision,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDevice {
    pub name: String,
    pub role: DeviceRole,
    pub runner_id: Option<RunnerId>,
}
// Secret-bearing DTOs deliberately do not implement Debug.
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
pub const REPO_SKILLS_CAPABILITY: &str = "repo-skills-v1";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectSkills {
    pub revision: factory_core::Revision,
    pub repo_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factory_skills: Vec<FactorySkillRef>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProjectSkills {
    pub request_id: factory_core::RequestId,
    pub expected_revision: factory_core::Revision,
    pub repo_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factory_skills: Vec<FactorySkillRef>,
}
pub mod skills;

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
    /// Peer IP observed by the Factory (may be a proxy or tunnel endpoint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<std::net::IpAddr>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub id: RunnerId,
    pub name: String,
    pub location: RunnerLocation,
    pub version: String,
    pub connection: ConnectionState,
    pub last_seen_unix_ms: u64,
    pub opencode: OpenCodeStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerRestartHold {
    pub request_id: RequestId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerRestartStatus {
    pub instance_id: Option<InstanceId>,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentProblem {
    pub ticket_id: TicketId,
    pub command_id: CommandId,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solution: Option<IntentSolution>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSolution {
    ContinueSession,
    RetryWorkspace,
    ReassignSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptIntentSolution {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub previous_command: CommandId,
}

pub const DRAFT_RESET_CAPABILITY: &str = "intent-draft-reset-v1";
pub const MERGER_DRAFT_RESET_CAPABILITY: &str = "intent-merger-draft-reset-v1";
pub const DISCARDED_SESSION_REASON: &str = "intent returned to draft";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnIntentToDraft {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub previous_command: CommandId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftReset {
    pub request_id: RequestId,
    pub ticket_id: TicketId,
    pub command_id: CommandId,
    pub root_id: CommandId,
    pub stream_id: StreamId,
    pub ownership_epoch: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merger_commands: Vec<CommandId>,
}

pub const INTENT_ATTENTION_EVENT: &str = "intent.attention_required";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub draft_resets: Vec<DraftReset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intent_problems: Vec<IntentProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directs: Vec<directs::Direct>,
    /// Optional feature metadata; an older server may omit this entirely.
    #[serde(default)]
    pub server_capabilities: Vec<String>,
    /// Observed input + output tokens; absent entries have no recorded usage.
    #[serde(default, skip_serializing_if = "ObservedTokenTotals::is_empty")]
    pub token_totals: ObservedTokenTotals,
    /// Last durable activity per OpenCode session, excluding presence heartbeats.
    #[serde(default)]
    pub session_activity_unix_ms: std::collections::BTreeMap<String, u64>,
    /// Durable failure time per command; absent from older servers.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub command_failed_unix_ms: std::collections::BTreeMap<CommandId, u64>,
    #[serde(default)]
    pub editable_tasks: std::collections::BTreeMap<TicketId, Revision>,
    #[serde(default)]
    pub session_titles: std::collections::BTreeMap<String, String>,
    /// Desktop metadata transport; binary prompt inputs are fetched on demand.
    #[serde(default)]
    pub payloads_deferred: bool,
    #[serde(default)]
    pub session_closures: Vec<SessionClosure>,
    #[serde(default)]
    pub compactions: Vec<Compaction>,
    #[serde(default)]
    pub runtime_terminations: Vec<RuntimeTermination>,
    #[serde(default)]
    pub scheduler_policies: Vec<SchedulerPolicy>,
    #[serde(default)]
    pub token_budgets: Vec<TokenBudget>,
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

/// Explicitly retry the current failed execution without creating another intent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryTask {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub previous_command: CommandId,
}

pub const DELETE_FAILED_TASK_CAPABILITY: &str = "delete-failed-intent-v1";
pub const CANCEL_INTENT_CAPABILITY: &str = "cancel-intent-v1";
pub const DELETE_INACTIVE_TASK_CAPABILITY: &str = "delete-inactive-intent-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteInactiveTask {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub execution_command: Option<CommandId>,
}

/// Operator attestation after reconciling an old result in Git.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcilePublication {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub command_id: CommandId,
    pub base_branch: String,
    pub integrated_snapshot: String,
    pub target_commit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteFailedTask {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub failed_command: CommandId,
}

impl Snapshot {
    /// Use only the intent's own execution, not the latest run in a shared session.
    pub fn task_result_summary(&self, ticket: &Ticket) -> Option<&str> {
        let id = ticket.execution_command.filter(|_| !ticket.draft)?;
        let command = self.commands.iter().find(|command| command.id == id)?;
        if command.kind != CommandKind::RunAgent
            || command.state != CommandState::Succeeded
            || command.agent_spec.as_ref().is_some_and(|spec| spec.read_only)
        {
            return None;
        }
        let Some(CommandOutput::Agent { agent }) = &command.result else {
            return None;
        };
        let summary = agent.summary.trim();
        (agent.phase == AgentPhase::Succeeded && !summary.is_empty()).then_some(summary)
    }
}

/// A user-directed follow-up. All execution settings come from the target session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSession {
    pub request_id: RequestId,
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<PromptImage>,
    #[serde(default)]
    pub files: Vec<PromptFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObservedTokenTotals {
    pub by_session: std::collections::BTreeMap<CommandId, u64>,
    pub by_execution: std::collections::BTreeMap<CommandId, u64>,
}

impl ObservedTokenTotals {
    fn is_empty(&self) -> bool {
        self.by_session.is_empty() && self.by_execution.is_empty()
    }
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
pub struct RetryControl {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub previous_request: RequestId,
    pub expected_proof: ContainmentProof,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTermination {
    pub request_id: RequestId,
    pub expected_stream_revision: Revision,
    pub runtime: RuntimeRef,
    pub previous_request: Option<RequestId>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffStream {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub checkpoint_command: CommandId,
    pub target_runner: RunnerId,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestReview {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub checkpoint_command: CommandId,
    pub expected_target_commit: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestIntegration {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub review_command: CommandId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetScheduler {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub enabled: bool,
    pub runner_ids: Vec<RunnerId>,
    pub max_parallel: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDependencies {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub depends_on: Vec<TicketId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportTermination {
    pub request_id: RequestId,
    pub ownership_epoch: u64,
    pub state: TerminationState,
    pub proof: Option<RemovalProof>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<ContainmentProof>,
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
    /// Kept only to replay requests persisted by older clients.
    #[serde(default, rename = "expected_revision", skip_serializing_if = "Option::is_none")]
    pub legacy_revision: Option<Revision>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<ProjectLogo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_mapping: Option<ProjectRunnerMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub request_id: RequestId,
    pub name: String,
    pub repository: String,
    pub base_branch: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupProject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub request_id: RequestId,
    pub project_id: Option<ProjectId>,
    pub expected_revision: Revision,
    pub name: String,
    pub repository: String,
    pub base_branch: String,
    pub profile: ExecutionProfile,
    pub setup: ProjectSetup,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectSetupResult {
    pub project: Project,
    pub stream: Workstream,
    pub command: RunnerCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProject {
    /// Omitted: keep existing logo. Present with image=null: remove it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<ProjectLogoUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_mapping: Option<ProjectRunnerMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub name: Option<String>,
    pub repository: Option<String>,
    pub base_branch: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProjectMirror {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub runner_id: RunnerId,
    /// Empty removes this runner's mirror.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectLogoUpdate {
    pub image: Option<ProjectLogo>,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveTask {
    #[serde(default)]
    pub requested_session: Option<CommandId>,
    pub request_id: RequestId,
    pub id: Option<TicketId>,
    pub expected_revision: Revision,
    pub project_id: ProjectId,
    pub prompt: String,
    pub images: Vec<PromptImage>,
}

/// New endpoint: old SaveTask continues to release immediately for compatibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitTask {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_token: Option<RequestId>,
    pub task: SaveTask,
    pub draft: bool,
    #[serde(default)]
    pub files: Vec<PromptFile>,
}
pub mod models;
pub mod mcp;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginTaskEdit {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_token: Option<RequestId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEdit {
    pub ticket: Ticket,
    pub edit_token: RequestId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_mcps: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseProject {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub paused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteProject {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub confirm_name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportHostedReview {
    pub request_id: RequestId,
    pub ownership_epoch: u64,
    pub result: HostedReview,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTokenBudget {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub limits: TokenLimits,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSessionLimit {
    pub request_id: RequestId,
    pub expected_revision: Revision,
    pub max_sessions: u8,
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
