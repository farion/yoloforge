use super::*;
use factory_protocol::mcp::{ProjectLocalMcps, RemoteMcp, RemoteMcpAuth, RemoveRemoteMcp};

type Stored = (RemoteMcp, Option<Vec<u8>>);

pub(super) fn entries(db: &Connection) -> Result<Vec<RemoteMcp>> {
    let mut statement = db.prepare("SELECT data FROM remote_mcps WHERE COALESCE(json_extract(data,'$.deleted'),0)=0 ORDER BY id")?;
    let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}

fn entry(db: &Connection, id: &str) -> Result<Option<Stored>> {
    let row: Option<(String, Option<Vec<u8>>)> = db
        .query_row(
            "SELECT data,sealed FROM remote_mcps WHERE id=?1 AND COALESCE(json_extract(data,'$.deleted'),0)=0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(data, sealed)| Ok((serde_json::from_str(&data)?, sealed)))
        .transpose()
}

impl Store {
    pub(crate) fn refresh_remote_mcp_secret(
        &self,
        id: &str,
        revision: Revision,
        expected: &[u8],
        replacement: &[u8],
    ) -> Result<bool> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        Ok(db.execute(
            "UPDATE remote_mcps SET sealed=?1 WHERE id=?2 AND sealed=?3 AND json_extract(data,'$.revision')=?4",
            params![replacement, id, expected, revision.0],
        )? == 1)
    }

    pub fn has_remote_mcp_key(&self) -> Result<bool> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM remote_mcps WHERE sealed IS NOT NULL)",
            [],
            |r| r.get(0),
        )?)
    }

    pub(crate) fn remote_mcps(&self) -> Result<Vec<RemoteMcp>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        entries(&db)
    }

    pub(crate) fn remote_mcp_credentials(&self, id: &str) -> Result<Stored> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        entry(&db, id)?.ok_or(StoreError::NotFound)
    }

    pub(crate) fn save_remote_mcp(
        &self,
        mut meta: RemoteMcp,
        expected: Revision,
        replacement: Option<Vec<u8>>,
        remove: bool,
    ) -> Result<RemoteMcp> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = &meta.reference.id;
        if super::mcp::catalog(&tx)?
            .entries
            .iter()
            .any(|local| &local.id == id)
        {
            return Err(StoreError::Invalid(
                "Diese MCP-ID ist bereits lokal vergeben",
            ));
        }
        let current = entry(&tx, id)?;
        let revision = current
            .as_ref()
            .map_or(Revision(0), |(old, _)| old.reference.revision);
        if expected != revision {
            return Err(StoreError::RevisionConflict(revision));
        }
        if current.is_none() && entries(&tx)?.len() >= 32 {
            return Err(StoreError::Invalid("Maximal 32 Remote-MCPs"));
        }
        let sealed = match (
            current,
            replacement,
            remove || matches!(meta.auth, RemoteMcpAuth::None),
        ) {
            (_, _, true) => None,
            (_, Some(new), false) => Some(new),
            (Some((old, Some(sealed))), None, false) => {
                if old.url != meta.url || old.auth != meta.auth {
                    return Err(StoreError::Invalid(
                        "Bei Änderung des Endpunkts oder Headers den Schlüssel erneut eingeben",
                    ));
                }
                Some(sealed)
            }
            _ => None,
        };
        meta.has_key = sealed.is_some();
        let previous: Option<u64> = tx
            .query_row(
                "SELECT json_extract(data,'$.revision') FROM remote_mcps WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        meta.reference.revision = Revision(previous.unwrap_or(0) + 1);
        tx.execute(
            "INSERT INTO remote_mcps(id,data,sealed) VALUES(?1,?2,?3)
             ON CONFLICT(id) DO UPDATE SET data=excluded.data,sealed=excluded.sealed",
            params![id, serde_json::to_string(&meta)?, sealed],
        )?;
        event(&tx, "mcp.remote", id.clone(), &meta.reference)?;
        tx.commit()?;
        Ok(meta)
    }

    pub(crate) fn remove_remote_mcp(&self, id: &str, body: RemoveRemoteMcp) -> Result<bool> {
        self.mutate(body.request_id, &format!("mcp.remote.remove/{id}"), &body, |tx| {
            let (entry, _) = entry(tx, id)?.ok_or(StoreError::NotFound)?;
            if body.expected_revision != entry.reference.revision {
                return Err(StoreError::RevisionConflict(entry.reference.revision));
            }
            let mut statement = tx.prepare("SELECT m.data FROM project_local_mcps m JOIN active_projects p ON p.id=m.project_id")?;
            for row in statement.query_map([], |r| r.get::<_,String>(0))? {
                let settings: ProjectLocalMcps = serde_json::from_str(&row?)?;
                if settings.required.iter().chain(&settings.optional).any(|entry| entry == id) {
                    return Err(StoreError::Invalid("MCP zuerst in allen Projekten entfernen"));
                }
            }
            tx.execute("UPDATE remote_mcps SET data=?1,sealed=NULL WHERE id=?2",
                params![serde_json::json!({"revision":entry.reference.revision,"deleted":true}).to_string(),id])?;
            event(tx, "mcp.remote.removed", id.to_owned(), &entry.reference)?;
            Ok(true)
        })
    }

    pub(crate) fn remote_for_command(
        &self,
        runner: RunnerId,
        command: CommandId,
        id: &str,
    ) -> Result<Stored> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let raw: Option<(String, String)> = db
            .query_row(
                "SELECT c.data, s.data FROM commands c JOIN streams s ON s.id=c.stream_id
             JOIN active_projects p ON p.id=s.project_id
             WHERE c.id=?1 AND c.runner_id=?2 AND c.state='accepted'",
                params![command.to_string(), runner.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (raw, stream) = raw.ok_or(StoreError::NotFound)?;
        let command: RunnerCommand = serde_json::from_str(&raw)?;
        let stream: Workstream = serde_json::from_str(&stream)?;
        let selected = command
            .agent_spec
            .as_ref()
            .filter(|a| !a.read_only)
            .and_then(|a| a.remote_mcps.iter().find(|m| m.id == id))
            .ok_or(StoreError::NotFound)?;
        if command.ownership_epoch != stream.ownership_epoch
            || stream.runner_id != runner
            || !super::mcp::granted(&db, stream.project_id, id)?
        {
            return Err(StoreError::NotFound);
        }
        let (meta, sealed) = entry(&db, id)?.ok_or(StoreError::NotFound)?;
        if &meta.reference != selected {
            return Err(StoreError::NotFound);
        }
        Ok((meta, sealed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_protocol::mcp::{ProjectLocalMcps, SetProjectLocalMcps};

    #[test]
    fn proxy_requires_accepted_command_owner_current_revision_and_live_project_grant() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let p = store
            .create_project(CreateProject {
                logo: None,
                runner_mapping: None,
                request_id: RequestId::new(),
                key: None,
                name: "Docs".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        let runner = RunnerId::new();
        let command_id = CommandId::new();
        let stream = store
            .create_stream(CreateStream {
                request_id: RequestId::new(),
                project_id: p.id,
                runner_id: runner,
                goal: "Read docs".into(),
                ticket_ids: vec![],
            })
            .unwrap();
        let meta = store
            .save_remote_mcp(
                RemoteMcp {
                    reference: RemoteMcpRef {
                        id: "docs".into(),
                        name: "Docs".into(),
                        revision: Revision(0),
                    },
                    url: "https://docs.example.invalid/mcp".into(),
                    auth: RemoteMcpAuth::None,
                    has_key: false,
                },
                Revision(0),
                None,
                false,
            )
            .unwrap();
        let grant = |revision, required| {
            store
                .save_project_local_mcps(
                    p.id,
                    SetProjectLocalMcps {
                        request_id: RequestId::new(),
                        settings: ProjectLocalMcps {
                            revision: Revision(revision),
                            required,
                            optional: vec![],
                        },
                    },
                )
                .unwrap()
        };
        grant(0, vec!["docs".into()]);
        let mut command = RunnerCommand {
            id: command_id,
            stream_id: stream.id,
            runner_id: runner,
            ownership_epoch: stream.ownership_epoch,
            kind: CommandKind::RunAgent,
            state: CommandState::Pending,
            result: None,
            workspace_spec: None,
            agent_progress: None,
            created_unix_ms: 0,
            agent_spec: Some(AgentSpec {
                repo_skills: None,
                factory_skills: vec![],
                local_mcps: vec![],
                remote_mcps: vec![meta.reference.clone()],
                files: vec![],
                images: vec![],
                role: String::new(),
                review_of: None,
                read_only: false,
                continuation: None,
                prompt: "Read docs".into(),
                profile: ExecutionProfile::default(),
            }),
        };
        {
            let db = store.0.lock().unwrap();
            db.execute("INSERT INTO commands(id,stream_id,runner_id,state,data) VALUES(?1,?2,?3,'pending',?4)",
                params![command_id.to_string(), stream.id.to_string(), runner.to_string(), serde_json::to_string(&command).unwrap()]).unwrap();
        }
        assert!(store
            .remote_for_command(runner, command_id, "docs")
            .is_err());
        command.state = CommandState::Accepted;
        {
            let db = store.0.lock().unwrap();
            db.execute(
                "UPDATE commands SET state='accepted',data=?1 WHERE id=?2",
                params![
                    serde_json::to_string(&command).unwrap(),
                    command_id.to_string()
                ],
            )
            .unwrap();
        }
        assert_eq!(
            store
                .remote_for_command(runner, command_id, "docs")
                .unwrap()
                .0,
            meta
        );
        assert!(store
            .remote_for_command(RunnerId::new(), command_id, "docs")
            .is_err());
        assert!(store
            .remote_for_command(runner, command_id, "other")
            .is_err());
        grant(1, vec![]);
        assert!(store
            .remote_for_command(runner, command_id, "docs")
            .is_err());
        grant(2, vec!["docs".into()]);
        store
            .save_remote_mcp(meta, Revision(1), None, false)
            .unwrap();
        assert!(store
            .remote_for_command(runner, command_id, "docs")
            .is_err());
    }

    #[test]
    fn deletion_erases_secret_and_does_not_reuse_remote_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let config = RemoteMcp {
            reference: RemoteMcpRef {
                id: "docs".into(),
                name: "Docs".into(),
                revision: Revision(0),
            },
            url: "https://docs.example.invalid/mcp".into(),
            auth: RemoteMcpAuth::Header {
                name: "authorization".into(),
                bearer: true,
            },
            has_key: false,
        };
        let saved = store
            .save_remote_mcp(config.clone(), Revision(0), Some(vec![1, 2]), false)
            .unwrap();
        let remove = |revision| RemoveRemoteMcp {
            request_id: RequestId::new(),
            expected_revision: revision,
        };
        assert!(matches!(
            store.remove_remote_mcp("docs", remove(Revision(0))),
            Err(StoreError::RevisionConflict(_))
        ));
        assert!(store
            .remove_remote_mcp("docs", remove(saved.reference.revision))
            .unwrap());
        assert!(store.remote_mcps().unwrap().is_empty());
        assert!(!store.has_remote_mcp_key().unwrap());
        let new = store
            .save_remote_mcp(config, Revision(0), None, false)
            .unwrap();
        assert!(new.reference.revision.0 > saved.reference.revision.0);
        assert!(store.remote_mcp_credentials("docs").unwrap().1.is_none());
    }
}
