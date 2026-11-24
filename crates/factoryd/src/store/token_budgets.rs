use super::*;
pub(super) fn budget_reason(reason: &str) -> bool {
    reason.starts_with("Token budget") || reason.starts_with("Project token limit")
}

pub(super) fn load(tx: &Transaction<'_>, id: ProjectId) -> Result<TokenBudget> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
        [id.to_string()],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    let raw: Option<String> = tx
        .query_row(
            "SELECT data FROM token_budgets WHERE project_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match raw {
        Some(raw) => serde_json::from_str(&raw)?,
        None => TokenBudget {
            project_id: id,
            revision: Revision(0),
            limits: TokenLimits::default(),
        },
    })
}

fn bucket(role: &str) -> &'static str {
    match role {
        "" | "work" => "work",
        "memorizer" | "orchestrator-memory" => "memory",
        _ => "orchestration",
    }
}
pub(super) fn admit(tx: &Transaction<'_>, id: ProjectId, role: &str) -> Result<()> {
    let budget = load(tx, id)?;
    let category = bucket(role);
    let limit = match category {
        "work" => budget.limits.work,
        "memory" => budget.limits.memory,
        _ => budget.limits.orchestration,
    };
    let Some(limit) = limit else {
        return Ok(());
    };
    let usage = super::usage::usage_in(tx, id)?;
    let mut used = 0u64;
    for group in usage.groups.iter().filter(|g| bucket(&g.role) == category) {
        if group.messages_missing_tokens > 0 {
            return Err(StoreError::Invalid(
                "Token budget: archived usage is incomplete",
            ));
        }
        used = used
            .checked_add(group.tokens.input)
            .and_then(|v| v.checked_add(group.tokens.output))
            .ok_or(StoreError::Invalid("Token budget total overflow"))?;
    }
    if used >= limit {
        return Err(StoreError::Invalid(
            "Project token limit reached; increase or disable this category's limit",
        ));
    }
    // Finished commands without final archives are unknown, never zero consumption.
    let mut stmt = tx.prepare("SELECT COALESCE(json_extract(c.data,'$.agent_spec.role'),'') FROM commands c JOIN streams s ON s.id=c.stream_id LEFT JOIN archive_status a ON a.command_id=c.id WHERE s.project_id=?1 AND json_extract(c.data,'$.kind')='run_agent' AND c.state IN ('succeeded','failed') AND COALESCE(json_extract(a.data,'$.final_scan_complete'),0)=0")?;
    for role in stmt.query_map([id.to_string()], |r| r.get::<_, String>(0))? {
        if bucket(&role?) == category {
            return Err(StoreError::Invalid(
                "Token budget: final archive missing for previous execution",
            ));
        }
    }
    Ok(())
}
impl Store {
    pub fn token_budget(&self, id: ProjectId) -> Result<TokenBudget> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        load(&tx, id)
    }
    pub fn set_token_budget(&self, id: ProjectId, body: SetTokenBudget) -> Result<TokenBudget> {
        self.mutate(body.request_id,&format!("project.token_budget/{id}"),&body,|tx| {
            let mut budget=load(tx,id)?;
            if budget.revision!=body.expected_revision {return Err(StoreError::RevisionConflict(budget.revision));}
            budget.revision.0=budget.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
            budget.limits=body.limits.clone();
            tx.execute("INSERT INTO token_budgets(project_id,data) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET data=excluded.data",params![id.to_string(),serde_json::to_string(&budget)?])?;
            event(tx,"project.token_budget",id.to_string(),&budget)?;
            Ok(budget)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_default_unlimited_and_revision_fenced_limits_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budget.db");
        let store = Store::open(&path).unwrap();
        let project = store
            .create_project(CreateProject {
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                name: "budget".into(),
                repository: "/repo".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        assert_eq!(
            store.token_budget(project.id).unwrap().limits,
            TokenLimits::default()
        );
        let body = SetTokenBudget {
            request_id: RequestId::new(),
            expected_revision: Revision(0),
            limits: TokenLimits {
                work: Some(0),
                orchestration: None,
                memory: Some(0),
            },
        };
        let saved = store.set_token_budget(project.id, body.clone()).unwrap();
        assert_eq!(
            store.set_token_budget(project.id, body.clone()).unwrap(),
            saved
        );
        assert!(store
            .set_token_budget(
                project.id,
                SetTokenBudget {
                    request_id: RequestId::new(),
                    ..body
                }
            )
            .is_err());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.token_budget(project.id).unwrap(), saved);
        let mut db = store.0.lock().unwrap();
        let tx = db.transaction().unwrap();
        for role in ["", "work", "memorizer", "orchestrator-memory"] {
            assert!(admit(&tx, project.id, role).is_err());
        }
        for role in ["supervisor", "orchestrator", "orchestrator-question"] {
            assert!(admit(&tx, project.id, role).is_ok());
        }
    }
}
