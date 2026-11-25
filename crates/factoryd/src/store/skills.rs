use super::*;

fn settings(db: &Connection, project: ProjectId) -> Result<Option<ProjectSkills>> {
    let row: Option<String> = db
        .query_row(
            "SELECT data FROM project_skills WHERE project_id=?1",
            [project.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    row.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
}

pub(super) fn check_project(db: &Connection, project: ProjectId) -> Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
        [project.to_string()],
        |r| r.get(0),
    )? {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

impl Store {
    pub(crate) fn require_factory_skill_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        if !supported {
            let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
            let tx = db.transaction()?;
            if super::work::command(&tx, id, runner, epoch)?
                .agent_spec
                .as_ref()
                .is_some_and(|a| !a.factory_skills.is_empty())
            {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }

    pub fn require_repo_skill_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        if !supported {
            let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
            let tx = db.transaction()?;
            let command = super::work::command(&tx, id, runner, epoch)?;
            if command
                .agent_spec
                .as_ref()
                .is_some_and(|s| s.repo_skills.is_some())
            {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }

    pub fn project_skills(&self, project: ProjectId) -> Result<ProjectSkills> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        check_project(&db, project)?;
        Ok(settings(&db, project)?.unwrap_or(ProjectSkills {
            revision: Revision(0),
            repo_skills: Vec::new(),
            factory_skills: Vec::new(),
        }))
    }

    pub fn set_project_skills(
        &self,
        project: ProjectId,
        body: SetProjectSkills,
    ) -> Result<ProjectSkills> {
        self.mutate(
            body.request_id,
            &format!("project.skills/{project}"),
            &body,
            |tx| {
                check_project(tx, project)?;
                if body.repo_skills.len() > 32
                    || !body
                        .repo_skills
                        .iter()
                        .all(|id| valid_repo_skill_id(id) && !id.starts_with(FACTORY_SKILL_PREFIX))
                {
                    return Err(StoreError::Invalid(
                        "Maximal 32 Skill-IDs in Kleinbuchstaben, mit Ziffern und Bindestrichen",
                    ));
                }
                let mut ids = body.repo_skills.clone();
                ids.sort_unstable();
                ids.dedup();
                if ids.len() != body.repo_skills.len() {
                    return Err(StoreError::Invalid("Doppelte Skill-ID"));
                }
                let mut imports = body.factory_skills.clone();
                imports.sort_unstable();
                if imports.len() > 8 || imports.windows(2).any(|pair| pair[0].id == pair[1].id) {
                    return Err(StoreError::Invalid(
                        "Maximal acht Factory-Skills, jeweils mit genau einer Version",
                    ));
                }
                for skill in &imports {
                    if !skill.is_valid() || !super::factory_skills::exists(tx, skill)? {
                        return Err(StoreError::Invalid(
                            "Factory-Skill oder Version nicht gefunden",
                        ));
                    }
                }
                let current = settings(tx, project)?.map_or(Revision(0), |s| s.revision);
                if current != body.expected_revision {
                    return Err(StoreError::RevisionConflict(current));
                }
                let saved = ProjectSkills {
                    revision: Revision(current.0 + 1),
                    repo_skills: ids,
                    factory_skills: imports,
                };
                tx.execute(
                    "INSERT INTO project_skills(project_id,data) VALUES(?1,?2)
                 ON CONFLICT(project_id) DO UPDATE SET data=excluded.data",
                    params![project.to_string(), serde_json::to_string(&saved)?],
                )?;
                event(tx, "project.skills", project.to_string(), &saved)?;
                Ok(saved)
            },
        )
    }
}

#[derive(Default, PartialEq, Eq, Debug, Serialize, serde::Deserialize)]
pub(super) struct SkillProfile {
    pub repo: Option<Vec<String>>,
    pub factory: Vec<FactorySkillRef>,
}

impl SkillProfile {
    pub fn matches(&self, spec: &AgentSpec) -> bool {
        spec.repo_skills == self.repo && spec.factory_skills == self.factory
    }

    /// Explicit prompts inherit the old grants, not newly approved skills.
    /// A legacy session without an allowlist can be narrowed to no repo skills.
    pub(super) fn for_session_prompt(&self, spec: &AgentSpec) -> Option<Self> {
        let repo = match (&spec.repo_skills, &self.repo) {
            (None, None) => None,
            (None, Some(_)) => Some(Vec::new()),
            (Some(pinned), grants) if pinned.iter().all(|id| grants.as_ref().is_some_and(|ids| ids.contains(id))) => {
                Some(pinned.clone())
            }
            _ => return None,
        };
        spec.factory_skills.iter().all(|skill| self.factory.contains(skill))
            .then(|| Self { repo, factory: spec.factory_skills.clone() })
    }

    pub(super) fn can_continue(&self, spec: &AgentSpec) -> bool {
        self.matches(spec)
            || (spec.repo_skills.is_none()
                && self.repo.as_ref().is_some_and(Vec::is_empty)
                && self.factory == spec.factory_skills)
    }
}

pub(super) fn selected(db: &Connection, project: ProjectId) -> Result<SkillProfile> {
    Ok(
        settings(db, project)?.map_or_else(SkillProfile::default, |s| SkillProfile {
            repo: Some(s.repo_skills),
            factory: s.factory_skills,
        }),
    )
}

pub(super) fn pin(tx: &Transaction<'_>, ticket: TicketId, skills: &SkillProfile) -> Result<()> {
    tx.execute(
        "INSERT INTO task_skills(ticket_id,data) VALUES(?1,?2)
         ON CONFLICT(ticket_id) DO UPDATE SET data=excluded.data",
        params![ticket.to_string(), serde_json::to_string(skills)?],
    )?;
    Ok(())
}

pub(super) fn for_ticket(
    db: &Connection,
    ticket: TicketId,
    project: ProjectId,
) -> Result<SkillProfile> {
    let row: Option<String> = db
        .query_row(
            "SELECT data FROM task_skills WHERE ticket_id=?1",
            [ticket.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    match row {
        Some(raw) => {
            if raw.starts_with('{') {
                Ok(serde_json::from_str(&raw)?)
            } else {
                Ok(SkillProfile {
                    repo: serde_json::from_str(&raw)?,
                    factory: Vec::new(),
                })
            }
        }
        None => selected(db, project),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_are_sorted_revisioned_and_pinned_at_submission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("factory.sqlite");
        let store = Store::open(&path).unwrap();
        let project = store
            .create_project(CreateProject {
                logo: None,
                key: None,
                request_id: RequestId::new(),
                name: "Example".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
                runner_mapping: None,
            })
            .unwrap();
        let set = |rev, skills: &[&str]| {
            store.set_project_skills(
                project.id,
                SetProjectSkills {
                    request_id: RequestId::new(),
                    expected_revision: Revision(rev),
                    repo_skills: skills.iter().map(|id| (*id).into()).collect(),
                    factory_skills: vec![],
                },
            )
        };
        assert_eq!(
            store.project_skills(project.id).unwrap().revision,
            Revision(0)
        );
        assert!(matches!(
            set(0, &["../unsafe"]),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            set(0, &["review", "review"]),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(
            set(0, &["review", "docs"]).unwrap().repo_skills,
            ["docs", "review"]
        );
        let ticket = store
            .submit_task(SubmitTask {
                task: SaveTask {
                    request_id: RequestId::new(),
                    id: None,
                    project_id: project.id,
                    expected_revision: Revision(0),
                    prompt: "Work".into(),
                    images: vec![],
                    requested_session: None,
                },
                edit_token: None,
                files: vec![],
                draft: false,
            })
            .unwrap();
        assert!(matches!(
            set(0, &[]),
            Err(StoreError::RevisionConflict(Revision(1)))
        ));
        set(1, &[]).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let db = store.0.lock().unwrap();
        assert_eq!(selected(&db, project.id).unwrap().repo, Some(vec![]));
        assert_eq!(
            for_ticket(&db, ticket.id, project.id).unwrap().repo,
            Some(vec!["docs".into(), "review".into()])
        );
    }
}
