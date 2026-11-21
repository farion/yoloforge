//! Revision-fenced management forms. Uncertain writes retain their immutable request.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    *,
};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Target {
    SetupProject(Option<ProjectId>),
    Scheduler(ProjectId),
    Dependencies(TicketId),
    NewProject,
    Project(ProjectId),
    Profile(ProjectId),
    NewTicket(ProjectId),
    Ticket(TicketId),
    NewStream(ProjectId),
}

#[derive(Clone)]
pub enum Request {
    SetupProject(Box<SetupProject>),
    Scheduler(ProjectId, SetScheduler),
    Dependencies(TicketId, SetDependencies),
    Project(CreateProject),
    UpdateProject(ProjectId, UpdateProject),
    Profile(ProjectId, UpdateProfile),
    NewTicket(CreateTicket),
    Ticket(TicketId, UpdateTicket),
    Stream(CreateStream),
}
impl Request {
    pub fn id(&self) -> RequestId {
        match self {
            Self::SetupProject(b) => b.request_id,
            Self::Scheduler(_, b) => b.request_id,
            Self::Dependencies(_, b) => b.request_id,
            Self::Project(b) => b.request_id,
            Self::UpdateProject(_, b) => b.request_id,
            Self::Profile(_, b) => b.request_id,
            Self::NewTicket(b) => b.request_id,
            Self::Ticket(_, b) => b.request_id,
            Self::Stream(b) => b.request_id,
        }
    }
    pub async fn send(&self, client: &FactoryClient) -> Result<(), ClientError> {
        match self {
            Self::SetupProject(b) => {
                client.setup_project(b).await?;
            }
            Self::Scheduler(id, b) => {
                client.set_scheduler(*id, b).await?;
            }
            Self::Dependencies(id, b) => {
                client.set_dependencies(*id, b).await?;
            }
            Self::Project(b) => {
                client.create_project(b).await?;
            }
            Self::UpdateProject(id, b) => {
                client.update_project(*id, b).await?;
            }
            Self::Profile(id, b) => {
                client.update_profile(*id, b).await?;
            }
            Self::NewTicket(b) => {
                client.create_ticket(b).await?;
            }
            Self::Ticket(id, b) => {
                client.update_ticket(*id, b).await?;
            }
            Self::Stream(b) => {
                client.create_stream(b).await?;
            }
        }
        Ok(())
    }
}

