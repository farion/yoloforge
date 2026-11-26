//! Domain vocabulary. No network, database or OpenCode dependencies.
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
use uuid::Uuid;
pub fn default_session_limit() -> u8 {
    6
}
mod images;
pub use images::*;
mod project_logo;
pub use project_logo::*;
mod archive_files;
pub use archive_files::*;
pub const TASK_IMAGES_CAPABILITY: &str = "task-images-v1";
pub const CONTEXT_PLANNING_CAPABILITY: &str = "context-planning-v1";
pub const LOCAL_PROJECT_CAPABILITY: &str = "local-project-create-v1";

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

/// Untruncated projected OpenCode messages, optionally augmented with bounded file snapshots.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_plan: Option<ContextTokenCount>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic_failover: bool,
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
    SessionInterrupted,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub orchestration_note: String,
    pub id: QuestionId,
    pub command_id: CommandId,
    pub session_id: String,
    /// Orders status updates and receipts. Never a precondition for answering.
    #[serde(rename = "revision", default)]
    pub update_seq: Revision,
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
    TargetGit,
}
pub const TARGET_GIT_CAPABILITY: &str = "target-git-v2";
pub const RUNNER_RESTART_CAPABILITY: &str = "desktop-runner-restart-v1";

/// A bounded plan, never a shell command or a model-selected checkout path.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TargetGitPlan {
    /// Set only by the Factory retry endpoint, never by model routing.
    #[serde(default)]
    pub resume: bool,
    pub commit: bool,
    pub commit_message: Option<String>,
    pub tag: Option<TargetTag>,
    pub push: bool,
    pub remote: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetTag {
    Named { name: String },
    Patch,
    Minor,
    Major,
}

impl TargetGitPlan {
    pub fn valid(&self) -> bool {
        let name = |s: &str| {
            !s.is_empty() && s.len() <= 128 && !s.starts_with('-') && !s.starts_with('.')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
        };
        (self.commit || self.tag.is_some() || self.push)
            && self.commit_message.as_ref().is_none_or(|s| {
                self.commit && !s.trim().is_empty() && s.len() <= 1024
                    && !s.chars().any(|c| c.is_control() && c != '\n')
            })
            && self.remote.as_ref().is_none_or(|s| self.push && name(s))
            && match &self.tag {
                Some(TargetTag::Named { name: tag }) => name(tag),
                _ => true,
            }
    }
}
pub const REVIEW_CAPABILITY: &str = "git-review-v1";
/// Runner-generated status prefix; preserves a blocked publication in the
/// supervisor's bounded evidence rather than burying it after the model summary.
pub const PUBLICATION_CONFLICT_PREFIX: &str = "[yoloforge:publication-conflict]";
pub const PUBLICATION_BLOCKED_PREFIX: &str = "[yoloforge:publication-blocked]";
pub const PUBLICATION_CONFIRMED_PREFIX: &str = "[yoloforge:publication-confirmed]";
pub const SESSION_STOPPED_PREFIX: &str = "[yoloforge:session-stopped]";

pub fn session_stopped_without_result(result: &Option<CommandOutput>) -> bool {
    matches!(result, Some(CommandOutput::Agent { agent })
        if agent.phase == AgentPhase::Failed && agent.summary.starts_with(SESSION_STOPPED_PREFIX))
}
pub const MERGER_ROLE: &str = "merger";
pub const MERGER_CAPABILITY: &str = "merger-v1";
pub const MERGE_PARENT_PREFIX: &str = "factory_merge_parent=";
pub const MERGER_RECOVERY_CAPABILITY: &str = "merger-recovery-v1";
pub const MERGER_RECOVERY_PROMPT_MARKER: &str = "factory_merge_recovery=workspace-preflight";

pub fn is_merger_recovery(command: &RunnerCommand) -> bool {
    command.agent_spec.as_ref().is_some_and(|spec| spec.role == MERGER_ROLE
        && spec.prompt.lines().nth(1) == Some(MERGER_RECOVERY_PROMPT_MARKER))
}

/// Legacy runners appended the publication error to an otherwise successful
/// agent result. It still means the Intent did not land on its target branch.
pub fn publication_was_blocked(summary: &str) -> bool {
    summary.starts_with(PUBLICATION_CONFLICT_PREFIX)
        || summary.starts_with(PUBLICATION_BLOCKED_PREFIX)
        || summary.lines().any(|line| line.starts_with(
            "Ergebnis im Arbeitsbereich erhalten; automatische Uebernahme in den Projektordner blockiert:",
        ))
}

