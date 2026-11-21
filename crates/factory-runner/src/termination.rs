use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    ErrorCode, ReportTermination,
};

fn finished(journal: &Journal, record: &mut Record, t: &RuntimeTermination) -> Result<()> {
    ensure!(
        t.command_id == record.command.id
            && t.stream_id == record.command.stream_id
            && t.runner_id == record.command.runner_id
            && t.ownership_epoch == record.command.ownership_epoch
            && t.state == TerminationState::Stopped
            && t.proof.as_ref().is_some_and(|p| p.runtime == t.runtime),
        "Invalid termination confirmation"
    );
    let result = t.result.clone().context("Termination result missing")?;
    ensure!(
        matches!(&result,CommandOutput::Agent {agent} if matches!(agent.phase,AgentPhase::Failed|AgentPhase::Succeeded) && agent.session_id==record.command.session_id()),
        "Invalid termination result"
    );
    record.result = Some(result);
    record.reported = true;
    tokio::task::block_in_place(|| journal.save(record))
}
pub async fn process(
    client: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    registration: RegistrationId,
) -> Result<bool> {
    if record.command.kind != CommandKind::RunAgent {
        return Ok(false);
    }
    let Some(t) = client
        .runner_termination(
            record.command.runner_id,
            registration,
            record.command.id,
            record.command.ownership_epoch,
        )
        .await?
    else {
        return Ok(false);
    };
    if t.state == TerminationState::Stopped {
        finished(journal, record, &t)?;
        return Ok(true);
    }
    let proof = crate::containment::remove(journal, record, args, &t)
        .await
        .ok();
    let result = client
        .report_termination(
            record.command.runner_id,
            registration,
            record.command.id,
            &ReportTermination {
                request_id: t.request_id,
                ownership_epoch: record.command.ownership_epoch,
                state: if proof.is_some() {
                    TerminationState::Stopped
                } else {
                    TerminationState::Unknown
                },
                proof,
            },
        )
        .await;
    if matches!(
        &result,
        Err(ClientError::Http {
            code: Some(ErrorCode::WorkConflict),
            ..
        })
    ) && client
        .runner_termination(
            record.command.runner_id,
            registration,
            record.command.id,
            record.command.ownership_epoch,
        )
        .await?
        .is_some_and(|new| new.request_id != t.request_id)
    {
        return Ok(true);
    }
    let result = result?;
    if result.state == TerminationState::Stopped {
        finished(journal, record, &result)?;
    }
    Ok(true)
}