struct Field {
    label: String,
    text: String,
    base: String,
    cursor: usize,
    required: bool,
    multiline: bool,
    choices: Vec<(String, String)>,
    choice: usize,
    multiple: bool,
    selectable: bool,
}
impl Field {
    fn text(label: &str, value: &str, required: bool, multiline: bool) -> Self {
        Self {
            label: label.into(),
            text: value.into(),
            base: value.into(),
            cursor: value.len(),
            required,
            multiline,
            choices: vec![],
            choice: 0,
            multiple: false,
            selectable: false,
        }
    }
    fn select(label: &str, value: &str, choices: Vec<(String, String)>, multiple: bool) -> Self {
        let mut field = Self::text(label, value, !multiple, false);
        field.choice = choices.iter().position(|(id, _)| id == value).unwrap_or(0);
        field.choices = choices;
        field.multiple = multiple;
        field.selectable = true;
        field
    }
    fn selected(&self, value: &str) -> bool {
        self.text.split(',').any(|s| s == value)
    }
    fn changed(&self) -> Option<String> {
        (self.text != self.base).then(|| self.text.clone())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Editing,
    Sending,
    Uncertain,
    Conflict,
    Submitted,
}
pub struct Dialog {
    setup_result: Option<ProjectSetupResult>,
    target: Target,
    title: String,
    revision: Revision,
    fields: Vec<Field>,
    focus: usize,
    state: State,
    request: Option<Request>,
    message: String,
}
pub enum Action {
    None,
    Close,
    Send(Request),
}

impl Dialog {
    pub fn new(target: Target, snapshot: &Snapshot) -> Option<Self> {
        let priority = || {
            Field::select(
                "Prioritaet",
                "1",
                (0..=3).map(|n| (n.to_string(), format!("P{n}"))).collect(),
                false,
            )
        };
        let ticket_fields = || {
            vec![
                Field::text("Titel", "", true, false),
                Field::text("Beschreibung", "", false, true),
                Field::text("Abnahmekriterien", "", false, true),
                priority(),
            ]
        };
        let (title, revision, fields) = match target {
            Target::SetupProject(id) => {
                let p = id.and_then(|id| snapshot.projects.iter().find(|p| p.id == id));
                if id.is_some() && p.is_none() {
                    return None;
                }
                let setup = p.and_then(|p| p.setup.as_ref());
                let mut runners = snapshot
                    .runners
                    .iter()
                    .map(|r| (r.id.to_string(), format!("{} / {:?}", r.name, r.location)))
                    .collect::<Vec<_>>();
                if let Some(s) = setup {
                    if !runners.iter().any(|(id, _)| id == &s.runner_id.to_string()) {
                        runners.push((
                            s.runner_id.to_string(),
                            "Bisheriger Runner (offline)".into(),
                        ));
                    }
                }
                ("Projekt-Assistent | Tab: naechster Schritt | Ctrl-S: einrichten".into(),p.map_or(Revision(0),|p|p.revision),vec![
                    Field::text("1. Projektname",p.map_or("",|p|p.name.as_str()),true,false),
                    Field::text("2. Repository (Metadaten: URL oder Pfad)",p.map_or("",|p|p.repository.as_str()),true,false),
                    Field::text("3. Basisbranch",p.map_or("main",|p|p.base_branch.as_str()),true,false),
                    Field::select("4. Ausfuehrungsort / Runner (Space)",&setup.map(|s|s.runner_id.to_string()).unwrap_or_default(),runners,false),
                    Field::text("5. Vorhandenes Git-Repository: absoluter Pfad AUF DEM RUNNER",setup.map_or("",|s|s.repository_path.as_str()),true,false),
                    Field::text("6. OpenCode-Verbindung: leer = automatisch starten/finden; sonst Registrierungsdatei auf Runner",setup.map_or("",|s|s.service_registration.as_str()),false,false),
                    Field::text("7. Provider",p.map_or("openai",|p|p.profile.provider_id.as_str()),true,false),
                    Field::text("8. Modell-ID",p.map_or("",|p|p.profile.model_id.as_str()),true,false),
                    Field::text("9. Projekt-Harness JSON: leer = Konfiguration aus Repository; sonst neue opencode.json (keine Secrets)",&setup.and_then(|s|s.harness.as_ref()).map(|h|serde_json::to_string_pretty(h).unwrap_or_default()).unwrap_or_default(),false,true),
                    Field::text("10. Toolchain",p.map_or("",|p|p.profile.toolchain.as_str()),false,true),
                    Field::text("11. Startbefehl",p.map_or("",|p|p.profile.start_command.as_str()),false,true),
                    Field::text("12. Testbefehl",p.map_or("",|p|p.profile.test_command.as_str()),false,true),
                ])
            }
            Target::Scheduler(id) => {
                let project = snapshot.projects.iter().find(|p| p.id == id)?;
                let policy = snapshot
                    .scheduler_policies
                    .iter()
                    .find(|p| p.project_id == id);
                let mut runners = snapshot
                    .runners
                    .iter()
                    .map(|r| (r.id.to_string(), format!("{} ({:?})", r.name, r.connection)))
                    .collect::<Vec<_>>();
                if let Some(p) = policy {
                    for id in &p.runner_ids {
                        if !runners.iter().any(|(value, _)| value == &id.to_string()) {
                            runners.push((id.to_string(), format!("{id} (nicht verbunden)")));
                        }
                    }
                }
                (
                    format!("Ticketplanung: {}", project.name),
                    policy.map_or(Revision(0), |p| p.revision),
                    vec![
                        Field::select(
                            "Automatische Planung",
                            if policy.is_some_and(|p| p.enabled) {
                                "true"
                            } else {
                                "false"
                            },
                            vec![
                                ("false".into(), "Deaktiviert".into()),
                                ("true".into(), "Aktiviert".into()),
                            ],
                            false,
                        ),
                        Field::select(
                            "Runner (Space: Auswahl)",
                            &policy
                                .map(|p| {
                                    p.runner_ids
                                        .iter()
                                        .map(ToString::to_string)
                                        .collect::<Vec<_>>()
                                        .join(",")
                                })
                                .unwrap_or_default(),
                            runners,
                            true,
                        ),
                        Field::text(
                            "Maximal parallele Streams (1..16)",
                            &policy.map_or(1, |p| p.max_parallel).to_string(),
                            true,
                            false,
                        ),
                        Field::text(
                            "Verbleibendes Startbudget ersetzen (0..1000)",
                            &policy.map_or(10, |p| p.remaining_starts).to_string(),
                            true,
                            false,
                        ),
                    ],
                )
            }
            Target::Dependencies(id) => {
                let t = snapshot.tickets.iter().find(|t| t.id == id)?;
                (
                    format!("Abhaengigkeiten: {} (nur Done gibt frei)", t.title),
                    t.revision,
                    vec![Field::select(
                        "Voraussetzungen (Space: Auswahl)",
                        &t.depends_on
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                        snapshot
                            .tickets
                            .iter()
                            .filter(|other| other.project_id == t.project_id && other.id != id)
                            .map(|t| (t.id.to_string(), format!("{} [{:?}]", t.title, t.status)))
                            .collect(),
                        true,
                    )],
                )
            }
            Target::NewProject => (
                "Projekt anlegen".into(),
                Revision(0),
                vec![
                    Field::text("Name", "", true, false),
                    Field::text("Repository (Pfad oder URL)", "", true, false),
                    Field::text("Basisbranch", "main", true, false),
                ],
            ),
            Target::Project(id) => {
                let p = snapshot.projects.iter().find(|p| p.id == id)?;
                (
                    "Projekt bearbeiten".into(),
                    p.revision,
                    vec![
                        Field::text("Name", &p.name, true, false),
                        Field::text(
                            "Repository (nach Streamanlage fest)",
                            &p.repository,
                            true,
                            false,
                        ),
                        Field::text(
                            "Basisbranch (nach Streamanlage fest)",
                            &p.base_branch,
                            true,
                            false,
                        ),
                    ],
                )
            }
            Target::Profile(id) => {
                let p = snapshot.projects.iter().find(|p| p.id == id)?;
                (
                    format!("Projektprofil: {}", p.name),
                    p.revision,
                    vec![
                        Field::text("Provider", &p.profile.provider_id, true, false),
                        Field::text("Modell", &p.profile.model_id, true, false),
                        Field::text("Toolchain", &p.profile.toolchain, false, true),
                        Field::text("Startbefehl", &p.profile.start_command, false, true),
                        Field::text("Testbefehl", &p.profile.test_command, false, true),
                    ],
                )
            }
            Target::NewTicket(id) => {
                let p = snapshot.projects.iter().find(|p| p.id == id)?;
                (
                    format!("Ticket anlegen: {}", p.name),
                    Revision(0),
                    ticket_fields(),
                )
            }
            Target::Ticket(id) => {
                let t = snapshot.tickets.iter().find(|t| t.id == id)?;
                let mut fields = ticket_fields();
                for (f, value) in fields.iter_mut().zip([
                    t.title.clone(),
                    t.description.clone(),
                    t.acceptance_criteria.clone(),
                    t.priority.to_string(),
                ]) {
                    f.choice = f
                        .choices
                        .iter()
                        .position(|(id, _)| id == &value)
                        .unwrap_or(0);
                    f.cursor = value.len();
                    f.base = value.clone();
                    f.text = value;
                }
                let status = serde_json::to_value(t.status).ok()?.as_str()?.to_string();
                fields.push(Field::select(
                    "Status",
                    &status,
                    [
                        "backlog",
                        "ready",
                        "in_progress",
                        "blocked",
                        "review",
                        "done",
                        "cancelled",
                    ]
                    .into_iter()
                    .map(|s| (s.into(), s.into()))
                    .collect(),
                    false,
                ));
                ("Ticket bearbeiten".into(), t.revision, fields)
            }
            Target::NewStream(id) => {
                let p = snapshot.projects.iter().find(|p| p.id == id)?;
                (
                    format!("Stream anlegen: {}", p.name),
                    Revision(0),
                    vec![
                        Field::text("Ziel", "", true, false),
                        Field::select(
                            "Runner",
                            "",
                            snapshot
                                .runners
                                .iter()
                                .map(|r| {
                                    (
                                        r.id.to_string(),
                                        format!(
                                            "{} | {:?} | {:?} | {}",
                                            r.name, r.location, r.connection, r.id
                                        ),
                                    )
                                })
                                .collect(),
                            false,
                        ),
                        Field::select(
                            "Tickets (optional)",
                            "",
                            snapshot
                                .tickets
                                .iter()
                                .filter(|t| t.project_id == id)
                                .map(|t| {
                                    (
                                        t.id.to_string(),
                                        format!("{} | {:?} | {}", t.title, t.status, t.id),
                                    )
                                })
                                .collect(),
                            true,
                        ),
                    ],
                )
            }
        };
        Some(Self {
            setup_result: None,
            target,
            title,
            revision,
            fields,
            focus: 0,
            state: State::Editing,
            request: None,
            message: String::new(),
        })
    }
    pub fn submitted(&self) -> bool {
        self.state == State::Submitted
    }
    pub fn insert(&mut self, text: &str) {
        if self.state != State::Editing {
            return;
        }
        let f = &mut self.fields[self.focus];
        if f.selectable {
            return;
        }
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || (f.multiline && matches!(c, '\n' | '\t')))
            .collect();
        if f.text.len() + text.len() <= 32768 {
            f.text.insert_str(f.cursor, &text);
            f.cursor += text.len();
        }
    }
    pub fn setup_result(&mut self, result: ProjectSetupResult) {
        self.setup_result = Some(result);
    }
    pub fn monitor(&mut self, snapshot: &Snapshot) {
        let Some(result) = &self.setup_result else {
            return;
        };
        let Some(c) = snapshot.commands.iter().find(|c| c.id == result.command.id) else {
            return;
        };
        self.message=match &c.result {
            Some(CommandOutput::Workspace {workspace})=>format!("Projekt eingerichtet. OpenCode-Service erreichbar (Provider-Login separat erforderlich). Worktree: {}\nStream: {}\nEsc, dann 4: diesen Stream waehlen und Agent starten.",workspace.path,c.stream_id),
            Some(CommandOutput::WorkspaceError {workspace_error})=>format!("Einrichtung braucht Hilfe: {workspace_error}\nStream: {}. Nach Korrektur: Esc, 4, Stream oeffnen, Worktree vorbereiten erneut senden.",c.stream_id),
            _=>format!("Einrichtung {:?} auf Runner {}. Projekt {}, Stream {}. Dieser Dialog zeigt das Ergebnis automatisch.",c.state,c.runner_id,result.project.id,c.stream_id),
        };
    }
    fn reload(&mut self, snapshot: &Snapshot) {
        if matches!(
            self.state,
            State::Sending | State::Uncertain | State::Submitted
        ) {
            return;
        }
        let Some(mut fresh) = Self::new(self.target, snapshot) else {
            self.message = "Eintrag nicht im aktuellen Snapshot vorhanden.".into();
            return;
        };
        for (new, old) in fresh.fields.iter_mut().zip(&self.fields) {
            if old.text != old.base {
                new.text = old.text.clone();
                new.cursor = new.text.len();
                new.choice = new
                    .choices
                    .iter()
                    .position(|(id, _)| id == &new.text)
                    .unwrap_or(0);
            }
        }
        fresh.focus = self.focus;
        fresh.message =
            "Aktuellen Stand geladen; eigene Aenderungen behalten. Felder pruefen, dann Ctrl-S."
                .into();
        *self = fresh;
    }
    fn build(&self) -> Result<Request, String> {
        for f in &self.fields {
            if f.required && f.text.trim().is_empty() {
                return Err(format!("{} fehlt", f.label));
            }
            if f.selectable
                && f.text
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .any(|s| !f.choices.iter().any(|(id, _)| id == s))
            {
                return Err(format!(
                    "{}: Auswahl nicht mehr verfuegbar; Ctrl-R zum Aktualisieren",
                    f.label
                ));
            }
        }
        let value = |i: usize| self.fields[i].text.clone();
        let request_id = RequestId::new();
        let expected_revision = self.revision;
        Ok(match self.target {
            Target::SetupProject(project_id) => {
                let repository_path = value(4);
                let service_registration = value(5);
                if !repository_path.starts_with('/')
                    || (!service_registration.is_empty() && !service_registration.starts_with('/'))
                {
                    return Err("Runner-Pfade muessen absolut sein".into());
                }
                let harness = if value(8).trim().is_empty() {
                    None
                } else {
                    let config: serde_json::Value = serde_json::from_str(&value(8))
                        .map_err(|_| "Harness: gueltiges JSON-Objekt erforderlich")?;
                    if !config.is_object() {
                        return Err("Harness muss ein JSON-Objekt sein".into());
                    }
                    Some(config)
                };
                Request::SetupProject(Box::new(SetupProject {
                    request_id,
                    project_id,
                    expected_revision,
                    name: value(0),
                    repository: value(1),
                    base_branch: value(2),
                    profile: ExecutionProfile {
                        provider_id: value(6),
                        model_id: value(7),
                        toolchain: value(9),
                        start_command: value(10),
                        test_command: value(11),
                    },
                    setup: ProjectSetup {
                        runner_id: value(3).parse().map_err(|_| "Runner auswaehlen")?,
                        repository_path,
                        service_registration,
                        harness,
                    },
                }))
            }
            Target::Scheduler(id) => {
                let runner_ids = value(1)
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::parse)
                    .collect::<Result<Vec<RunnerId>, _>>()
                    .map_err(|_| "Runnerauswahl ungueltig")?;
                let max_parallel = value(2).parse::<u8>().map_err(|_| "Parallelitaet: 1..16")?;
                let start_budget = value(3)
                    .parse::<u32>()
                    .map_err(|_| "Startbudget: 0..1000")?;
                if runner_ids.is_empty()
                    || runner_ids.len() > 16
                    || !(1..=16).contains(&max_parallel)
                    || start_budget > 1000
                {
                    return Err(
                        "1..16 Runner, Parallelitaet 1..16, Startbudget 0..1000 erforderlich"
                            .into(),
                    );
                }
                Request::Scheduler(
                    id,
                    SetScheduler {
                        request_id,
                        expected_revision,
                        enabled: value(0) == "true",
                        runner_ids,
                        max_parallel,
                        start_budget,
                    },
                )
            }
            Target::Dependencies(id) => Request::Dependencies(
                id,
                SetDependencies {
                    request_id,
                    expected_revision,
                    depends_on: value(0)
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::parse)
                        .collect::<Result<_, _>>()
                        .map_err(|_| "Ticketauswahl ungueltig")?,
                },
            ),
            Target::NewProject => Request::Project(CreateProject {
                request_id,
                name: value(0),
                repository: value(1),
                base_branch: value(2),
            }),
            Target::Project(id) => {
                if self.fields.iter().all(|f| f.text == f.base) {
                    return Err("Keine Aenderungen".into());
                }
                Request::UpdateProject(
                    id,
                    UpdateProject {
                        request_id,
                        expected_revision,
                        name: self.fields[0].changed(),
                        repository: self.fields[1].changed(),
                        base_branch: self.fields[2].changed(),
                    },
                )
            }
            Target::Profile(id) => Request::Profile(
                id,
                UpdateProfile {
                    request_id,
                    expected_revision,
                    profile: ExecutionProfile {
                        provider_id: value(0),
                        model_id: value(1),
                        toolchain: value(2),
                        start_command: value(3),
                        test_command: value(4),
                    },
                },
            ),
            Target::NewTicket(project_id) => Request::NewTicket(CreateTicket {
                request_id,
                project_id,
                title: value(0),
                description: value(1),
                acceptance_criteria: value(2),
                priority: value(3).parse().map_err(|_| "Prioritaet ungueltig")?,
            }),
            Target::Ticket(id) => {
                if self.fields.iter().all(|f| f.text == f.base) {
                    return Err("Keine Aenderungen".into());
                }
                Request::Ticket(
                    id,
                    UpdateTicket {
                        request_id,
                        expected_revision,
                        title: self.fields[0].changed(),
                        description: self.fields[1].changed(),
                        acceptance_criteria: self.fields[2].changed(),
                        priority: self.fields[3]
                            .changed()
                            .map(|v| v.parse())
                            .transpose()
                            .map_err(|_| "Prioritaet ungueltig")?,
                        status: self.fields[4]
                            .changed()
                            .map(|v| serde_json::from_value(serde_json::Value::String(v)))
                            .transpose()
                            .map_err(|_| "Status ungueltig")?,
                    },
                )
            }
            Target::NewStream(project_id) => Request::Stream(CreateStream {
                request_id,
                project_id,
                goal: value(0),
                runner_id: value(1).parse().map_err(|_| "Runner auswaehlen")?,
                ticket_ids: value(2)
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| "Ticketauswahl ungueltig")?,
            }),
        })
    }
    pub fn response(&mut self, id: RequestId, result: Result<(), ClientError>) {
        if self.request.as_ref().is_none_or(|r| r.id() != id) {
            return;
        }
        match result {
            Ok(()) => {
                self.state = State::Submitted;
                self.message = if matches!(self.target, Target::SetupProject(_)) {
                    "Einrichtung eingereiht. Esc, dann 5: Auftrag/Fehler pruefen. Nach Erfolg 4: vorbereiteten Stream oeffnen und Agent starten. Provider-Anmeldung muss im OpenCode-Service vorhanden sein.".into()
                } else {
                    "Gespeichert. Esc: Liste.".into()
                };
            }
            Err(error @ ClientError::Http { status, .. })
                if matches!(status.as_u16(), 400 | 401 | 403 | 404 | 409 | 422) =>
            {
                self.state = State::Conflict;
                self.message = format!("{error}. Entwurf erhalten; Ctrl-R: neu laden und pruefen.");
            }
            Err(_) => {
                self.state = State::Uncertain;
                self.message =
                    "Ergebnis unklar. Ctrl-S wiederholt exakt denselben Request; Entwurf gesperrt."
                        .into();
            }
        }
    }
    pub fn key(&mut self, key: KeyEvent, online: bool, snapshot: Option<&Snapshot>) -> Action {
        if key.code == KeyCode::Esc {
            return Action::Close;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('r') if online => {
                    if let Some(s) = snapshot {
                        self.reload(s);
                    }
                }
                KeyCode::Char('s') if !online => {
                    self.message = "Factory getrennt; Entwurf erhalten.".into()
                }
                KeyCode::Char('s') if matches!(self.state, State::Editing | State::Uncertain) => {
                    if self.state == State::Editing {
                        match self.build() {
                            Ok(request) => self.request = Some(request),
                            Err(e) => {
                                self.message = e;
                                return Action::None;
                            }
                        }
                    }
                    self.state = State::Sending;
                    self.message = "Sende ...".into();
                    return Action::Send(self.request.clone().unwrap());
                }
                _ => {}
            }
            return Action::None;
        }
        if key.code == KeyCode::Tab {
            self.focus = (self.focus + 1) % self.fields.len();
            return Action::None;
        }
        if key.code == KeyCode::BackTab {
            self.focus = (self.focus + self.fields.len() - 1) % self.fields.len();
            return Action::None;
        }
        if self.state != State::Editing {
            return Action::None;
        }
        let f = &mut self.fields[self.focus];
        if f.selectable {
            if f.choices.is_empty() {
                return Action::None;
            }
            let previous = f.text.clone();
            match key.code {
                KeyCode::Down | KeyCode::Up => {
                    f.choice = if f.text.is_empty() && !f.multiple {
                        0
                    } else if key.code == KeyCode::Down {
                        (f.choice + 1) % f.choices.len()
                    } else {
                        (f.choice + f.choices.len() - 1) % f.choices.len()
                    };
                    if !f.multiple {
                        f.text = f.choices[f.choice].0.clone();
                        f.cursor = f.text.len();
                    }
                }
                KeyCode::Char(' ') => {
                    let id = &f.choices[f.choice].0;
                    if f.multiple {
                        let mut ids: Vec<_> = f
                            .text
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .collect();
                        if f.selected(id) {
                            ids.retain(|v| v != id);
                        } else {
                            ids.push(id.clone());
                        }
                        f.text = ids.join(",");
                    } else {
                        f.text = id.clone();
                    }
                    f.cursor = f.text.len();
                }
                _ => {}
            }
            let selected = f.text.clone();
            if self.focus == 3 && previous != selected {
                if let Target::SetupProject(Some(project)) = self.target {
                    let binding = snapshot
                        .and_then(|s| s.projects.iter().find(|p| p.id == project))
                        .and_then(|p| {
                            selected
                                .parse::<RunnerId>()
                                .ok()
                                .and_then(|id| p.runner_bindings.get(&id))
                        });
                    for (i, value) in [
                        (4, binding.map_or("", |b| b.repository_path.as_str())),
                        (5, binding.map_or("", |b| b.service_registration.as_str())),
                    ] {
                        self.fields[i].text = value.into();
                        self.fields[i].base = value.into();
                        self.fields[i].cursor = value.len();
                    }
                    self.message="Ausfuehrungsort gewechselt; die Pfade dieses Runners pruefen. Projekt-Harness bleibt gemeinsam.".into();
                }
            }
            return Action::None;
        }
        match key.code {
            KeyCode::Left => {
                f.cursor = f.text[..f.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i)
            }
            KeyCode::Right => {
                f.cursor += f.text[f.cursor..].chars().next().map_or(0, char::len_utf8)
            }
            KeyCode::Home => f.cursor = 0,
            KeyCode::End => f.cursor = f.text.len(),
            KeyCode::Backspace => {
                let start = f.text[..f.cursor]
                    .char_indices()
                    .last()
                    .map_or(0, |(i, _)| i);
                f.text.replace_range(start..f.cursor, "");
                f.cursor = start;
            }
            KeyCode::Delete => {
                let end = f.cursor + f.text[f.cursor..].chars().next().map_or(0, char::len_utf8);
                f.text.replace_range(f.cursor..end, "");
            }
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Char(c) => self.insert(&c.to_string()),
            _ => {}
        }
        Action::None
    }
    pub fn draw(&self, frame: &mut Frame, online: bool) {
        let areas = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(4),
            Constraint::Length(6),
        ])
        .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "{} | Revision {} | {}\n{:?}",
                clean(&self.title),
                self.revision.0,
                if online { "verbunden" } else { "GETRENNT" },
                self.target
            ))
            .block(Block::bordered().title(" Verwaltung "))
            .wrap(Wrap { trim: false }),
            areas[0],
        );
        let f = &self.fields[self.focus];
        let mut lines = vec![];
        if f.text != f.base
            && matches!(
                self.target,
                Target::Ticket(_) | Target::Profile(_) | Target::Project(_)
            )
        {
            lines.push(Line::from(format!(
                "Geladener Serverwert: {}",
                clean(&f.base).chars().take(160).collect::<String>()
            )));
        }
        if !f.choices.is_empty() {
            lines.push(Line::from(
                "Hoch/runter: Auswahl | Leertaste: waehlen/umschalten",
            ));
            let available = areas[1].height.saturating_sub(4).max(1) as usize;
            let start = f.choice.saturating_sub(available / 2);
            for (index, (id, label)) in f.choices.iter().enumerate().skip(start).take(available) {
                lines.push(Line::styled(
                    format!(
                        "{} [{}] {}",
                        if index == f.choice { ">" } else { " " },
                        if f.selected(id) { "x" } else { " " },
                        clean(label)
                    ),
                    Style::default().fg(if index == f.choice {
                        Color::Cyan
                    } else {
                        Color::White
                    }),
                ));
            }
        } else {
            let limit = (areas[1].width.saturating_sub(6) as usize / 4).max(1);
            let before = f.text[..f.cursor]
                .chars()
                .rev()
                .take(limit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            let after = f.text[f.cursor..].chars().take(limit).collect::<String>();
            lines.push(Line::from(vec![
                Span::raw(format!(
                    "{}{}",
                    if before.len() < f.cursor { "…" } else { "" },
                    clean(&before)
                )),
                Span::styled("▏", Style::default().fg(Color::Cyan)),
                Span::raw(format!(
                    "{}{}",
                    clean(&after),
                    if after.len() < f.text.len() - f.cursor {
                        "…"
                    } else {
                        ""
                    }
                )),
            ]));
            lines.push(Line::from(if f.selectable {
                "Keine Eintraege vorhanden. Ctrl-R: Auswahl aktualisieren."
            } else if f.multiline {
                "Enter: Zeilenumbruch | Pfeile/Home/End: Cursor | Text/Paste"
            } else {
                "Text/Paste | Pfeile/Home/End: Cursor"
            }));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(format!(
                    " Feld {}/{}: {}{} ",
                    self.focus + 1,
                    self.fields.len(),
                    f.label,
                    if f.required { " *" } else { "" }
                ))),
            areas[1],
        );
        frame.render_widget(Paragraph::new(format!("Tab/Shift-Tab: Feld | Ctrl-S: speichern/wiederholen | Ctrl-R: neu laden | Esc: Liste\n{}\n{:?} | Request: {}",self.message,self.state,self.request.as_ref().map(|r|r.id().to_string()).unwrap_or_default())).wrap(Wrap {trim:false}).block(Block::bordered()),areas[2]);
    }
}
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' | '\r' => '↵',
            '\t' => '⇥',
            c if c.is_control() => '�',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn snapshot() -> Snapshot {
        let project = ProjectId::new();
        serde_json::from_value(json!({"protocol_version":1,"server_version":"test","boot_id":BootId::new(),"revision":1,"runners":[],
            "projects":[{"id":project,"revision":1,"name":"test","repository":"/repo","base_branch":"main","created_unix_ms":1}],
            "tickets":[{"id":TicketId::new(),"project_id":project,"revision":1,"title":"old","description":"original","acceptance_criteria":"","priority":1,"status":"backlog","created_unix_ms":1,"updated_unix_ms":1}]})).unwrap()
    }
    fn control(d: &mut Dialog, c: char, s: &Snapshot) -> Action {
        d.key(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            true,
            Some(s),
        )
    }
    fn send(d: &mut Dialog, s: &Snapshot) -> Request {
        match control(d, 's', s) {
            Action::Send(r) => r,
            _ => panic!("expected request"),
        }
    }
    #[test]
    fn conflict_rebase_retains_only_local_edits_and_uses_new_revision() {
        let mut snapshot = snapshot();
        let target = Target::Ticket(snapshot.tickets[0].id);
        let mut d = Dialog::new(target, &snapshot).unwrap();
        d.insert(" changed");
        let first = send(&mut d, &snapshot);
        d.response(
            first.id(),
            Err(ClientError::Http {
                status: "409".parse().unwrap(),
                code: Some(ErrorCode::RevisionConflict),
                current_revision: Some(Revision(2)),
            }),
        );
        assert!(matches!(control(&mut d, 's', &snapshot), Action::None));
        snapshot.tickets[0].description = "other client".into();
        snapshot.tickets[0].revision = Revision(2);
        control(&mut d, 'r', &snapshot);
        assert_eq!(d.fields[0].text, "old changed");
        assert_eq!(d.fields[1].text, "other client");
        let second = send(&mut d, &snapshot);
        assert_ne!(first.id(), second.id());
        let Request::Ticket(_, body) = second else {
            panic!()
        };
        assert_eq!(body.expected_revision, Revision(2));
        assert_eq!(body.title, Some("old changed".into()));
        assert_eq!(body.description, None);
    }
    #[test]
    fn setup_wizard_requires_host_and_freezes_project_config_after_uncertain_delivery() {
        let mut s = snapshot();
        let runner = RunnerId::new();
        s.runners.push(RunnerView {
            id: runner,
            name: "Laptop".into(),
            location: RunnerLocation::Local,
            version: "test".into(),
            connection: ConnectionState::Online,
            last_seen_unix_ms: 1,
            opencode: OpenCodeStatus::NotConfigured,
        });
        let mut d = Dialog::new(Target::SetupProject(None), &s).unwrap();
        for (i, text) in [
            (0, "Project"),
            (1, "/repo"),
            (4, "/repo"),
            (7, "test"),
            (8, "{\"mcp\":{\"servers\":{}}}"),
        ] {
            d.focus = i;
            d.insert(text);
        }
        assert!(d.build().is_err());
        d.fields[3].text = runner.to_string();
        let first = send(&mut d, &s);
        d.response(
            first.id(),
            Err(ClientError::Http {
                status: "408".parse().unwrap(),
                code: None,
                current_revision: None,
            }),
        );
        d.focus = 8;
        d.insert("changed");
        control(&mut d, 'r', &s);
        let retry = send(&mut d, &s);
        let (Request::SetupProject(a), Request::SetupProject(b)) = (first, retry) else {
            panic!()
        };
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        assert_eq!(a.setup.runner_id, runner);
        assert!(a.setup.service_registration.is_empty());
        assert!(a.setup.harness.unwrap().is_object());
    }
    #[test]
    fn scheduler_rebase_keeps_user_parallelism_but_refreshes_consumed_budget() {
        let mut s = snapshot();
        let id = s.projects[0].id;
        let runner = RunnerId::new();
        s.scheduler_policies.push(SchedulerPolicy {
            project_id: id,
            revision: Revision(1),
            enabled: true,
            runner_ids: vec![runner],
            max_parallel: 1,
            remaining_starts: 10,
        });
        let mut d = Dialog::new(Target::Scheduler(id), &s).unwrap();
        d.fields[2].text = "3".into();
        let first = send(&mut d, &s);
        d.response(
            first.id(),
            Err(ClientError::Http {
                status: "409".parse().unwrap(),
                code: Some(ErrorCode::RevisionConflict),
                current_revision: Some(Revision(2)),
            }),
        );
        s.scheduler_policies[0].revision = Revision(2);
        s.scheduler_policies[0].remaining_starts = 9;
        control(&mut d, 'r', &s);
        let Request::Scheduler(_, b) = send(&mut d, &s) else {
            panic!()
        };
        assert_eq!(b.max_parallel, 3);
        assert_eq!(b.start_budget, 9);
        assert_eq!(b.expected_revision, Revision(2));
    }
    #[test]
    fn changing_setup_host_loads_its_paths_without_changing_project_harness() {
        let mut s = snapshot();
        let first = RunnerId::new();
        let second = RunnerId::new();
        for id in [first, second] {
            s.runners.push(RunnerView {
                id,
                name: "host".into(),
                location: RunnerLocation::Local,
                version: "test".into(),
                connection: ConnectionState::Online,
                last_seen_unix_ms: 1,
                opencode: OpenCodeStatus::NotConfigured,
            });
        }
        let project = &mut s.projects[0];
        let id = project.id;
        project.setup = Some(ProjectSetup {
            runner_id: first,
            repository_path: "/first".into(),
            service_registration: "/first/service.json".into(),
            harness: Some(serde_json::json!({"instructions":["RULES.md"]})),
        });
        project.runner_bindings.insert(
            second,
            ProjectLocation {
                repository_path: "/second".into(),
                service_registration: "/second/service.json".into(),
            },
        );
        let mut d = Dialog::new(Target::SetupProject(Some(id)), &s).unwrap();
        let original = d.fields[8].text.clone();
        d.focus = 3;
        d.key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            true,
            Some(&s),
        );
        assert_eq!(d.fields[4].text, "/second");
        assert_eq!(d.fields[5].text, "/second/service.json");
        assert_eq!(d.fields[8].text, original);
    }
    #[test]
    fn unknown_creation_freezes_request_across_close_reload_and_paste() {
        let snapshot = snapshot();
        let mut d = Dialog::new(Target::NewProject, &snapshot).unwrap();
        d.insert("Grüße");
        d.focus = 1;
        d.insert("/repo");
        let first = send(&mut d, &snapshot);
        d.response(first.id(), Err(ClientError::Transport));
        d.insert("changed");
        control(&mut d, 'r', &snapshot);
        assert_eq!(d.state, State::Uncertain);
        assert!(matches!(
            d.key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                true,
                Some(&snapshot)
            ),
            Action::Close
        ));
        let retry = send(&mut d, &snapshot);
        assert_eq!(retry.id(), first.id());
        let (Request::Project(a), Request::Project(b)) = (first, retry) else {
            panic!()
        };
        assert_eq!(
            serde_json::to_value(a).unwrap(),
            serde_json::to_value(b).unwrap()
        );
    }
    #[test]
    fn unicode_cursor_and_offline_submission_keep_draft() {
        let s = snapshot();
        let mut d = Dialog::new(Target::NewProject, &s).unwrap();
        d.insert("aü界");
        for code in [KeyCode::Left, KeyCode::Backspace] {
            d.key(KeyEvent::new(code, KeyModifiers::NONE), true, Some(&s));
        }
        d.insert("ö");
        assert_eq!(d.fields[0].text, "aö界");
        assert!(matches!(
            d.key(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                false,
                Some(&s)
            ),
            Action::None
        ));
        assert!(d.request.is_none());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 20)).unwrap();
        terminal.draw(|f| d.draw(f, false)).unwrap();
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.symbol() == "▏"));
    }
    #[test]
    fn stream_requires_explicit_registered_runner_and_selects_ticket_ids() {
        let mut s = snapshot();
        let project = s.projects[0].id;
        let mut d = Dialog::new(Target::NewStream(project), &s).unwrap();
        d.insert("goal");
        assert!(d.build().is_err());
        let runner = RunnerId::new();
        s.runners.push(RunnerView {
            id: runner,
            name: "local".into(),
            location: RunnerLocation::Local,
            version: "test".into(),
            connection: ConnectionState::Online,
            last_seen_unix_ms: 1,
            opencode: OpenCodeStatus::NotConfigured,
        });
        control(&mut d, 'r', &s);
        d.focus = 1;
        assert!(d.build().is_err());
        d.key(
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            true,
            Some(&s),
        );
        d.focus = 2;
        d.key(
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            true,
            Some(&s),
        );
        let Request::Stream(body) = d.build().unwrap() else {
            panic!()
        };
        assert_eq!(body.runner_id, runner);
        assert_eq!(body.ticket_ids, vec![s.tickets[0].id]);
        s.runners.clear();
        control(&mut d, 'r', &s);
        assert!(d.build().is_err());
    }
}
