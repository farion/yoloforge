use super::*;
impl Store {
    pub fn require_file_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = super::work::command(&tx, id, runner, epoch)?;
        if !supported && c.agent_spec.as_ref().is_some_and(|s| !s.files.is_empty()) {
            return Err(StoreError::WorkConflict);
        }
        Ok(())
    }
    pub fn require_image_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = super::work::command(&tx, id, runner, epoch)?;
        if !supported && c.agent_spec.as_ref().is_some_and(|s| !s.images.is_empty()) {
            return Err(StoreError::WorkConflict);
        }
        Ok(())
    }
    pub fn require_setup_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = super::work::command(&tx, id, runner, epoch)?;
        if !supported && c.workspace_spec.as_ref().is_some_and(|s| s.setup.is_some()) {
            return Err(StoreError::WorkConflict);
        }
        Ok(())
    }
    pub fn setup_project(&self, mut b: SetupProject) -> Result<ProjectSetupResult> {
        super::knowledge::validate_plan(&b.profile)?;
        if b.repository.trim().is_empty() {
            b.repository = "factory:managed".into();
        }
        b.setup.repository_source = b.repository.clone();
        self.mutate(b.request_id,"project.setup",&b,|tx| {
            project_fields(&b.name,&b.repository,&b.base_branch)?;
            if !b.profile.provider_id.is_empty() || !b.profile.model_id.is_empty(){line(&b.profile.provider_id,120)?;line(&b.profile.model_id,200)?;}
            for text in [&b.profile.toolchain,&b.profile.start_command,&b.profile.test_command] {if text.len()>2048 || text.chars().any(|c|c.is_control() && !matches!(c,'\n'|'\t')){return Err(StoreError::Invalid("Invalid execution profile text"));}}
            for path in [&b.setup.repository_path,&b.setup.service_registration] {
                if (!path.is_empty() && !path.starts_with('/')) || path.len()>4096 || path.chars().any(char::is_control){return Err(StoreError::Invalid("Setup paths must be absolute paths on the selected runner"));}
            }
            if let Some(config)=&b.setup.harness {if !config.is_object() || serde_json::to_vec(config)?.len()>32768 {return Err(StoreError::Invalid("Harness must be a JSON object up to 32 KiB"));}}
            let mut project=if let Some(id)=b.project_id {
                let data:Option<String>=tx.query_row("SELECT data FROM active_projects WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;
                let mut p:Project=serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if p.revision!=b.expected_revision{return Err(StoreError::RevisionConflict(p.revision));}
                // Existing stream plans stay pinned; project setup applies to new preparations.
                if p.repository!=b.repository || p.base_branch!=b.base_branch {
                    let used:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM streams WHERE project_id=?1)",[id.to_string()],|r|r.get(0))?;
                    if used {return Err(StoreError::WorkConflict);}p.repository=b.repository.clone();p.base_branch=b.base_branch.clone();
                }
                p.name=b.name.clone();p.profile=b.profile.clone();p.setup=Some(b.setup.clone());p.revision.0+=1;p
            }else {
                if b.expected_revision!=Revision(0){return Err(StoreError::RevisionConflict(Revision(0)));}
                Project {logo:None,runner_mapping:None,key:super::numbering::project_key(tx,&b.name,b.key.as_deref(),None)?,deleted:false,max_sessions:default_session_limit(),paused:false,runner_bindings:Default::default(),mirrors:Default::default(),id:ProjectId::new(),revision:Revision(1),name:b.name.clone(),repository:b.repository.clone(),base_branch:b.base_branch.clone(),profile:b.profile.clone(),setup:Some(b.setup.clone()),created_unix_ms:super::super::unix_ms()}
            };
            if b.project_id.is_some() {
                if let Some(key)=b.key.as_deref().filter(|key|!key.trim().is_empty()) {
                    let key=super::numbering::project_key(tx,&project.name,Some(key),Some(project.id))?;
                    if key!=project.key {
                        super::numbering::rename(tx,project.id,&key)?;
                        project.key=key;
                    }
                }
            }
            project.runner_bindings.insert(b.setup.runner_id,ProjectLocation {repository_path:b.setup.repository_path.clone(),service_registration:b.setup.service_registration.clone()});
            tx.execute("INSERT INTO projects(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![project.id.to_string(),serde_json::to_string(&project)?])?;
            super::numbering::reserve_key(tx,project.id,&project.key)?;
            event(tx,"project.setup",project.id.to_string(),&project)?;
            let stream=Self::create_stream_in(tx,&CreateStream {request_id:b.request_id,project_id:project.id,runner_id:b.setup.runner_id,goal:"Projekt einrichten und OpenCode pruefen".into(),ticket_ids:vec![]})?;
            let command=Self::enqueue_in(tx,stream.id,&EnqueueCommand {request_id:b.request_id,expected_revision:stream.revision,kind:CommandKind::PrepareWorkspace,prompt:None},None,false)?;
            Ok(ProjectSetupResult {project,stream:super::work::stream(tx,stream.id)?,command})
        })
    }
}