pub fn publication_was_confirmed(summary: &str) -> bool {
    summary.starts_with(PUBLICATION_CONFIRMED_PREFIX)
        || summary.rsplit_once("\n\n").is_some_and(|(_, tail)|
            tail.starts_with("Ergebnis automatisch im Projektordner bereitgestellt: "))
}

pub fn command_publication_blocked(command: &RunnerCommand) -> bool {
    matches!(&command.result, Some(CommandOutput::Agent { agent }) if publication_was_blocked(&agent.summary))
}

pub fn command_publication_unconfirmed(command: &RunnerCommand) -> bool {
    command.state == CommandState::Succeeded
        && command.workspace_spec.as_ref().is_some_and(|spec| spec.setup.is_some())
        && !command.requires_target_git()
        && command.agent_spec.as_ref().is_some_and(|spec| !spec.read_only && spec.role != "direct")
        && matches!(&command.result, Some(CommandOutput::Agent { agent }) if !publication_was_confirmed(&agent.summary))
}
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
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedReview {
    pub review_command: CommandId,
    pub candidate_commit: String,
    pub target_branch: String,
    pub publication_branch: String,
    pub repository: String,
    pub url: String,
}
pub const CHECKPOINT_CAPABILITY: &str = "git-checkpoint-v1";
pub const AUTOMATIC_CHECKPOINT_CAPABILITY: &str = "automatic-checkpoint-v1";
pub const CHECKPOINT_REPOSITORY: &str = "factory:checkpoint";
pub const CHECKPOINT_RETURN: &str = "factory:checkpoint-return";
pub const AGENT_START_GRANT: &str = "Start authorized before writing to OpenCode";
pub const NATIVE_RUNNER_BOOT_HEADER: &str = "x-factory-native-boot";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeRunnerBoot {
    pub machine: String,
    pub boot: String,
}

