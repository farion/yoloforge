use super::*;
fn hash(s: &str) -> Result<()> {
    if !matches!(s.len(), 40 | 64) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StoreError::Invalid("Invalid Git object ID"));
    }
    Ok(())
}
pub(super) fn load(tx: &Transaction<'_>, id: CommandId) -> Result<ReviewJob> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM reviews WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?)
}
fn latest(tx: &Transaction<'_>, id: StreamId, expected: CommandId) -> Result<RunnerCommand> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM commands WHERE stream_id=?1 ORDER BY rowid DESC LIMIT 1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let c: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::WorkConflict)?)?;
    let owner = super::work::stream(tx, id)?;
    if c.id != expected
        || c.state != CommandState::Succeeded
        || c.ownership_epoch != owner.ownership_epoch
        || c.runner_id != owner.runner_id
    {
        return Err(StoreError::WorkConflict);
    }
    Ok(c)
}
fn save(tx: &Transaction<'_>, c: &RunnerCommand, job: &ReviewJob) -> Result<()> {
    tx.execute(
        "INSERT INTO reviews(command_id,data) VALUES(?1,?2)",
        params![c.id.to_string(), serde_json::to_string(job)?],
    )?;
    event(tx, "review.requested", c.id.to_string(), job)
}
impl Store {
    pub fn report_hosted_review(
        &self,
        id: CommandId,
        body: ReportHostedReview,
    ) -> Result<HostedReview> {
        self.mutate(
            body.request_id,
            &format!("review.hosting/{id}"),
            &body,
            |tx| {
                let raw: String = tx
                    .query_row(
                        "SELECT data FROM commands WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or(StoreError::NotFound)?;
                let command: RunnerCommand = serde_json::from_str(&raw)?;
                let owner = super::work::stream(tx, command.stream_id)?;
                let Some(CommandOutput::Review { review }) = &command.result else {
                    return Err(StoreError::WorkConflict);
                };
                let ReviewJob::Review { target_branch, .. } = load(tx, id)? else {
                    return Err(StoreError::WorkConflict);
                };
                let result = &body.result;
                if command.state != CommandState::Succeeded
                    || command.kind != CommandKind::ReviewCheckpoint
                    || owner.ownership_epoch != body.ownership_epoch
                    || command.ownership_epoch != body.ownership_epoch
                    || owner.runner_id != command.runner_id
                    || result.review_command != id
                    || result.candidate_commit != review.candidate_commit
                    || result.target_branch != target_branch
                    || result.publication_branch != format!("yoloforge/review-{id}")
                {
                    return Err(StoreError::WorkConflict);
                }
                line(&result.repository, 240)?;
                line(&result.url, 2048)?;
                let url = url::Url::parse(&result.url)
                    .map_err(|_| StoreError::Invalid("Invalid hosting URL"))?;
                if url.scheme() != "https"
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || !url
                        .path()
                        .starts_with(&format!("/{}/pull", result.repository))
                {
                    return Err(StoreError::Invalid("Invalid hosting URL"));
                }
                let previous: Option<String> = tx
                    .query_row(
                        "SELECT data FROM hosted_reviews WHERE command_id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(previous) = previous {
                    if serde_json::from_str::<HostedReview>(&previous)? != *result {
                        return Err(StoreError::WorkConflict);
                    }
                    return Ok(result.clone());
                }
                tx.execute(
                    "INSERT INTO hosted_reviews(command_id,data) VALUES(?1,?2)",
                    params![id.to_string(), serde_json::to_string(result)?],
                )?;
                event(tx, "review.published", id.to_string(), result)?;
                Ok(result.clone())
            },
        )
    }
    pub fn request_review(&self, id: StreamId, body: RequestReview) -> Result<RunnerCommand> {
        self.mutate(
            body.request_id,
            &format!("stream.review/{id}"),
            &body,
            |tx| {
                hash(&body.expected_target_commit)?;
                let source = latest(tx, id, body.checkpoint_command)?;
                let Some(CommandOutput::Checkpoint { checkpoint }) = source.result else {
                    return Err(StoreError::WorkConflict);
                };
                super::checkpoints::verify_bundle(tx, source.id, &checkpoint)?;
                let owner = super::work::stream(tx, id)?;
                let data: String = tx.query_row(
                    "SELECT data FROM projects WHERE id=?1",
                    [owner.project_id.to_string()],
                    |r| r.get(0),
                )?;
                let p: Project = serde_json::from_str(&data)?;
                if p.profile.test_command.trim().is_empty() {
                    return Err(StoreError::Invalid(
                        "Configure the project test command before review",
                    ));
                }
                let c = Self::enqueue_in(
                    tx,
                    id,
                    &EnqueueCommand {
                        request_id: body.request_id,
                        expected_revision: body.expected_revision,
                        kind: CommandKind::ReviewCheckpoint,
                        prompt: None,
                    },
                    None,
                    false,
                )?;
                save(
                    tx,
                    &c,
                    &ReviewJob::Review {
                        checkpoint: CheckpointImport {
                            command_id: source.id,
                            commit: checkpoint.commit,
                            bundle_sha256: checkpoint.bundle_sha256,
                            bundle_bytes: checkpoint.bundle_bytes,
                        },
                        target_branch: p.base_branch,
                        target_commit: body.expected_target_commit.clone(),
                        test_command: p.profile.test_command,
                    },
                )?;
                Ok(c)
            },
        )
    }
    pub fn request_integration(
        &self,
        id: StreamId,
        body: RequestIntegration,
    ) -> Result<RunnerCommand> {
        self.mutate(
            body.request_id,
            &format!("stream.integrate/{id}"),
            &body,
            |tx| {
                let source = latest(tx, id, body.review_command)?;
                let Some(CommandOutput::Review { review }) = source.result else {
                    return Err(StoreError::WorkConflict);
                };
                let ReviewJob::Review { target_branch, .. } = load(tx, source.id)? else {
                    return Err(StoreError::WorkConflict);
                };
                let c = Self::enqueue_in(
                    tx,
                    id,
                    &EnqueueCommand {
                        request_id: body.request_id,
                        expected_revision: body.expected_revision,
                        kind: CommandKind::IntegrateReview,
                        prompt: None,
                    },
                    None,
                    false,
                )?;
                save(
                    tx,
                    &c,
                    &ReviewJob::Integrate {
                        review_command: source.id,
                        target_branch,
                        target_commit: review.target_commit,
                        candidate_commit: review.candidate_commit,
                    },
                )?;
                Ok(c)
            },
        )
    }
    pub fn review_job(&self, id: CommandId) -> Result<ReviewJob> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        load(&tx, id)
    }
    pub fn runner_review_job(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
    ) -> Result<ReviewJob> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = super::work::command(&tx, id, runner, epoch)?;
        if c.state != CommandState::Accepted {
            return Err(StoreError::WorkConflict);
        }
        load(&tx, id)
    }
    pub fn require_review_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        if !supported
            && matches!(
                super::work::command(&tx, id, runner, epoch)?.kind,
                CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview
            )
        {
            return Err(StoreError::WorkConflict);
        }
        Ok(())
    }
}
pub(super) fn validate(
    tx: &Transaction<'_>,
    c: &RunnerCommand,
    result: &CommandOutput,
) -> Result<bool> {
    let job = load(tx, c.id)?;
    match (&job, result) {
        (_, CommandOutput::ReviewError { review_error }) => {
            line(review_error, 240)?;
            Ok(false)
        }
        (
            ReviewJob::Review {
                checkpoint,
                target_commit,
                ..
            },
            CommandOutput::Review { review },
        ) if c.kind == CommandKind::ReviewCheckpoint => {
            if review.checkpoint != checkpoint.command_id
                || review.target_commit != *target_commit
                || review.test_exit_code != 0
                || review.removal.observed_unix_ms == 0
            {
                return Err(StoreError::WorkConflict);
            }
            hash(&review.candidate_commit)?;
            hash(&review.tree)?;
            if review.removal.runtime.container_id.len() != 64
                || !review
                    .removal
                    .runtime
                    .container_id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                return Err(StoreError::WorkConflict);
            }
            line(&review.removal.daemon_id, 200)?;
            line(&review.image, 200)?;
            line(&review.log_path, 4096)?;
            Ok(true)
        }
        (
            ReviewJob::Integrate {
                review_command,
                target_branch,
                target_commit,
                candidate_commit,
            },
            CommandOutput::Integration { integration },
        ) if c.kind == CommandKind::IntegrateReview => {
            if integration.review_command != *review_command
                || integration.target_branch != *target_branch
                || integration.previous_commit != *target_commit
                || integration.commit != *candidate_commit
            {
                return Err(StoreError::WorkConflict);
            }
            Ok(true)
        }
        _ => Err(StoreError::WorkConflict),
    }
}
