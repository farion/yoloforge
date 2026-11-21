use super::*;
pub(super) fn load(tx: &Transaction<'_>, id: StreamId) -> Result<Option<StreamControl>> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM controls WHERE stream_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
fn save(tx: &Transaction<'_>, control: &StreamControl) -> Result<()> {
    tx.execute("INSERT INTO controls(stream_id,data) VALUES(?1,?2) ON CONFLICT(stream_id) DO UPDATE SET data=excluded.data",params![control.stream_id.to_string(),serde_json::to_string(control)?])?;
    event(tx, "stream.control", control.stream_id.to_string(), control)
}
pub(super) fn stream_status(
    tx: &Transaction<'_>,
    id: StreamId,
    fallback: StreamState,
) -> Result<StreamState> {
    Ok(match load(tx, id)?.map(|c| c.state) {
        Some(PauseState::Requested | PauseState::ResumeRequested) => StreamState::Pausing,
        Some(PauseState::HeldBeforeExecution | PauseState::Frozen) => StreamState::Paused,
        Some(PauseState::InterruptUnverified) => StreamState::Blocked,
        _ => fallback,
    })
}
pub(super) fn completed(tx: &Transaction<'_>, stream: StreamId, command: CommandId) -> Result<()> {
    if let Some(mut control) = load(tx, stream)? {
        if control.command_id == Some(command) && control.state != PauseState::Released {
            control.state = PauseState::InterruptUnverified;
            control.summary =
                "Command completed while stop was requested; process quiescence not verified"
                    .into();
            save(tx, &control)?;
        }
    }
    Ok(())
}
impl Store {
    pub fn set_control(&self, id: StreamId, body: SetControl) -> Result<StreamControl> {
        self.mutate(body.request_id,&format!("stream.control/{id}"),&body,|tx| {
            let mut owner=super::work::stream(tx,id)?;
            if owner.revision!=body.expected_revision {return Err(StoreError::RevisionConflict(owner.revision));}
            let previous=load(tx,id)?;
            let active:Option<String>=tx.query_row("SELECT data FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted')",[id.to_string()],|r|r.get(0)).optional()?;
            let active:Option<RunnerCommand>=active.map(|s|serde_json::from_str(&s)).transpose()?;
            let control=match body.action {
                ControlAction::Pause=>{
                    if let Some(previous)=previous.filter(|c|c.state!=PauseState::Released) {return Ok(previous);}
                    if active.as_ref().is_some_and(|c|c.kind!=CommandKind::RunAgent) {return Err(StoreError::WorkConflict);}
                    StreamControl {action:ControlAction::Pause,proof:None,stream_id:id,request_id:body.request_id,command_id:active.as_ref().map(|c|c.id),
                        state:if active.is_some() {PauseState::Requested} else {PauseState::HeldBeforeExecution},
                        summary:if active.is_some() {"Stop requested; awaiting runner"} else {"Scheduling held; no active Factory command"}.into(),previous_status:owner.status}
                },
                ControlAction::Resume=>{
                    let previous=previous.ok_or(StoreError::WorkConflict)?;
                    if !matches!(previous.state,PauseState::HeldBeforeExecution|PauseState::Frozen) {return Err(StoreError::WorkConflict);}
                    let frozen=previous.state==PauseState::Frozen;
                    if frozen && (previous.proof.as_ref().is_none_or(|p|!p.frozen) || active.as_ref().map(|c|c.id)!=previous.command_id) {return Err(StoreError::WorkConflict);}
                    StreamControl {action:ControlAction::Resume,request_id:body.request_id,state:if frozen {PauseState::ResumeRequested}else{PauseState::Released},summary:if frozen {"Thaw requested; awaiting owning runner"}else{"Scheduling released explicitly"}.into(),..previous}
                },
            };
            save(tx,&control)?;
            let status=active.as_ref().map_or(control.previous_status,|c|if c.state==CommandState::Pending {StreamState::Queued} else {StreamState::Running});
            super::work::save_stream(tx,&mut owner,status)?;
            Ok(control)
        })
    }
    pub fn runner_control(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Option<StreamControl>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let command = super::work::command(&tx, id, runner, epoch)?;
        load(&tx, command.stream_id)
    }
    pub fn report_control(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ReportControl,
    ) -> Result<StreamControl> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let command = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        let mut control = load(&tx, command.stream_id)?.ok_or(StoreError::WorkConflict)?;
        if control.request_id != body.control_request
            || control.command_id != Some(id)
            || command.state != CommandState::Accepted
        {
            return Err(StoreError::WorkConflict);
        }
        // A lost thaw response is replayable, but cannot reopen a released control.
        if control.state == PauseState::Released {
            if body.state == control.state
                && body.summary == control.summary
                && body.proof == control.proof
            {
                return Ok(control);
            }
            return Err(StoreError::WorkConflict);
        }
        if let Some(proof) = &body.proof {
            if proof.container_id.len() != 64
                || !proof.container_id.bytes().all(|b| b.is_ascii_hexdigit())
                || proof.observed_unix_ms == 0
            {
                return Err(StoreError::Invalid("Invalid containment proof"));
            }
            line(&proof.started_at, 100)?;
            if control.proof.as_ref().is_some_and(|old| {
                old.container_id != proof.container_id || old.started_at != proof.started_at
            }) {
                return Err(StoreError::WorkConflict);
            }
            if control
                .proof
                .as_ref()
                .is_some_and(|old| proof.observed_unix_ms < old.observed_unix_ms)
            {
                return Err(StoreError::WorkConflict);
            }
        }
        let valid = match body.state {
            PauseState::InterruptUnverified => body.proof.is_none(),
            PauseState::HeldBeforeExecution => {
                control.action == ControlAction::Pause
                    && body.proof.is_none()
                    && control.proof.is_none()
                    && matches!(
                        control.state,
                        PauseState::Requested | PauseState::HeldBeforeExecution
                    )
            }
            PauseState::Frozen => {
                control.action == ControlAction::Pause
                    && body.proof.as_ref().is_some_and(|p| p.frozen)
            }
            PauseState::Released => {
                control.action == ControlAction::Resume
                    && control.proof.is_some()
                    && body.proof.as_ref().is_some_and(|p| !p.frozen)
            }
            _ => false,
        };
        if !valid {
            return Err(StoreError::WorkConflict);
        }
        line(&body.summary, 240)?;
        if control.state != body.state
            || control.summary != body.summary
            || body
                .proof
                .as_ref()
                .is_some_and(|p| control.proof.as_ref() != Some(p))
        {
            let state_changed = control.state != body.state;
            control.state = body.state;
            control.summary = body.summary;
            if body.proof.is_some() {
                control.proof = body.proof;
            }
            save(&tx, &control)?;
            // A fresh observation of the same freezer state must not invalidate
            // an operator's pending resume solely because a heartbeat arrived.
            if state_changed {
                let mut owner = super::work::stream(&tx, command.stream_id)?;
                super::work::save_stream(&tx, &mut owner, StreamState::Running)?;
            }
        }
        tx.commit()?;
        Ok(control)
    }
}