impl NativeRunnerBoot {
    pub fn from_header(value: &str) -> Option<Self> {
        let (machine, boot) = value.split_once(':')?;
        let digest = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        (digest(machine) && digest(boot)).then(|| Self { machine: machine.into(), boot: boot.into() })
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckpointSpec {
    pub command_id: CommandId,
    pub source_commands: Vec<CommandId>,
    pub stopped_runtimes: Vec<RemovalProof>,
    pub context: String,
    #[serde(default)]
    pub committed_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_result: Option<ProjectCheckpointOrigin>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectCheckpointOrigin {
    pub runner_id: RunnerId,
    pub repository: String,
    pub repository_path: String,
    pub base_branch: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_planning: Option<ContextPlanning>,
    pub toolchain: String,
    pub start_command: String,
    pub test_command: String,
    pub provider_id: String,
    pub model_id: String,
}

/// Explicit tokenizer binding for this exact project model, not model selection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextPlanning {
    pub tokenizer: ContextTokenizer,
    pub knowledge_tokens: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextTokenizer {
    Cl100kBase,
    O200kBase,
    P50kBase,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextTokenCount {
    pub tokenizer: ContextTokenizer,
    pub tokens: usize,
    pub budget_tokens: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// An explicit allowlist for native repository skills. None is a legacy run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factory_skills: Vec<FactorySkillRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_mcps: Vec<LocalMcp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_mcps: Vec<RemoteMcpRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PromptFile>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// Intent evaluated by a standalone text Reviewer session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_of: Option<TicketId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PromptImage>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<SessionContinuation>,
    pub prompt: String,
    pub profile: ExecutionProfile,
}

pub const FACTORY_SKILL_PREFIX: &str = "yf-catalog-";

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactorySkillRef {
    pub id: String,
    pub version: u32,
}

impl FactorySkillRef {
    pub fn runtime_id(&self) -> String {
        format!("{FACTORY_SKILL_PREFIX}{}-v{}", self.id, self.version)
    }

    pub fn is_valid(&self) -> bool {
        self.id.len() <= 40 && valid_repo_skill_id(&self.id) && self.version > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalMcp {
    pub id: String,
    pub name: String,
    pub command: Vec<String>,
    pub runners: Vec<RunnerId>,
    /// Automatic membership also covers runners registered in the future.
    /// Defaults to false for immutable execution profiles created by older versions.
    #[serde(default)]
    pub all_runners: bool,
    #[serde(default)]
    pub excluded_runners: Vec<RunnerId>,
}

impl LocalMcp {
    pub fn enabled_on(&self, runner: RunnerId) -> bool {
        if self.all_runners { !self.excluded_runners.contains(&runner) }
        else { self.runners.contains(&runner) }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteMcpRef {
    pub id: String,
    pub name: String,
    pub revision: Revision,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct McpProfile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local: Vec<LocalMcp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote: Vec<RemoteMcpRef>,
}

impl McpProfile {
    pub fn is_empty(&self) -> bool {
        self.local.is_empty() && self.remote.is_empty()
    }

    pub fn matches(&self, agent: &AgentSpec) -> bool {
        self.local == agent.local_mcps && self.remote == agent.remote_mcps
    }

    pub fn from_agent(agent: &AgentSpec) -> Self {
        Self { local: agent.local_mcps.clone(), remote: agent.remote_mcps.clone() }
    }
}

pub fn valid_repo_skill_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
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
    Continue,
    Reallocate,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorDecision {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub followup: String,
    pub action: SupervisorAction,
    pub reason: String,
    pub memory: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SupervisorEvaluation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followup_command_id: Option<CommandId>,
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
    pub fn requires_target_git(&self) -> bool {
        self.kind == CommandKind::TargetGit
            || self.workspace_spec.as_ref().is_some_and(|s| s.target_git.is_some())
    }
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
    Finalizing,
    Unknown,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentProgress {
    pub session_id: String,
    pub message_id: String,
    pub phase: AgentPhase,
    pub summary: String,
}

pub const PUBLICATION_ARCHIVE_TYPE: &str = "factory_publication";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationOutcome {
    Local,
    Managed,
    Blocked,
    NotPublished,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicationNotice {
    pub unix_ms: u64,
    pub outcome: PublicationOutcome,
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
    pub target_git: Option<TargetGitPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<ProjectSetup>,
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
    TargetGit { git_summary: String, git_succeeded: bool },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<ProjectLogo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_mapping: Option<ProjectRunnerMapping>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// Removed from normal operation; history and repository files are retained.
    #[serde(default)]
    pub deleted: bool,
    #[serde(default = "default_session_limit")]
    pub max_sessions: u8,
    #[serde(default)]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub runner_bindings: std::collections::BTreeMap<RunnerId, ProjectLocation>,
    /// Optional device-bound checkouts of a Factory-managed project. Mirrors
    /// receive confirmed project checkpoints but are never execution sources.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub mirrors: std::collections::BTreeMap<RunnerId, ProjectMirror>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<ProjectSetup>,
    pub id: ProjectId,
    pub revision: Revision,
    pub name: String,
    pub repository: String,
    pub base_branch: String,
    pub created_unix_ms: u64,
    #[serde(default)]
    pub profile: ExecutionProfile,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRunnerMapping {
    Automatic,
    Runner(RunnerId),
}
/// Project harness settings and the selected execution-host binding. No credentials.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectSetup {
    #[serde(default)]
    pub repository_source: String,
    pub runner_id: RunnerId,
    pub repository_path: String,
    /// Empty means discover/start the runner user's native OpenCode service.
    pub service_registration: String,
    /// None preserves the repository's native OpenCode config; Some creates a new config.
    pub harness: Option<serde_json::Value>,
}
impl ProjectSetup {
    pub fn has_local_target(&self) -> bool {
        std::path::Path::new(&self.repository_source).is_absolute() && self.repository_path.is_empty()
    }
}
pub const PROJECT_SETUP_CAPABILITY: &str = "project-setup-v2";
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectLocation {
    pub repository_path: String,
    pub service_registration: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectMirror {
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    /// Stable project-scoped human reference, e.g. WOAI-1; never part of the prompt.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// Older clients created released tickets. Missing field preserves that behavior.
    #[serde(default)]
    pub draft: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PromptFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_command: Option<CommandId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_session: Option<CommandId>,
    /// Request that queued this prompt directly from the session composer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_prompt_request: Option<RequestId>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub routing_note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PromptImage>,
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
    Validating,
    ReworkQueued,
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
mod usage;
pub use usage::*;
