use super::*;
use factory_protocol::skills::*;

pub(super) fn exists(db: &Connection, skill: &FactorySkillRef) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM factory_skill_versions WHERE id=?1 AND version=?2)",
        params![skill.id, skill.version],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_are_immutable_and_project_approval_pins_an_exact_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let p = store
            .create_project(CreateProject {
                logo: None,
                request_id: RequestId::new(),
                key: None,
                runner_mapping: None,
                name: "Example".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        let body = |version, text: &str| ImportFactorySkill {
            request_id: RequestId::new(),
            id: "review".into(),
            name: "Review".into(),
            description: "Review changes".into(),
            current_version: version,
            files: [
                ("SKILL.md".into(), text.to_owned()),
                ("references/review.md".into(), "Details".into()),
            ]
            .into(),
        };
        let v1 = store.import_factory_skill(body(0, "Version one")).unwrap();
        assert_eq!(v1.reference.version, 1);
        assert!(matches!(
            store.import_factory_skill(body(0, "Do not overwrite")),
            Err(StoreError::RevisionConflict(_))
        ));
        let mut traversal = body(1, "Invalid");
        traversal.files.insert("../escape".into(), "Invalid".into());
        assert!(matches!(
            store.import_factory_skill(traversal),
            Err(StoreError::Invalid(_))
        ));
        let grant = |revision, skill| {
            store
                .set_project_skills(
                    p.id,
                    SetProjectSkills {
                        request_id: RequestId::new(),
                        expected_revision: Revision(revision),
                        repo_skills: vec![],
                        factory_skills: vec![skill],
                    },
                )
                .unwrap()
        };
        grant(0, v1.reference.clone());
        let ticket = store
            .submit_task(SubmitTask {
                edit_token: None,
                draft: false,
                files: vec![],
                task: SaveTask {
                    request_id: RequestId::new(),
                    id: None,
                    expected_revision: Revision(0),
                    project_id: p.id,
                    prompt: "Review".into(),
                    images: vec![],
                    requested_session: None,
                },
            })
            .unwrap();
        let v2 = store.import_factory_skill(body(1, "Version two")).unwrap();
        assert_eq!(v2.reference.version, 2);
        assert_eq!(
            store.project_skills(p.id).unwrap().factory_skills,
            [v1.reference.clone()]
        );
        grant(1, v2.reference.clone());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.factory_skill_catalog().unwrap(), [v2, v1.clone()]);
        let db = store.0.lock().unwrap();
        assert_eq!(
            super::super::skills::for_ticket(&db, ticket.id, p.id)
                .unwrap()
                .factory,
            [v1.reference]
        );
    }
}

impl Store {
    pub fn factory_skill_catalog(&self) -> Result<Vec<FactorySkillSummary>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut statement =
            db.prepare("SELECT summary FROM factory_skill_versions ORDER BY id,version DESC")?;
        let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn import_factory_skill(&self, body: ImportFactorySkill) -> Result<FactorySkillSummary> {
        self.mutate(body.request_id, "skills.factory.import", &body, |tx| {
            if !valid_repo_skill_id(&body.id) || body.id.len() > 40 || body.name.trim().is_empty()
                || body.name.len() > 80 || body.name.chars().any(char::is_control)
                || body.description.len() > 360 || body.description.trim().is_empty()
                || body.description.chars().any(char::is_control)
                || !valid_skill_files(&body.files)
            {
                return Err(StoreError::Invalid("Skill braucht eine gültige ID, Name, Beschreibung und sichere UTF-8-Dateien einschließlich SKILL.md"));
            }
            let version: u32 = tx.query_row("SELECT COALESCE(MAX(version),0) FROM factory_skill_versions WHERE id=?1",
                [&body.id], |r| r.get(0))?;
            if body.current_version != version {
                return Err(StoreError::RevisionConflict(Revision(u64::from(version))));
            }
            let count: u32 = tx.query_row("SELECT COUNT(DISTINCT id) FROM factory_skill_versions", [], |r| r.get(0))?;
            if version >= 32 || (version == 0 && count >= 32) {
                return Err(StoreError::Invalid("Maximal 32 Factory-Skills mit je 32 Versionen"));
            }
            let files = serde_json::to_string(&body.files)?;
            let bytes: u64 = tx.query_row("SELECT COALESCE(SUM(length(CAST(files AS BLOB))),0) FROM factory_skill_versions",
                [], |row| row.get(0))?;
            if bytes.saturating_add(files.len() as u64) > 16 * 1024 * 1024 {
                return Err(StoreError::Invalid("Der Factory-Skill-Katalog hat das 16-MiB-Limit erreicht"));
            }
            let skill = FactorySkillSummary {
                reference:FactorySkillRef {id:body.id.clone(),version:version + 1},
                name:body.name.clone(), description:body.description.clone(), files:body.files.keys().cloned().collect(),
            };
            tx.execute("INSERT INTO factory_skill_versions(id,version,summary,files) VALUES(?1,?2,?3,?4)",
                params![skill.reference.id,skill.reference.version,serde_json::to_string(&skill)?,files])?;
            event(tx, "skills.factory.imported", skill.reference.id.clone(), &skill)?;
            Ok(skill)
        })
    }

    pub fn factory_skill_for_command(
        &self,
        runner: RunnerId,
        id: CommandId,
        skill: FactorySkillRef,
    ) -> Result<FactorySkillBundle> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let pair: Option<(String,String)> = db.query_row(
            "SELECT c.data, s.data FROM commands c JOIN streams s ON s.id=c.stream_id
             JOIN active_projects p ON p.id=s.project_id WHERE c.id=?1 AND c.runner_id=?2 AND c.state='accepted'",
            params![id.to_string(),runner.to_string()], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        let (command, stream) = pair.ok_or(StoreError::NotFound)?;
        let command: RunnerCommand = serde_json::from_str(&command)?;
        let stream: Workstream = serde_json::from_str(&stream)?;
        if command.ownership_epoch != stream.ownership_epoch
            || stream.runner_id != runner
            || command
                .agent_spec
                .as_ref()
                .is_none_or(|a| a.read_only || !a.factory_skills.contains(&skill))
        {
            return Err(StoreError::NotFound);
        }
        let (summary, files): (String, String) = db
            .query_row(
                "SELECT summary,files FROM factory_skill_versions WHERE id=?1 AND version=?2",
                params![skill.id, skill.version],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let summary: FactorySkillSummary = serde_json::from_str(&summary)?;
        Ok(FactorySkillBundle {
            reference: skill,
            name: summary.name,
            description: summary.description,
            files: serde_json::from_str(&files)?,
        })
    }
}
