use crate::*;
use anyhow::{ensure, Context, Result};
use reqwest::{header, Client, StatusCode, Url};
use std::{env, fmt, time::Duration};
mod live;
pub use live::EventFeed;

#[derive(Clone)]
pub struct FactoryClient {
    http: Client,
    base: String,
}

#[derive(Debug)]
pub enum ClientError {
    Transport,
    Http {
        status: StatusCode,
        code: Option<ErrorCode>,
        current_revision: Option<Revision>,
    },
    InvalidResponse,
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport => write!(f, "Factory nicht erreichbar"),
            Self::Http {
                status,
                code,
                current_revision,
            } => write!(
                f,
                "Factory HTTP {status} ({code:?}); aktuelle Revision: {current_revision:?}"
            ),
            Self::InvalidResponse => write!(f, "Ungueltige Factory-Antwort"),
        }
    }
}
impl std::error::Error for ClientError {}

impl FactoryClient {
    pub async fn pause_project(
        &self,
        id: ProjectId,
        b: &PauseProject,
    ) -> Result<Project, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/projects/{id}/pause", self.base))
                .json(b)
                .send()
                .await,
        )
        .await
    }
    pub async fn save_task(&self, b: &SaveTask) -> Result<Ticket, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/tasks", self.base))
                .json(b)
                .send()
                .await,
        )
        .await
    }
    pub async fn setup_project(&self, b: &SetupProject) -> Result<ProjectSetupResult, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/projects/setup", self.base))
                .json(b)
                .send()
                .await,
        )
        .await
    }
    pub async fn scheduler_policy(
        &self,
        id: ProjectId,
    ) -> Result<Option<SchedulerPolicy>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/projects/{id}/scheduler", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn set_scheduler(
        &self,
        id: ProjectId,
        body: &SetScheduler,
    ) -> Result<SchedulerPolicy, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/projects/{id}/scheduler", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn set_dependencies(
        &self,
        id: TicketId,
        body: &SetDependencies,
    ) -> Result<Ticket, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/tickets/{id}/dependencies", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn request_review(
        &self,
        id: StreamId,
        body: &RequestReview,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/review", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn request_integration(
        &self,
        id: StreamId,
        body: &RequestIntegration,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/integrate", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn review_job(&self, id: CommandId) -> Result<ReviewJob, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/commands/{id}/review", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn runner_review_job(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
    ) -> Result<ReviewJob, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/runners/{runner}/commands/{id}/review",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("epoch", epoch)])
                .send()
                .await,
        )
        .await
    }
    pub async fn terminal_cleanup_commands(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
    ) -> Result<Vec<RunnerCommand>, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/runners/{runner}/runtime-cleanups",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .send()
                .await,
        )
        .await
    }
    pub async fn handoff(
        &self,
        id: StreamId,
        body: &HandoffStream,
    ) -> Result<Workstream, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/handoff", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn upload_checkpoint_bundle(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
        bytes: Vec<u8>,
    ) -> Result<BundleReceipt, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/checkpoint-bundle",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("epoch", epoch)])
                .body(bytes)
                .send()
                .await,
        )
        .await
    }
    pub async fn download_checkpoint_bundle(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
    ) -> anyhow::Result<Vec<u8>> {
        let response = self
            .http
            .get(format!(
                "{}/v1/runners/{runner}/commands/{id}/checkpoint-bundle",
                self.base
            ))
            .header(RUNNER_HEADER, registration.to_string())
            .query(&[("epoch", epoch)])
            .send()
            .await?
            .error_for_status()?;
        anyhow::ensure!(
            response
                .content_length()
                .is_some_and(|n| n <= CHECKPOINT_BUNDLE_LIMIT as u64),
            "Invalid checkpoint bundle length"
        );
        let bytes = response.bytes().await?;
        anyhow::ensure!(
            bytes.len() <= CHECKPOINT_BUNDLE_LIMIT,
            "Checkpoint bundle too large"
        );
        Ok(bytes.to_vec())
    }
    pub async fn checkpoint(&self, id: CommandId) -> Result<Option<CheckpointSpec>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/commands/{id}/checkpoint", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn runner_checkpoint(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
    ) -> Result<CheckpointSpec, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/runners/{runner}/commands/{id}/checkpoint",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("epoch", epoch)])
                .send()
                .await,
        )
        .await
    }
    pub async fn report_runtime_binding(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &RuntimeBinding,
    ) -> Result<RuntimeBinding, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/runtime-binding",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn termination(
        &self,
        id: CommandId,
    ) -> Result<Option<RuntimeTermination>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/commands/{id}/termination", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn request_termination(
        &self,
        id: CommandId,
        body: &RequestTermination,
    ) -> Result<RuntimeTermination, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/commands/{id}/termination", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn runner_termination(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Option<RuntimeTermination>, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/runners/{runner}/commands/{id}/termination",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("epoch", epoch)])
                .send()
                .await,
        )
        .await
    }
    pub async fn report_termination(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &ReportTermination,
    ) -> Result<RuntimeTermination, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/termination",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn knowledge_note(&self, id: KnowledgeId) -> Result<KnowledgeNote, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/knowledge/{id}", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn supervisor_policy(
        &self,
        id: ProjectId,
    ) -> Result<Option<SupervisorPolicy>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/projects/{id}/supervisor", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn set_supervisor(
        &self,
        id: ProjectId,
        body: &SetSupervisor,
    ) -> Result<SupervisorPolicy, ClientError> {
        Self::decode(
            self.http
                .put(format!("{}/v1/projects/{id}/supervisor", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn supervisor_evaluations(
        &self,
        id: ProjectId,
    ) -> Result<Vec<SupervisorEvaluation>, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/projects/{id}/supervisor/evaluations",
                    self.base
                ))
                .send()
                .await,
        )
        .await
    }
    pub async fn close_session(
        &self,
        id: CommandId,
        body: &CloseSession,
    ) -> Result<SessionClosure, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/commands/{id}/closure", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn session_closure(
        &self,
        id: CommandId,
    ) -> Result<Option<SessionClosure>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/commands/{id}/closure", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn continue_agent(
        &self,
        id: StreamId,
        body: &ContinueAgent,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/continue", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn request_compaction(
        &self,
        id: CommandId,
        body: &RequestCompaction,
    ) -> Result<Compaction, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/commands/{id}/compaction", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn compaction(&self, id: CommandId) -> Result<Option<Compaction>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/commands/{id}/compaction", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn runner_compaction(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Option<Compaction>, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/runners/{runner}/commands/{id}/compaction",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("epoch", epoch)])
                .send()
                .await,
        )
        .await
    }
    pub async fn report_compaction(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &ReportCompaction,
    ) -> Result<Compaction, ClientError> {
        Self::decode(
            self.http
                .put(format!(
                    "{}/v1/runners/{runner}/commands/{id}/compaction",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn archive(
        &self,
        id: CommandId,
        after: u64,
        through: Option<u64>,
    ) -> Result<ArchivePage, ClientError> {
        let mut request = self
            .http
            .get(format!("{}/v1/commands/{id}/archive", self.base))
            .query(&[("after", after)]);
        if let Some(revision) = through {
            request = request.query(&[("through_revision", revision)]);
        }
        Self::decode(request.send().await).await
    }
    pub async fn report_archive(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &ReportArchive,
    ) -> Result<ArchiveStatus, ClientError> {
        Self::decode(
            self.http
                .put(format!(
                    "{}/v1/runners/{runner}/commands/{id}/archive",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn create_knowledge(
        &self,
        body: &CreateKnowledge,
    ) -> Result<KnowledgeNote, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/knowledge", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn retire_knowledge(
        &self,
        id: KnowledgeId,
        body: &RetireKnowledge,
    ) -> Result<KnowledgeNote, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/knowledge/{id}/retire", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn knowledge(&self, id: ProjectId) -> Result<Vec<KnowledgeNote>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/projects/{id}/knowledge", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn knowledge_context(
        &self,
        id: ProjectId,
        ticket: Option<TicketId>,
        budget: usize,
    ) -> Result<KnowledgeContext, ClientError> {
        self.knowledge_context_for_branch(id, ticket, None, budget)
            .await
    }
    pub async fn knowledge_context_for_branch(
        &self,
        id: ProjectId,
        ticket: Option<TicketId>,
        branch: Option<&str>,
        budget: usize,
    ) -> Result<KnowledgeContext, ClientError> {
        let mut request = self
            .http
            .get(format!("{}/v1/projects/{id}/context", self.base))
            .query(&[("budget_bytes", budget)]);
        if let Some(ticket) = ticket {
            request = request.query(&[("ticket", ticket)]);
        }
        if let Some(branch) = branch {
            request = request.query(&[("branch", branch)]);
        }
        Self::decode(request.send().await).await
    }
    pub async fn enrollment(&self, body: &CreateEnrollment) -> Result<Enrollment, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/devices/enrollment", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn redeem(&self, body: &RedeemEnrollment) -> Result<DeviceCredential, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/enroll", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn devices(&self) -> Result<Vec<Device>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/devices", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn revoke_device(&self, id: DeviceId) -> Result<Device, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/devices/{id}/revoke", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn excerpt(&self, id: CommandId) -> Result<Option<SessionExcerpt>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/commands/{id}/excerpt", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn report_excerpt(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &ReportExcerpt,
    ) -> Result<SessionExcerpt, ClientError> {
        Self::decode(
            self.http
                .put(format!(
                    "{}/v1/runners/{runner}/commands/{id}/excerpt",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn update_project(
        &self,
        id: ProjectId,
        body: &UpdateProject,
    ) -> Result<Project, ClientError> {
        Self::decode(
            self.http
                .patch(format!("{}/v1/projects/{id}", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn set_control(
        &self,
        id: StreamId,
        body: &SetControl,
    ) -> Result<StreamControl, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/control", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn retry_control(
        &self,
        id: StreamId,
        body: &RetryControl,
    ) -> Result<StreamControl, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/control/retry", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn runner_control(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Option<StreamControl>, ClientError> {
        Self::decode(
            self.http
                .get(format!(
                    "{}/v1/runners/{runner}/commands/{id}/control",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("epoch", epoch)])
                .send()
                .await,
        )
        .await
    }
    pub async fn report_control(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &ReportControl,
    ) -> Result<StreamControl, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/control",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn answer_question(
        &self,
        id: QuestionId,
        body: &AnswerQuestion,
    ) -> Result<Question, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/questions/{id}/answer", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn sync_questions(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &QuestionSnapshot,
    ) -> Result<Vec<Question>, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/questions",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn answer_delivery(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: QuestionId,
        body: &AnswerDelivery,
    ) -> Result<Question, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/questions/{id}/delivery",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn update_profile(
        &self,
        id: ProjectId,
        body: &UpdateProfile,
    ) -> Result<Project, ClientError> {
        Self::decode(
            self.http
                .patch(format!("{}/v1/projects/{id}/profile", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn observe_agent(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &ObserveAgent,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/observe",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn create_stream(&self, body: &CreateStream) -> Result<Workstream, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn enqueue(
        &self,
        id: StreamId,
        body: &EnqueueCommand,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/streams/{id}/commands", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn pending(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
    ) -> Result<Vec<RunnerCommand>, ClientError> {
        self.pending_wait(runner, registration, 0).await
    }
    pub async fn pending_wait(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        seconds: u8,
    ) -> Result<Vec<RunnerCommand>, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/runners/{runner}/commands", self.base))
                .header(RUNNER_HEADER, registration.to_string())
                .query(&[("wait_seconds", seconds)])
                .timeout(Duration::from_secs(u64::from(seconds) + 5))
                .send()
                .await,
        )
        .await
    }
    pub async fn acknowledge(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &AckCommand,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/ack",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn command_result(
        &self,
        runner: RunnerId,
        registration: RegistrationId,
        id: CommandId,
        body: &CommandResult,
    ) -> Result<RunnerCommand, ClientError> {
        Self::decode(
            self.http
                .post(format!(
                    "{}/v1/runners/{runner}/commands/{id}/result",
                    self.base
                ))
                .header(RUNNER_HEADER, registration.to_string())
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub fn from_env(url: &str) -> Result<Self> {
        let token = match (env::var("FACTORY_TOKEN"), env::var_os("FACTORY_TOKEN_FILE")) {
            (Ok(_), Some(_)) => anyhow::bail!("Set only FACTORY_TOKEN or FACTORY_TOKEN_FILE"),
            (Ok(token), None) => token,
            (Err(_), Some(path)) => std::fs::read_to_string(path)
                .context("Cannot read FACTORY_TOKEN_FILE")?
                .trim_end_matches(['\n', '\r'])
                .to_string(),
            _ => anyhow::bail!("FACTORY_TOKEN or FACTORY_TOKEN_FILE required"),
        };
        Self::new(url, &token)
    }

    pub fn new(url: &str, token: &str) -> Result<Self> {
        let parsed = Url::parse(url)?;
        ensure!(
            parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none()
                && parsed.path() == "/",
            "Factory URL must be an origin without credentials"
        );
        ensure!(
            parsed.scheme() == "https"
                || (parsed.scheme() == "http"
                    && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
            "HTTP requires loopback (use an SSH tunnel)"
        );
        ensure!(
            token.len() >= 32,
            "FACTORY_TOKEN must contain at least 32 bytes"
        );
        let mut headers = header::HeaderMap::new();
        let mut auth = header::HeaderValue::from_str(&format!("Bearer {token}"))?;
        auth.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth);
        headers.insert(VERSION_HEADER, header::HeaderValue::from_static("1"));
        Ok(Self {
            http: Client::builder()
                .default_headers(headers)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()?,
            base: url.trim_end_matches('/').to_owned(),
        })
    }

    async fn decode<T: serde::de::DeserializeOwned>(
        response: Result<reqwest::Response, reqwest::Error>,
    ) -> Result<T, ClientError> {
        let response = response.map_err(|_| ClientError::Transport)?;
        let status = response.status();
        if !status.is_success() {
            let error = response.json::<ApiError>().await.ok();
            let current_revision = error.as_ref().and_then(|e| e.current_revision);
            let code = error.map(|e| e.code);
            return Err(ClientError::Http {
                status,
                code,
                current_revision,
            });
        }
        response
            .json()
            .await
            .map_err(|_| ClientError::InvalidResponse)
    }

    pub async fn snapshot(&self) -> Result<Snapshot, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/snapshot", self.base))
                .send()
                .await,
        )
        .await
    }
    pub async fn create_project(&self, body: &CreateProject) -> Result<Project, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/projects", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn create_ticket(&self, body: &CreateTicket) -> Result<Ticket, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/tickets", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn update_ticket(
        &self,
        id: TicketId,
        body: &UpdateTicket,
    ) -> Result<Ticket, ClientError> {
        Self::decode(
            self.http
                .patch(format!("{}/v1/tickets/{id}", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn events(&self, after: u64) -> Result<EventPage, ClientError> {
        Self::decode(
            self.http
                .get(format!("{}/v1/events", self.base))
                .query(&[("after", after)])
                .send()
                .await,
        )
        .await
    }
    pub async fn register(&self, body: &RegisterRunner) -> Result<Registration, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/runners/register", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
    pub async fn heartbeat(
        &self,
        id: RunnerId,
        body: &Heartbeat,
    ) -> Result<Registration, ClientError> {
        Self::decode(
            self.http
                .post(format!("{}/v1/runners/{id}/heartbeat", self.base))
                .json(body)
                .send()
                .await,
        )
        .await
    }
}
