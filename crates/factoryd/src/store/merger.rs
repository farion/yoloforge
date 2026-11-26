use super::*;

pub(super) fn owns_conflict(original: &RunnerCommand, owner: &Workstream) -> bool {
    original.runner_id == owner.runner_id
        && original.ownership_epoch == owner.ownership_epoch
        && !owner.ticket_ids.is_empty()
        && !original.requires_target_git()
        && original.agent_spec.as_ref().is_some_and(|spec| {
            !spec.read_only
                && spec.role != MERGER_ROLE
                && spec.role != factory_protocol::directs::ROLE
        })
        && matches!(&original.result, Some(CommandOutput::Agent { agent })
            if agent.summary.starts_with(PUBLICATION_CONFLICT_PREFIX))
        && original
            .workspace_spec
            .as_ref()
            .and_then(|s| s.setup.as_ref())
            .is_some_and(|setup| {
                setup.repository_path.is_empty()
                    && (setup.repository_source == "factory:managed"
                        || super::work::absolute_runner_path(&setup.repository_source))
            })
}

impl Store {
    /// A real merge conflict is attempted once, with one recovery if the old
    /// checkout preflight failed before the Merger started.
    pub fn merger_tick(&self) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = {
            let mut query = tx.prepare(
                "SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id
                 JOIN active_projects p ON p.id=s.project_id
                 WHERE COALESCE(json_extract(p.data,'$.paused'),0)=0
                    AND json_extract(c.data,'$.kind')='run_agent'
                    AND c.rowid=(SELECT MAX(n.rowid) FROM commands n WHERE n.stream_id=c.stream_id)
                    AND (
                        (c.state='succeeded'
                         AND COALESCE(json_extract(c.data,'$.agent_spec.role'),'') NOT IN (?1,'direct')
                         AND instr(json_extract(c.data,'$.result.agent.summary'),?2)=1
                         AND EXISTS(SELECT 1 FROM archive_status a WHERE a.command_id=c.id AND json_extract(a.data,'$.final_scan_complete')=1))
                        OR (c.state='failed' AND json_extract(c.data,'$.agent_spec.role')=?1
                            AND json_extract(c.data,'$.result.agent.phase')='failed'
                            AND json_extract(c.data,'$.result.agent.summary')=?3
                            AND NOT EXISTS(SELECT 1 FROM runtime_bindings r WHERE r.command_id=c.id))
                    )
                 ORDER BY c.rowid"
            )?;
            let rows = query
                .query_map(params![MERGER_ROLE, PUBLICATION_CONFLICT_PREFIX,
                    "Merger konnte nicht starten: Projektordner enthaelt eigene Aenderungen; nichts ueberschrieben"], |row| {
                    row.get::<_, String>(0)
                })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for row in rows {
            let latest: RunnerCommand = serde_json::from_str(&row)?;
            let recovery = latest.state == CommandState::Failed;
            let original = if recovery {
                if is_merger_recovery(&latest) {
                    continue;
                }
                let Some(parent) = latest.agent_spec.as_ref().and_then(|spec| spec.prompt.lines().next())
                    .and_then(|line| line.strip_prefix(MERGE_PARENT_PREFIX))
                    .and_then(|id| id.parse::<CommandId>().ok()) else { continue; };
                let raw: Option<String> = tx.query_row(
                    "SELECT c.data FROM commands c JOIN archive_status a ON a.command_id=c.id
                     WHERE c.id=?1 AND c.stream_id=?2 AND c.state='succeeded'
                         AND json_extract(a.data,'$.final_scan_complete')=1",
                    params![parent.to_string(), latest.stream_id.to_string()], |row| row.get(0),
                ).optional()?;
                let Some(raw) = raw else { continue; };
                serde_json::from_str(&raw)?
            } else {
                latest.clone()
            };
            let owner = super::work::stream(&tx, original.stream_id)?;
            if !owns_conflict(&original, &owner)
                || super::sessions::closed(&tx, original.session_root())?
                || (recovery && (latest.workspace_spec != original.workspace_spec
                    || latest.runner_id != owner.runner_id || latest.ownership_epoch != owner.ownership_epoch))
            {
                continue;
            }
            let mut tickets = Vec::new();
            for id in &owner.ticket_ids {
                let raw: String = tx.query_row(
                    "SELECT data FROM tickets WHERE id=?1",
                    [id.to_string()],
                    |row| row.get(0),
                )?;
                let ticket: Ticket = serde_json::from_str(&raw)?;
                if ticket.execution_command == Some(latest.id)
                    && ticket.status != TicketState::Cancelled
                    && (!recovery || ticket.status == TicketState::Blocked)
                {
                    tickets.push(ticket);
                }
            }
            if tickets.is_empty() {
                continue;
            }
            let model = match super::models::role_selection(&tx, owner.project_id, MERGER_ROLE) {
                Ok(Some(model)) => model,
                Ok(None) | Err(StoreError::Invalid(_)) => continue,
                Err(err) => return Err(err),
            };
             let marker = if recovery { format!("{MERGER_RECOVERY_PROMPT_MARKER}\n") } else { String::new() };
             let prompt = format!(
                "{MERGE_PARENT_PREFIX}{}\n{marker}Resolve the integration conflict only. The original result is \
                 at refs/yoloforge/publication-snapshots/{} and the pinned target is at \
                 refs/yoloforge/merger-targets/{}. Preserve both commits as ancestors of your resolution. \
                 Run the relevant tests and commit the \
                 resolution in the assigned worktree. Never edit the project checkout or push. \
                 If the intended resolution is ambiguous, ask the user first.", original.id, original.id, original.id);
            tx.execute_batch("SAVEPOINT merge_result")?;
            let result = Store::enqueue_with_pins(
                &tx,
                owner.id,
                &EnqueueCommand {
                    request_id: RequestId::new(),
                    expected_revision: owner.revision,
                    kind: CommandKind::RunAgent,
                    prompt: Some(prompt),
                },
                None,
                MERGER_ROLE,
                Some(&model),
                Some((&Default::default(), &McpProfile::default())),
                None,
                false,
            );
            match result {
                Ok(merge) => {
                    if recovery && merge.workspace_spec != original.workspace_spec {
                        tx.execute_batch("ROLLBACK TO merge_result; RELEASE merge_result")?;
                        continue;
                    }
                    for mut ticket in tickets {
                        ticket.execution_command = Some(merge.id);
                        ticket.status = TicketState::InProgress;
                        ticket.revision.0 += 1;
                        ticket.updated_unix_ms = super::super::unix_ms();
                        ticket.routing_note = if recovery {
                            "Merger zur Wiederaufnahme eingereiht; ursprünglicher Snapshot bleibt erhalten."
                        } else { "Merger bearbeitet einen Git-Konflikt." }.into();
                        tx.execute(
                            "UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",
                            params![
                                ticket.revision.0,
                                serde_json::to_string(&ticket)?,
                                ticket.id.to_string()
                            ],
                        )?;
                        tx.execute("UPDATE scheduler_runs SET agent_id=?1 WHERE ticket_id=?2 AND agent_id=?3",
                            params![merge.id.to_string(), ticket.id.to_string(), latest.id.to_string()])?;
                        if recovery {
                            event(&tx, "task.merger_recovered", ticket.id.to_string(), &serde_json::json!({
                                "previous_command": latest.id, "command": merge.id, "snapshot_command": original.id,
                            }))?;
                        }
                        event(&tx, "ticket.updated", ticket.id.to_string(), &ticket)?;
                    }
                }
                Err(
                    StoreError::Invalid(_)
                    | StoreError::WorkConflict
                    | StoreError::RevisionConflict(_),
                ) => tx.execute_batch("ROLLBACK TO merge_result")?,
                Err(error) => return Err(error),
            }
            tx.execute_batch("RELEASE merge_result")?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_protocol::models::{
        FactoryModelSettings, ModelAccess, ModelSelection, RoleModels, SetFactoryModels,
    };

    #[test]
    fn conflict_enqueues_once_with_merger_model_but_dirty_target_does_not() {
        check_merger_recovery(true);
    }

    #[test]
    fn unsuccessful_merger_recovery_does_not_queue_again() {
        check_merger_recovery(false);
    }

    fn check_merger_recovery(success: bool) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("factory.db")).unwrap();
        let runner = RunnerId::new();
        let project = store
            .create_project(CreateProject {
                request_id: RequestId::new(),
                key: None,
                logo: None,
                name: "Merger test".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
                runner_mapping: Some(ProjectRunnerMapping::Runner(runner)),
            })
            .unwrap();
        let selection = |name: &str| ModelSelection {
            access_id: format!("factory-{name}"),
            model_id: name.into(),
        };
        store
            .set_factory_models(SetFactoryModels {
                request_id: RequestId::new(),
                settings: FactoryModelSettings {
                    revision: Revision(0),
                    default_model: Some(selection("worker")),
                    roles: RoleModels {
                        merger: Some(selection("merge")),
                        ..Default::default()
                    },
                    accesses: ["worker", "merge"]
                        .into_iter()
                        .map(|name| ModelAccess {
                            id: format!("factory-{name}"),
                            name: name.into(),
                            models: vec![name.into()],
                            transports: Default::default(),
                        })
                        .collect(),
                },
            })
            .unwrap();
        let mut sources = Vec::new();
        for reason in [PUBLICATION_BLOCKED_PREFIX, PUBLICATION_CONFLICT_PREFIX] {
            let mut ticket = store
                .submit_task(SubmitTask {
                    task: SaveTask {
                        request_id: RequestId::new(),
                        id: None,
                        expected_revision: Revision(0),
                        project_id: project.id,
                        requested_session: None,
                        prompt: "Implement change".into(),
                        images: vec![],
                    },
                    draft: false,
                    edit_token: None,
                    files: vec![],
                })
                .unwrap();
            let owner = store
                .create_stream(CreateStream {
                    request_id: RequestId::new(),
                    project_id: project.id,
                    goal: "Work".into(),
                    runner_id: runner,
                    ticket_ids: vec![ticket.id],
                })
                .unwrap();
            let enqueue = |revision, kind, prompt| {
                store
                    .enqueue(
                        owner.id,
                        EnqueueCommand {
                            request_id: RequestId::new(),
                            expected_revision: revision,
                            kind,
                            prompt,
                        },
                    )
                    .unwrap()
            };
            let finish = |cmd: &RunnerCommand, result| {
                store
                    .acknowledge(
                        runner,
                        cmd.id,
                        AckCommand {
                            request_id: RequestId::new(),
                            ownership_epoch: 1,
                        },
                    )
                    .unwrap();
                if cmd.kind == CommandKind::RunAgent {
                    store
                        .report_archive(
                            runner,
                            cmd.id,
                            ReportArchive {
                                ownership_epoch: 1,
                                batch: ArchiveBatch {
                                    session_id: cmd.session_id(),
                                    revision: 1,
                                    captured_unix_ms: 1,
                                    final_scan_complete: true,
                                    messages: vec![],
                                },
                            },
                        )
                        .unwrap();
                }
                store
                    .complete(
                        runner,
                        cmd.id,
                        CommandResult {
                            request_id: RequestId::new(),
                            ownership_epoch: 1,
                            result,
                        },
                    )
                    .unwrap()
            };
            let prepare = enqueue(owner.revision, CommandKind::PrepareWorkspace, None);
            finish(
                &prepare,
                CommandOutput::Workspace {
                    workspace: Workspace {
                        branch: owner.branch.clone(),
                        path: format!("/work/{}", owner.id),
                        base_commit: "a".repeat(40),
                    },
                },
            );
            let revision = store
                .snapshot()
                .unwrap()
                .streams
                .iter()
                .find(|s| s.id == owner.id)
                .unwrap()
                .revision;
            let work = enqueue(revision, CommandKind::RunAgent, Some("Work".into()));
            ticket.execution_command = Some(work.id);
            store
                .0
                .lock()
                .unwrap()
                .execute(
                    "UPDATE tickets SET data=?1 WHERE id=?2",
                    params![
                        serde_json::to_string(&ticket).unwrap(),
                        ticket.id.to_string()
                    ],
                )
                .unwrap();
            finish(
                &work,
                CommandOutput::Agent {
                    agent: AgentProgress {
                        session_id: work.session_id(),
                        message_id: agent_message_id(work.id),
                        phase: AgentPhase::Succeeded,
                        summary: format!("{reason}\nOriginal result retained"),
                    },
                },
            );
            sources.push((ticket.id, work));
        }
        store.merger_tick().unwrap();
        store.merger_tick().unwrap();
        let pending = store.pending(runner).unwrap();
        assert_eq!(pending.len(), 1);
        let merge = &pending[0];
        let spec = merge.agent_spec.as_ref().unwrap();
        assert_eq!(spec.role, MERGER_ROLE);
        assert_eq!(spec.profile.provider_id, "factory-merge");
        assert!(!spec.read_only);
        assert!(spec
            .prompt
            .starts_with(&format!("{MERGE_PARENT_PREFIX}{}", sources[1].1.id)));
        assert_eq!(
            store.task_payload(sources[0].0).unwrap().status,
            TicketState::Blocked
        );
        let blocked = store.task_payload(sources[0].0).unwrap();
        let mut confirmation = ReconcilePublication {
            request_id: RequestId::new(),
            expected_revision: blocked.revision,
            command_id: sources[0].1.id,
            base_branch: "wrong-branch".into(),
            integrated_snapshot: "b".repeat(40),
            target_commit: "c".repeat(40),
        };
        assert!(matches!(store.reconcile_publication(blocked.id, confirmation.clone()), Err(StoreError::WorkConflict)));
        confirmation.request_id = RequestId::new();
        confirmation.base_branch = "main".into();
        assert_eq!(store.reconcile_publication(blocked.id, confirmation).unwrap().status, TicketState::Done);
        let snapshot = store.snapshot().unwrap();
        let reconciled = snapshot.commands.iter().find(|c| c.id == sources[0].1.id).unwrap();
        assert!(!command_publication_blocked(reconciled));
        assert!(!command_publication_unconfirmed(reconciled));
        assert_eq!(
            store.task_payload(sources[1].0).unwrap().execution_command,
            Some(merge.id)
        );
        store
            .acknowledge(
                runner,
                merge.id,
                AckCommand {
                    request_id: RequestId::new(),
                    ownership_epoch: merge.ownership_epoch,
                },
            )
            .unwrap();
        store
            .complete(
                runner,
                merge.id,
                CommandResult {
                    request_id: RequestId::new(),
                    ownership_epoch: merge.ownership_epoch,
                    result: CommandOutput::Agent {
                        agent: AgentProgress {
                            session_id: merge.session_id(),
                            message_id: agent_message_id(merge.id),
                            phase: AgentPhase::Failed,
                            summary: "Merger konnte nicht starten: Projektordner enthaelt eigene Aenderungen; nichts ueberschrieben".into(),
                        },
                    },
                },
            )
            .unwrap();
        assert_eq!(store.task_payload(sources[1].0).unwrap().status, TicketState::Blocked);
        store.merger_tick().unwrap();
        store.merger_tick().unwrap();
        let pending = store.pending(runner).unwrap();
        assert_eq!(pending.len(), 1);
        let retry = &pending[0];
        assert_ne!(retry.id, merge.id);
        assert_eq!(retry.workspace_spec, merge.workspace_spec);
        assert!(is_merger_recovery(retry));
        assert!(retry.agent_spec.as_ref().unwrap().prompt.starts_with(&format!("{MERGE_PARENT_PREFIX}{}", sources[1].1.id)));
        assert_eq!(store.task_payload(sources[1].0).unwrap().execution_command, Some(retry.id));
        assert!(store.require_merger_support(runner, retry.id, retry.ownership_epoch, true, false).is_err());
        store.acknowledge(runner, retry.id, AckCommand {
            request_id: RequestId::new(), ownership_epoch: retry.ownership_epoch,
        }).unwrap();
        store.complete(runner, retry.id, CommandResult {
            request_id: RequestId::new(), ownership_epoch: retry.ownership_epoch,
            result: CommandOutput::Agent { agent: AgentProgress {
                session_id: retry.session_id(), message_id: agent_message_id(retry.id),
                phase: if success { AgentPhase::Succeeded } else { AgentPhase::Failed },
                summary: if success { format!("{PUBLICATION_CONFIRMED_PREFIX}\nPublished") }
                    else { "Merger konnte nicht starten: Projektordner enthaelt eigene Aenderungen; nichts ueberschrieben".into() },
            }},
        }).unwrap();
        assert_eq!(
            store.task_payload(sources[1].0).unwrap().status,
            if success { TicketState::Done } else { TicketState::Blocked }
        );
        store.merger_tick().unwrap();
        assert!(store.pending(runner).unwrap().is_empty());
    }
}
