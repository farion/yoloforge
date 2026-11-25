use crate::{
    backend::{ArchiveRead, Backend, Event, Failure, InputAttachment},
    dialogs::Dialog,
    model::*,
    persistence::{PendingWrite, Settings},
};
use egui::{self, Color32, RichText};
use factory_core::*;
use factory_protocol::*;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

pub const BLUE: Color32 = Color32::from_rgb(54, 139, 231);
pub const GREEN: Color32 = Color32::from_rgb(93, 203, 137);
pub const RED: Color32 = Color32::from_rgb(248, 116, 107);
pub const MUTED: Color32 = Color32::from_rgb(173, 180, 192);
pub const AMBER: Color32 = Color32::from_rgb(244, 190, 84);

pub struct SessionTab {
    pub scroll_to_start: bool,
    pub selected_execution: Option<CommandId>,
    pub initialized: BTreeSet<CommandId>,
    pub older: BTreeMap<CommandId, Option<u64>>,
    pub message_positions: BTreeMap<String, u64>,
    pub root: CommandId,
    pub project: ProjectId,
    pub title: String,
    pub archives: BTreeMap<CommandId, Vec<std::sync::Arc<ArchivedMessage>>>,
    pub cursors: BTreeMap<CommandId, u64>,
    pub statuses: BTreeMap<CommandId, ArchiveStatus>,
    pub loading: BTreeMap<CommandId, u64>,
    pub error: Option<String>,
    pub follow: bool,
    pub show_versions: bool,
    pub next_poll: Instant,
}

pub struct App {
    pub session_prompts: BTreeMap<CommandId, Box<crate::dialogs::TaskDialog>>,
    pub local_runners: Vec<crate::local_runners::LocalRunner>,
    pub local_runners_busy: bool,
    pub(crate) restarting_runner: Option<RunnerId>,
    pub action_errors: Vec<(u64, String)>,
    restore_local_runners: bool,
    pub factory_flow: crate::overview::FlowState,
    pub markdown_cache: egui_commonmark::CommonMarkCache,
    pub task_payloads: BTreeMap<TicketId, std::sync::Arc<Ticket>>,
    pub task_loading: BTreeMap<TicketId, u64>,
    pub task_errors: BTreeMap<TicketId, (Revision, String)>,
    lists: BTreeMap<String, CachedList>,
    view_states: BTreeMap<(Option<ProjectId>, Page), ViewState>,
    pub backend: Backend,
    pub settings: Settings,
    pub server: String,
    pub snapshot: Option<std::sync::Arc<Snapshot>>,
    pub online: bool,
    pub access: Option<AccessView>,
    pub updated: Option<Instant>,
    pub error: Option<String>,
    pub instances: Vec<OpenCodeInstance>,
    pub clients: Vec<ClientPresence>,
    pub client_error: Option<String>,
    pub instance_error: Option<String>,
    pub notifications: Vec<Notification>,
    pub notification_states_loading: bool,
    pub next_notification_states: Instant,
    pub notification_states_updated: Option<Instant>,
    pub notification_error: Option<String>,
    pub unread: u64,
    pub notification_before: Option<u64>,
    pub notifications_initialized: bool,
    pub latest_notification: u64,
    pub events: Vec<FactoryEvent>,
    pub event_cursor: u64,
    pub events_loading: bool,
    pub event_before: Option<u64>,
    pub next_events_poll: Instant,
    pub tabs: Vec<SessionTab>,
    pub active_tab: usize,
    pub dialog: Option<Dialog>,
    pub factory_settings: Option<crate::project_settings::ProjectSettings>,
    pub models: crate::model_settings::Models,
    pub skill_settings: BTreeMap<ProjectId, crate::skill_settings::Pane>,
    pub factory_skills: crate::factory_skills::Catalog,
    pub mcp_settings: crate::mcp_settings::Settings,
    pub detail: Option<TicketId>,
    pub question_focus: Option<String>,
    pub query: String,
    pub task_filter: usize,
    pub show_answered: bool,
    pub sessions_closed: bool,
    pub diagnostics_project: bool,
    pub responses: BTreeMap<u64, Result<Value, Failure>>,
    pub in_flight: BTreeSet<u64>,
    pub notices: Vec<(String, Instant)>,
    pub next_request: u64,
    pub settings_dirty: bool,
    pub language_save: Option<(crate::i18n::Language, std::sync::mpsc::Receiver<bool>)>,
    pub next_save: Instant,
    pub previews: BTreeMap<String, Option<Result<egui::TextureHandle, String>>>,
    pub started: Instant,
    pub screenshot: Option<std::path::PathBuf>,
    pub screenshot_requested: bool,
    pub exit_after_ms: Option<u64>,
}
#[derive(Clone, Default)]
struct ViewState {
    query: String,
    filter: usize,
    answered: bool,
    sessions_closed: bool,
    detail: Option<TicketId>,
}
pub struct CachedList {
    pub list: egui_virtual_list::VirtualList,
    version: u64,
}
impl App {
    pub fn new(
        ctx: &egui::Context,
        server: String,
        screenshot: Option<std::path::PathBuf>,
        exit_after_ms: Option<u64>,
    ) -> Self {
        configure_style(ctx);
        egui_extras::install_image_loaders(ctx);
        let loaded = if cfg!(test) {
            Ok(Settings::default())
        } else {
            crate::persistence::load_settings()
        };
        let mut settings = loaded.as_ref().cloned().unwrap_or_default();
        #[cfg(test)]
        { settings.language = crate::i18n::Language::German; }
        crate::i18n::set_language(settings.language);
        let server = if server == "http://127.0.0.1:7432" && !settings.server.is_empty() {
            settings.server.clone()
        } else {
            server
        };
        let stored_credential = if settings.server == server {
            settings.credential_file.clone()
        } else {
            None
        };
        settings.server = server.clone();
        let backend = Backend::new(ctx.clone());
        let next_request = settings.pending.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        let mut app = Self {
            factory_flow: crate::overview::FlowState::default(),
            local_runners: vec![],
            session_prompts: BTreeMap::new(),
            local_runners_busy: false,
            restarting_runner: None,
            action_errors: vec![],
            restore_local_runners: true,
            markdown_cache: egui_commonmark::CommonMarkCache::default(),
            task_payloads: BTreeMap::new(),
            task_loading: BTreeMap::new(),
            task_errors: BTreeMap::new(),
            lists: BTreeMap::new(),
            view_states: BTreeMap::new(),
            backend,
            settings,
            server: server.clone(),
            snapshot: None,
            online: false,
            access: None,
            updated: None,
            error: loaded
                .err()
                .map(|e| crate::tr!("Einstellungen konnten nicht geladen werden: {e}", e = e)),
            instances: vec![],
            clients: vec![],
            client_error: None,
            instance_error: None,
            notifications: vec![],
            notification_states_loading: false,
            next_notification_states: Instant::now(),
            notification_states_updated: None,
            notification_error: None,
            unread: 0,
            notification_before: None,
            notifications_initialized: false,
            latest_notification: 0,
            events: vec![],
            event_cursor: 0,
            events_loading: false,
            event_before: None,
            next_events_poll: Instant::now(),
            tabs: vec![],
            active_tab: 0,
            dialog: None,
            factory_settings: None,
            models: Default::default(),
            detail: None,
            question_focus: None,
            query: String::new(),
            task_filter: 0,
            show_answered: false,
            sessions_closed: false,
            diagnostics_project: false,
            skill_settings: BTreeMap::new(),
            factory_skills: Default::default(),
            mcp_settings: Default::default(),
            responses: BTreeMap::new(),
            in_flight: BTreeSet::new(),
            notices: vec![],
            next_request,
            settings_dirty: false,
            language_save: None,
            next_save: Instant::now(),
            previews: BTreeMap::new(),
            started: Instant::now(),
            screenshot,
            screenshot_requested: false,
            exit_after_ms,
        };
        if !cfg!(test) {
            match crate::local_runners::load() {
                Ok(runners) => app.local_runners = runners,
                Err(e) => {
                    app.error = Some(e.to_string());
                    app.notice(crate::tr!("Gespeicherte lokale Runner konnten nicht geladen werden."));
                }
            }
        }
        let mut credential_file = None;
        let token = match (
            std::env::var("FACTORY_TOKEN"),
            std::env::var_os("FACTORY_TOKEN_FILE"),
        ) {
            (Ok(token), None) => Some(token),
            (Err(_), Some(path)) => {
                credential_file = Some(std::path::PathBuf::from(path));
                credential_file
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|s| s.trim_end().to_string())
            }
            (Err(_), None) => {
                credential_file = stored_credential;
                credential_file
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|s| s.trim_end().to_string())
            }
            (Ok(_), Some(_)) => {
                let error = crate::tr!("Nur FACTORY_TOKEN oder FACTORY_TOKEN_FILE setzen, nicht beides.");
                app.error = Some(error.into());
                app.notice(error);
                None
            }
        };
        if let Some(token) = token {
            if cfg!(test) {
                return app;
            }
            if let Err(error) = app.connect(server.clone(), token) {
                app.error = Some(error);
                app.notice(crate::tr!("Factory-Verbindung konnte nicht eingerichtet werden."));
            } else {
                app.settings.credential_file = credential_file;
            }
        } else {
            app.dialog = None;
        }
        app
    }
    pub fn connect(&mut self, server: String, token: String) -> Result<(), String> {
        if self.connection_active() {
            return Err(crate::tr!("Bitte zuerst die bestehende Verbindung trennen.").into());
        }
        self.backend
            .connect(&server, &token)
            .map_err(|e| e.to_string())?;
        if let Some(settings) = self.factory_settings.take() {
            settings.persist(self);
        }
        self.server = server.clone();
        self.restore_local_runners = true;
        self.settings.server = server;
        self.settings.credential_file = None;
        self.settings_dirty = true;
        self.snapshot = None;
        self.updated = None;
        self.online = false;
        self.access = None;
        self.error = None;
        self.instances.clear();
        self.instance_error = None;
        self.clients.clear();
        self.client_error = None;
        self.previews.clear();
        self.task_payloads.clear();
        self.task_loading.clear();
        self.task_errors.clear();
        self.lists.clear();
        self.view_states.clear();
        self.notifications.clear();
        self.notification_states_loading = false;
        self.notification_states_updated = None;
        self.events.clear();
        self.event_cursor = 0;
        self.tabs.clear();
        for (_, composer) in std::mem::take(&mut self.session_prompts) { composer.persist(self); }
        self.active_tab = 0;
        self.detail = None;
        self.responses.clear();
        self.models = Default::default();
        self.skill_settings.clear();
        self.factory_skills = Default::default();
        self.mcp_settings = Default::default();
        self.in_flight.clear();
        self.notifications_initialized = false;
        self.latest_notification = 0;
        Ok(())
    }
    pub fn connection_active(&self) -> bool {
        self.online || self.backend.is_connected()
    }
    pub fn runner_removed(&self, id: RunnerId) -> bool {
        self.local_runners.iter().any(|runner|
            runner.id == id && runner.server == self.server && runner.removed)
    }
    pub fn manage_local_runners(&mut self, action: crate::local_runners::Action) {
        if self.local_runners_busy || !self.online || !self.backend.is_connected() { return; }
        self.local_runners_busy = true;
        if let crate::local_runners::Action::Restart(id) = &action {
            self.restarting_runner = Some(*id);
            self.notice(crate::tr!("Lokaler Runner wird neu gestartet …"));
            self.backend.context().request_repaint();
        }
        self.backend.local_runners(self.server.clone(), action);
    }
    pub fn disconnect(&mut self) {
        egui::Popup::close_all(self.backend.context());
        self.backend.disconnect();
        self.online = false;
        self.access = None;
        self.error = None;
        self.notice(crate::tr!("Verbindung zur Factory getrennt."));
        self.enter_offline_mode();
    }
    pub fn project(&self) -> Option<&Project> {
        self.available_projects()
            .iter()
            .find(|p| Some(p.id) == self.settings.project)
    }
    pub fn page(&self) -> Page {
        self.settings.page
    }
    pub fn navigate(&mut self, target: Target) {
        if !self.online { self.enter_offline_mode(); return; }
        match target {
            Target::Task(_, task) => self.view_task(task),
            Target::Questions(project, session) => {
                self.set_project(Some(project));
                self.set_page(Page::Questions);
                self.question_focus = Some(session.clone());
                if let Some(s) = &self.snapshot {
                    let qs = question_groups_for_tab(s, project, false)
                        .remove(&session)
                        .unwrap_or_default();
                    if !qs.is_empty() {
                        self.show_answered = false;
                        self.dialog = Some(Dialog::questions(qs));
                    } else {
                        self.show_answered = true;
                        self.notice(crate::tr!("Keine offene Frage mehr in dieser Session. Bereits beantwortete Fragen werden angezeigt."));
                    }
                }
            }
            Target::Session(project, root) => self.open_session(project, root),
            Target::Orchestrator(project) => {
                self.set_project(Some(project));
                self.set_page(Page::Orchestrator);
                self.sessions_closed = false;
                self.query.clear();
            }
            Target::Diagnose(project) => {
                if let Some(p) = project {
                    self.set_project(Some(p));
                }
                self.set_page(Page::Diagnostics);
                self.diagnostics_project = project.is_some();
            }
        }
        self.settings_dirty = true;
    }
    pub fn set_page(&mut self, page: Page) {
        if !self.online && page != Page::Tasks { return; }
        if page == Page::Settings {
            self.dialog = Some(Dialog::FactorySettings);
            return;
        }
        if self.page() == Page::Settings && page != Page::Settings {
            if let Some(settings) = self.factory_settings.take() {
                settings.persist(self);
                self.factory_settings = Some(settings);
            }
        }
        self.remember_view();
        self.settings.page = page;
        self.active_tab = 0;
        self.restore_view();
        self.settings_dirty = true;
        if page == Page::Diagnostics && self.events.is_empty() && !self.events_loading {
            self.events_loading = true;
            self.backend.history(None);
        }
    }
    fn view_key(&self) -> (Option<ProjectId>, Page) {
        (
            if self.page().global() {
                None
            } else {
                self.settings.project
            },
            self.page(),
        )
    }
    fn remember_view(&mut self) {
        self.view_states.insert(
            self.view_key(),
            ViewState {
                query: self.query.clone(),
                filter: self.task_filter,
                answered: self.show_answered,
                sessions_closed: self.sessions_closed,
                detail: self.detail,
            },
        );
    }
    fn restore_view(&mut self) {
        let state = self
            .view_states
            .get(&self.view_key())
            .cloned()
            .unwrap_or_default();
        self.query = state.query;
        self.task_filter = state.filter;
        self.show_answered = state.answered;
        self.sessions_closed = state.sessions_closed;
        self.detail = state.detail;
    }
    pub fn set_project(&mut self, project: Option<ProjectId>) {
        if self.settings.project != project {
            self.remember_view();
            self.settings.project = project;
            self.restore_view();
            self.settings_dirty = true;
        }
    }
    pub fn open_session(&mut self, project: ProjectId, root: CommandId) {
        if !self.online { return; }
        if self.snapshot.as_ref().is_none_or(|s| {
            !s.commands
                .iter()
                .any(|c| c.session_root() == root && command_project(s, c) == Some(project))
        }) {
            self.notice(crate::tr!("Session ist im aktuellen Serverstand nicht verfügbar. Projekt und Verbindung prüfen."));
            return;
        }
        if let Some(index) = self.tabs.iter().position(|t| t.root == root) {
            self.active_tab = index + 1;
            return;
        }
        let title = self
            .snapshot
            .as_ref()
            .and_then(|s| {
                s.commands
                    .iter()
                    .find(|c| c.session_root() == root)
                    .map(|c| session_title(s, c))
            })
            .unwrap_or_else(|| root.to_string());
        self.tabs.push(SessionTab {
            scroll_to_start: false,
            selected_execution: None,
            initialized: BTreeSet::new(),
            older: BTreeMap::new(),
            message_positions: BTreeMap::new(),
            root,
            project,
            title,
            archives: BTreeMap::new(),
            cursors: BTreeMap::new(),
            statuses: BTreeMap::new(),
            loading: BTreeMap::new(),
            error: None,
            follow: true,
            show_versions: false,
            next_poll: Instant::now(),
        });
        self.active_tab = self.tabs.len();
    }
    pub fn write(&mut self, label: &str, method: &str, path: String, body: Value) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        self.settings.pending.push(PendingWrite {
            id,
            server: self.server.clone(),
            method: method.into(),
            path: path.clone(),
            body: body.clone(),
            label: label.into(),
        });
        self.settings_dirty = true;
        self.in_flight.insert(id);
        // Persist intent before HTTP, even if the process is closed during admission.
        // Backend serializes this write before starting the request.
        self.backend
            .durable_request(id, self.settings.clone(), method.into(), path, body);
        id
    }

    pub fn requeue_task(&mut self, ticket: &Ticket) {
        if self.snapshot.as_ref().is_none_or(|s| !retryable_task(s, ticket)) { return; }
        let Some(previous_command) = ticket.execution_command else { return; };
        let path = format!("/v1/tasks/{}/retry", ticket.id);
        if !self.online || self.settings.pending.iter().any(|r| r.server == self.server && r.path == path) {
            return;
        }
        self.write(crate::tr!("Intent erneut einreihen"), "POST", path, serde_json::json!(factory_protocol::RetryTask {
            request_id: RequestId::new(), expected_revision: ticket.revision, previous_command,
        }));
    }
    pub(crate) fn project_pause_pending(&self, project: ProjectId) -> bool {
        let path = format!("/v1/projects/{project}/pause");
        self.settings.pending.iter().any(|r| r.server == self.server && r.path == path)
    }
    pub(crate) fn toggle_project_pause(&mut self, project: &Project) {
        if !self.online || self.project_pause_pending(project.id) {
            return;
        }
        self.write(
            if project.paused { crate::tr!("Projekt fortsetzen") } else { crate::tr!("Projekt pausieren") },
            "POST", format!("/v1/projects/{}/pause", project.id),
            serde_json::json!(PauseProject { request_id: RequestId::new(),
                expected_revision: project.revision, paused: !project.paused }),
        );
    }
    pub fn retry(&mut self, id: u64) {
        if !self.online || self.in_flight.contains(&id) {
            return;
        }
        if let Some(r) = self
            .settings
            .pending
            .iter()
            .find(|r| r.id == id && r.server == self.server)
        {
            if r.path == "/v1/devices/enrollment" {
                self.notice(crate::tr!("Einladungserstellung hat keinen idempotenten Abgleich. Unklare Einladungen nicht erneut senden; Gültigkeit abwarten oder Gerätezugang prüfen."));
                return;
            }
            self.in_flight.insert(id);
            self.responses.remove(&id);
            self.backend
                .request(id, r.method.clone(), r.path.clone(), r.body.clone());
        }
    }
    pub fn notice(&mut self, message: impl Into<String>) {
        self.notices.push((message.into(), Instant::now()));
    }
    fn connection_lost(&mut self, error: String) {
        egui::Popup::close_all(self.backend.context());
        if self.online {
            self.notice(crate::tr!("Verbindung zur Factory unterbrochen."));
        } else if self.error.is_none() {
            self.notice(crate::tr!("Verbindung zur Factory fehlgeschlagen."));
        }
        self.online = false;
        self.error = Some(error);
        self.enter_offline_mode();
    }
    pub fn task_data(&mut self, ticket: &Ticket) -> Option<std::sync::Arc<Ticket>> {
        if self.snapshot.as_ref().is_none_or(|s| !s.payloads_deferred)
            || (ticket.images.is_empty() && ticket.files.is_empty())
        {
            return Some(std::sync::Arc::new(ticket.clone()));
        }
        if let Some(data) = self
            .task_payloads
            .get(&ticket.id)
            .filter(|data| data.revision == ticket.revision)
        {
            return Some(data.clone());
        }
        if self
            .task_errors
            .get(&ticket.id)
            .is_some_and(|(revision, _)| *revision != ticket.revision)
        {
            self.task_errors.remove(&ticket.id);
        }
        if !self.task_errors.contains_key(&ticket.id)
            && self.online
            && !self.task_loading.contains_key(&ticket.id)
        {
            let request = self.next_request;
            self.next_request += 1;
            self.task_loading.insert(ticket.id, request);
            self.backend
                .task_payload(ticket.id, ticket.revision, request);
        }
        None
    }
    pub fn edit_task(&mut self, id: TicketId) {
        let ticket = self
            .snapshot
            .as_ref()
            .and_then(|s| s.tickets.iter().find(|t| t.id == id))
            .cloned();
        if let Some(ticket) = ticket {
            if !self.online {
                self.notice(crate::tr!("Zum sicheren Bearbeiten zuerst mit der Factory verbinden."));
                return;
            }
            if let Some(pending) = self
                .settings
                .pending
                .iter()
                .find(|p| {
                    p.server == self.server
                        && crate::model_settings::pending_task(&p.path, &p.body)
                            .is_some_and(|task| task["id"] == serde_json::json!(id))
                })
                .cloned()
            {
                if let Some((submitted, implementer, local_mcps)) = crate::model_settings::submitted(&pending.path, &pending.body) {
                    let mut dialog = Dialog::task(self.server.clone(), ticket.project_id, None);
                    if let Dialog::Task(task) = &mut dialog {
                        task.draft = crate::persistence::TaskDraft {
                            edit_token: submitted.edit_token,
                            server: self.server.clone(),
                            project: submitted.task.project_id,
                            ticket: submitted.task.id,
                            revision: submitted.task.expected_revision,
                            prompt: submitted.task.prompt,
                            images: submitted.task.images,
                            files: submitted.files,
                            requested_session: submitted.task.requested_session,
                            implementer,
                            local_mcps,
                            model_loaded: true,
                        };
                        task.request = Some(pending.id);
                    }
                    self.retry(pending.id);
                    self.dialog = Some(dialog);
                    return;
                }
            }
            if self
                .snapshot
                .as_ref()
                .is_none_or(|s| !ticket.draft && !s.editable_tasks.contains_key(&id))
            {
                self.notice(crate::tr!("Diese Aufgabe kann nicht mehr bearbeitet werden oder der Server benötigt ein Update."));
                return;
            }
            let path = format!("/v1/tasks/{id}/edit");
            let pending = self
                .settings
                .pending
                .iter()
                .find(|p| p.server == self.server && p.path == path)
                .map(|p| p.id);
            let request = if let Some(request) = pending {
                self.retry(request);
                request
            } else {
                let edit_token = self
                    .settings
                    .task_drafts
                    .iter()
                    .find(|d| d.server == self.server && d.ticket == Some(id))
                    .and_then(|d| d.edit_token);
                self.write(
                    crate::tr!("Aufgabe zum Bearbeiten sichern"),
                    "POST",
                    path,
                    serde_json::json!(BeginTaskEdit {
                        request_id: RequestId::new(),
                        expected_revision: ticket.revision,
                        edit_token
                    }),
                )
            };
            self.dialog = Some(Dialog::TaskEditing { id, request });
        } else {
            self.notice(crate::tr!("Aufgabe nicht mehr vorhanden."));
        }
    }
    pub fn view_task(&mut self, id: TicketId) {
        let Some(ticket) = self
            .snapshot
            .as_ref()
            .and_then(|s| s.tickets.iter().find(|t| t.id == id))
            .cloned()
        else {
            self.notice(crate::tr!("Aufgabe nicht mehr vorhanden."));
            return;
        };
        let data = self.task_data(&ticket);
        self.dialog = Some(Dialog::task_read_only(
            self.server.clone(),
            ticket.project_id,
            data.as_deref().unwrap_or(&ticket),
            data.is_some(),
        ));
    }
    pub(crate) fn accept_task_edit(&mut self, request: u64, edit: TaskEdit) {
        let ticket = edit.ticket;
        let newer = self
            .snapshot
            .as_ref()
            .and_then(|s| {
                s.tickets
                    .iter()
                    .find(|t| t.id == ticket.id && t.revision.0 > ticket.revision.0)
            })
            .cloned();
        if newer.as_ref().is_some_and(|t| !t.draft) {
            self.notice(crate::tr!("Aufgabe wurde inzwischen geändert. Bitte den aktuellen Stand prüfen; lokale Entwürfe bleiben erhalten."));
            return;
        }
        if let Some(snapshot) = self.snapshot.as_mut().filter(|_| newer.is_none()) {
            let snapshot = std::sync::Arc::make_mut(snapshot);
            if let Some(current) = snapshot.tickets.iter_mut().find(|t| t.id == ticket.id) {
                *current = ticket.clone();
                if snapshot.payloads_deferred {
                    for image in &mut current.images {
                        image.data.clear();
                    }
                    for file in &mut current.files {
                        file.data.clear();
                    }
                }
            }
            snapshot.editable_tasks.insert(ticket.id, ticket.revision);
        }
        let mut dialog = Dialog::task(self.server.clone(), ticket.project_id, Some(&ticket));
        if let Dialog::Task(task) = &mut dialog {
            if let Some(draft) = self
                .settings
                .task_drafts
                .iter()
                .find(|d| d.server == self.server && d.ticket == Some(ticket.id))
            {
                 task.draft = draft.clone();
                if task.draft.revision == ticket.revision
                    || (task.draft.edit_token.is_none()
                        && task.draft.revision.0.checked_add(1) == Some(ticket.revision.0))
                {
                    task.draft.revision = ticket.revision;
                } else {
                    task.error=Some(crate::tr!("Lokaler Entwurf basiert auf einem anderen Serverstand. Bitte vor dem Speichern vergleichen.").into());
                }
            }
            task.draft.edit_token = Some(edit.edit_token);
            if task.draft.local_mcps.is_none() {
                task.draft.local_mcps = Some(edit.local_mcps);
            }
            if newer.is_some() {
                task.error=Some(crate::tr!("Serverstand wurde während der Reservierung geändert. Entwurf und Reservierung bleiben erhalten; bitte vergleichen.").into());
            }
            task.persist(self);
        }
        if newer.is_none() {
            self.task_payloads
                .insert(ticket.id, std::sync::Arc::new(ticket));
        }
        if matches!(self.dialog,Some(Dialog::TaskEditing {request:pending,..}) if pending==request)
        {
            self.dialog = Some(dialog);
        }
    }
    pub fn take_list(&mut self, key: &str, version: u64) -> CachedList {
        let mut cache = self.lists.remove(key).unwrap_or_else(|| CachedList {
            list: egui_virtual_list::VirtualList::new(),
            version,
        });
        cache.list.hide_on_resize(None);
        if cache.version != version {
            cache.list.reset();
            cache.version = version;
        }
        cache
    }
    pub fn put_list(&mut self, key: String, cache: CachedList) {
        if self.lists.len() >= 64 {
            if let Some(old) = self.lists.keys().next().cloned() {
                self.lists.remove(&old);
            }
        }
        self.lists.insert(key, cache);
    }
    pub fn preview_image(
        &mut self,
        ui: &mut egui::Ui,
        key: String,
        image: &PromptImage,
        size: egui::Vec2,
        full: bool,
    ) -> egui::Response {
        self.preview_image_surface(ui, key, image, size, full)
            .on_hover_text(&image.name)
    }
    pub fn preview_image_surface(
        &mut self,
        ui: &mut egui::Ui,
        key: String,
        image: &PromptImage,
        size: egui::Vec2,
        full: bool,
    ) -> egui::Response {
        if !ui.is_rect_visible(egui::Rect::from_min_size(ui.next_widget_position(), size)) {
            return ui.allocate_exact_size(size, egui::Sense::hover()).1;
        }
        if !self.previews.contains_key(&key) {
            if self.previews.len() >= 128 {
                if let Some(old) = self.previews.keys().find(|k| *k != &key).cloned() {
                    self.previews.remove(&old);
                }
            }
            self.previews.insert(key.clone(), None);
            self.backend.preview(key.clone(), image.clone(), full);
        }
        match self.previews.get(&key).and_then(Option::as_ref) {
            Some(Ok(texture)) => ui.add(
                egui::Image::new(texture)
                    .max_size(size)
                    .sense(egui::Sense::click()),
            ),
            Some(Err(error)) => ui
                .add_sized(
                    size,
                    egui::Label::new(crate::tr!("Bildvorschau nicht verfügbar")).wrap(),
                )
                .on_hover_text(error),
            None => ui.add_sized(size, egui::Label::new(crate::tr!("Bild lädt …"))),
        }
    }
    pub fn read(&mut self, title: &str, path: String) {
        self.read_with_method(title, "GET", path);
    }
    pub fn inspect(&mut self, title: &str, value: Value) {
        let id = self.next_request;
        self.next_request += 1;
        self.responses.insert(id, Ok(value));
        self.dialog = Some(Dialog::read(title.into(), id, String::new()));
    }
    pub fn read_with_method(&mut self, title: &str, method: &str, path: String) {
        let id = self.next_request;
        self.next_request += 1;
        self.in_flight.insert(id);
        self.backend
            .request(id, method.into(), path.clone(), Value::Null);
        self.dialog = Some(Dialog::read(title.into(), id, path));
    }
    pub fn notification_target(&self, n: &Notification) -> Option<Target> {
        let Some(project) = n.project_id else { return Some(Target::Diagnose(None)) };
        if let Some(q) = n.question_id {
            let session = self
                .snapshot
                .as_ref()?
                .questions
                .iter()
                .find(|v| v.id == q)?
                .session_id
                .clone();
            return Some(Target::Questions(project, session));
        }
        if let Some(t) = n.ticket_id
            .filter(|id| self.snapshot.as_ref().is_some_and(|s| s.tickets.iter().any(|t| t.id == *id)))
        {
            return Some(Target::Task(project, t));
        }
        if let Some(id) = n.command_id {
            let snapshot = self.snapshot.as_ref()?;
            let c = snapshot.commands.iter().find(|c| c.id == id)?;
            if c.kind != CommandKind::RunAgent {
                if snapshot.streams.iter().any(|w| w.id == c.stream_id && w.goal == "Orchestrator") {
                    return Some(Target::Orchestrator(project));
                }
                return Some(Target::Diagnose(Some(project)));
            }
            return Some(Target::Session(project, c.session_root()));
        }
        Some(Target::Diagnose(Some(project)))
    }

    pub fn notification_reads_pending(&self) -> BTreeSet<u64> {
        self.settings.pending.iter().filter(|r| r.server == self.server && r.path == "/v1/notifications/read")
            .filter_map(|r| serde_json::from_value::<MarkNotifications>(r.body.clone()).ok())
            .filter(|r| r.read).flat_map(|r| r.ids).collect()
    }

    pub fn visible_unread_notifications(&self, pending: &BTreeSet<u64>) -> u64 {
        let waiting = self.notifications.iter().filter(|n| !n.read && pending.contains(&n.id)).count();
        self.unread.saturating_sub(waiting as u64)
    }

    pub fn open_notification(&mut self, id: u64, target: Target) {
        self.mark_notifications_read([id]);
        self.navigate(target);
    }

    pub fn mark_notifications_read(&mut self, ids: impl IntoIterator<Item = u64>) {
        if !self.online {
            return;
        }
        let pending = self.notification_reads_pending();
        let ids: BTreeSet<_> = ids.into_iter().filter(|id| !pending.contains(id) &&
            !self.notifications.iter().any(|n| n.id == *id && n.read)).collect();
        for ids in ids.into_iter().collect::<Vec<_>>().chunks(1000) {
            let body = MarkNotifications { request_id: RequestId::new(), ids: ids.to_vec(), read: true };
            self.write(crate::tr!("Benachrichtigung gelesen"), "POST", "/v1/notifications/read".into(), serde_json::json!(body));
        }
    }
    pub(crate) fn receive(&mut self) {
        for _ in 0..128 {
            let Ok(envelope) = self.backend.rx.try_recv() else {
                break;
            };
            if envelope.generation != self.backend.generation && envelope.generation != u64::MAX {
                continue;
            }
            match envelope.event {
                Event::Snapshot(Ok(mut s)) => {
                    let reconnected = !self.online && (self.updated.is_some() || self.error.is_some());
                    if self
                        .snapshot
                        .as_ref()
                        .is_some_and(|old| old.event_cursor > s.event_cursor)
                    {
                        self.events.clear();
                        self.event_cursor = 0;
                        self.event_before = None;
                        self.events_loading = false;
                        self.notifications.clear();
                        self.notifications_initialized = false;
                        self.latest_notification = 0;
                        for tab in &mut self.tabs {
                            tab.archives.clear();
                            tab.initialized.clear();
                            tab.older.clear();
                            tab.message_positions.clear();
                            tab.cursors.clear();
                            tab.statuses.clear();
                            tab.loading.clear();
                        }
                        self.notice(crate::tr!("Serverstand zurückgesetzt. Verlaufsdaten werden neu abgeglichen; offene Anfragen bleiben zur Prüfung erhalten."));
                    }
                    if self
                        .settings
                        .project
                        .is_none_or(|p| !s.projects.iter().any(|v| v.id == p))
                    {
                        self.settings.project = s.projects.first().map(|p| p.id);
                    }
                    if let Some(old) = self.snapshot.as_ref().filter(|old|
                        old.boot_id == s.boot_id && old.event_cursor <= s.event_cursor)
                    {
                        // A snapshot in flight can predate the answer's write receipt.
                        let previous: std::collections::HashMap<_, _> =
                            old.questions.iter().map(|q| (q.id, q)).collect();
                        for question in &mut s.questions {
                            if let Some(newer) = previous.get(&question.id)
                                .filter(|newer| newer.revision.0 > question.revision.0)
                            {
                                *question = (**newer).clone();
                            }
                        }
                    }
                    self.cache_projects(&s.projects);
                    self.snapshot = Some(std::sync::Arc::new(s));
                    self.updated = Some(Instant::now());
                    self.online = true;
                    self.error = None;
                    self.sync_offline_tasks(reconnected);
                    if self.restore_local_runners && !self.local_runners_busy && !cfg!(test) {
                        self.restore_local_runners = false;
                        self.manage_local_runners(crate::local_runners::Action::Restore);
                    }
                    if reconnected {
                        self.notice(crate::tr!("Wieder mit der Factory verbunden."));
                    }
                }
                Event::Snapshot(Err(e)) => {
                    self.connection_lost(e);
                }
                Event::Instances(result) => match result {
                    Ok(v) => {
                        self.instances = v;
                        self.instance_error = None;
                    }
                    Err(e) => {
                        self.instances.clear();
                        self.instance_error = Some(e);
                    }
                },
                Event::Clients(result) => match result {
                    Ok(clients) => {
                        self.clients = clients;
                        self.client_error = None;
                    }
                    Err(error) => {
                        self.client_error = Some(error);
                        self.clients.clear();
                    }
                },
                Event::Notifications(result, append) => match result {
                    Ok(page) => {
                        if !append {
                            if self.notifications_initialized {
                                for n in page
                                    .entries
                                    .iter()
                                    .rev()
                                    .filter(|n| n.id > self.latest_notification)
                                {
                                    let project = n
                                        .project_id
                                        .and_then(|p| {
                                            self.snapshot
                                                .as_ref()?
                                                .projects
                                                .iter()
                                                .find(|v| v.id == p)
                                        })
                                        .map(|p| p.name.as_str())
                                        .unwrap_or("Factory");
                                    let label = format!("{project}: {}", n.title);
                                    self.notices.push((label.clone(), Instant::now()));
                                    if self.settings.desktop_notifications {
                                        self.backend.desktop_notification(
                                            n.id,
                                            label,
                                            self.notification_target(n).unwrap_or(Target::Diagnose(n.project_id)),
                                        );
                                    }
                                }
                            }
                            self.latest_notification = self
                                .latest_notification
                                .max(page.entries.first().map_or(0, |n| n.id));
                            self.notifications_initialized = true;
                            // Keep already loaded older pages, refresh changed read flags in the head.
                            let min = page.entries.last().map_or(u64::MAX, |n| n.id);
                            self.notifications.retain(|n| n.id < min);
                            self.notifications.splice(0..0, page.entries);
                        } else {
                            for n in page.entries {
                                if !self.notifications.iter().any(|v| v.id == n.id) {
                                    self.notifications.push(n);
                                }
                            }
                        }
                        self.unread = page.unread;
                        if append || self.notifications.len() <= 100 {
                            self.notification_before = page.next_before;
                        }
                        self.notification_error = None;
                    }
                    Err(e) => self.notification_error = Some(e),
                },
                Event::Events(result) => {
                    self.events_loading = false;
                    match result {
                        Ok(p) => {
                            self.event_cursor = p.next_cursor;
                            for event in p.events {
                                if !self.events.iter().any(|v| v.sequence == event.sequence) {
                                    self.events.push(event);
                                }
                            }
                        }
                        Err(e) => self.notice(e),
                    }
                }
                Event::NotificationStates(result) => {
                    self.notification_states_loading = false;
                    match result {
                        Ok(states) => {
                            let states = states
                                .into_iter()
                                .map(|s| (s.id, s.read))
                                .collect::<BTreeMap<_, _>>();
                            for notification in &mut self.notifications {
                                if let Some(read) = states.get(&notification.id) {
                                    notification.read = *read;
                                }
                            }
                            self.notification_states_updated = Some(Instant::now());
                        }
                        Err(e) => {
                            self.notice(crate::tr!("Lesestatus konnte nicht aktualisiert werden: {e}", e = e))
                        }
                    }
                }
                Event::Request(id, result) => {
                    self.next_events_poll = Instant::now();
                    self.in_flight.remove(&id);
                    let mut result = result;
                    if let Some(read) = self.settings.pending.iter()
                        .find(|r| r.id == id && r.path == "/v1/notifications/read")
                        .and_then(|r| serde_json::from_value::<MarkNotifications>(r.body.clone()).ok())
                    {
                        match &result {
                            Ok(_) => {
                                let mut changed = 0;
                                for notification in &mut self.notifications {
                                    if read.ids.contains(&notification.id) && notification.read != read.read {
                                        notification.read = read.read;
                                        changed += 1;
                                    }
                                }
                                self.unread = if read.read { self.unread.saturating_sub(changed) } else { self.unread + changed };
                            }
                            Err(error) => self.notice(crate::tr!("Lesestatus nicht bestätigt: {}", error.message)),
                        }
                    }
                    if let Some(request) = self
                        .settings
                        .pending
                        .iter()
                        .find(|r| {
                            r.id == id
                                && r.path.starts_with("/v1/tasks/")
                                && r.path.ends_with("/edit")
                        })
                        .cloned()
                    {
                        if let Ok(value) = &result {
                            let token = request
                                .body
                                .get("edit_token")
                                .filter(|v| !v.is_null())
                                .or_else(|| request.body.get("request_id"))
                                .and_then(|v| serde_json::from_value::<RequestId>(v.clone()).ok());
                            match serde_json::from_value::<TaskEdit>(value.clone()) {
                                Ok(edit) if edit.ticket.draft && request.path==format!("/v1/tasks/{}/edit",edit.ticket.id) && token==Some(edit.edit_token)=>self.accept_task_edit(id,edit),
                                _=>result=Err(Failure {message:crate::tr!("Reservierung nicht eindeutig bestätigt. Bitte dieselbe gespeicherte Anfrage abgleichen; keinen neuen Bearbeitungsversuch erzeugen.").into(),uncertain:true}),
                            }
                        }
                    }
                    if let Some(request) = self.settings.pending.iter().find(|r| r.id == id
                        && r.server == self.server && r.path.starts_with("/v1/projects/")
                        && r.path.ends_with("/pause"))
                    {
                        if let Ok(value) = &result {
                            match serde_json::from_value::<Project>(value.clone()) {
                                Ok(project) if request.path == format!("/v1/projects/{}/pause", project.id)
                                    && request.body["paused"].as_bool() == Some(project.paused) =>
                                {
                                    let paused = project.paused;
                                    if let Some(snapshot) = &mut self.snapshot {
                                        if let Some(current) = std::sync::Arc::make_mut(snapshot).projects.iter_mut()
                                            .find(|p| p.id == project.id && p.revision.0 < project.revision.0)
                                        {
                                            *current = project;
                                        }
                                    }
                                    self.notice(if paused { crate::tr!("Projekt pausiert.") } else { crate::tr!("Projekt fortgesetzt.") });
                                }
                                _ => result = Err(Failure { message: crate::tr!("Projektpause nicht eindeutig bestätigt.").into(), uncertain: true }),
                            }
                        }
                        if let Err(error) = &result {
                            self.notice(if error.uncertain {
                                crate::tr!("{} In der Diagnose über „Zustellungen prüfen“ abgleichen.", error.message)
                            } else { error.message.clone() });
                        }
                    }
                    let mut task_notice = None;
                    if let Some(request) = self.settings.pending.iter().find(|r| r.id == id
                        && r.method == "POST" && r.path.starts_with("/v1/tasks/") && r.path.ends_with("/retry")).cloned()
                    {
                        if let Ok(value) = &result {
                            match serde_json::from_value::<Ticket>(value.clone()) {
                                Ok(ticket) if request.path == format!("/v1/tasks/{}/retry", ticket.id)
                                    && ticket.execution_command.is_some()
                                    && serde_json::to_value(ticket.execution_command).ok().as_ref() != Some(&request.body["previous_command"]) =>
                                {
                                    if request.server == self.server {
                                        if let Some(snapshot) = &mut self.snapshot {
                                            if let Some(current) = std::sync::Arc::make_mut(snapshot).tickets.iter_mut()
                                                .find(|t| t.id == ticket.id && t.revision.0 <= ticket.revision.0)
                                            { *current = ticket; }
                                        }
                                    }
                                    task_notice = Some(crate::tr!("Intent erneut eingereiht."));
                                }
                                _ => result = Err(Failure { message: crate::tr!("Erneutes Einreihen nicht eindeutig bestätigt.").into(), uncertain: true }),
                            }
                        }
                        if let Err(error) = &result {
                            self.notice(crate::tr!("Intent konnte nicht erneut eingereiht werden. Details stehen in der Diagnose."));
                            self.action_errors.push((crate::overview::now_ms(), format!("{}: {}", request.label, error.message)));
                        }
                    }
                    if let Some(request) = self.settings.pending.iter().find(|request| request.id == id
                        && request.method == "DELETE"
                        && request.path.strip_prefix("/v1/projects/").is_some_and(|id| id.parse::<ProjectId>().is_ok())).cloned()
                    {
                        if let Ok(value) = &result {
                            match serde_json::from_value::<Project>(value.clone()) {
                                Ok(project) if project.deleted && request.path == format!("/v1/projects/{}", project.id) => {
                                    if request.server == self.server {
                                        if let Some(snapshot) = &mut self.snapshot {
                                            std::sync::Arc::make_mut(snapshot).projects.retain(|current| current.id != project.id);
                                        }
                                        if self.settings.project == Some(project.id) {
                                            self.set_project(None);
                                        }
                                    }
                                    task_notice = Some(crate::tr!("Projekt gelöscht."));
                                }
                                _ => result = Err(Failure {
                                    message: crate::tr!("Löschen des Projekts nicht eindeutig bestätigt.").into(),
                                    uncertain: true,
                                }),
                            }
                        }
                        if let Err(error) = &result {
                            self.notice(if error.uncertain {
                                crate::tr!("Löschen nicht bestätigt. Bitte die Zustellung prüfen.")
                            } else {
                                crate::tr!("Projekt konnte nicht gelöscht werden. Bitte den aktuellen Projektstand prüfen.")
                            });
                            self.action_errors.push((crate::overview::now_ms(), format!("{}: {}", request.label, error.message)));
                        }
                    }
                    let offline_task = self.settings.offline_tasks.iter().find(|t| t.sync_request == Some(id)).cloned();
                    if let (Some(task), Ok(value)) = (&offline_task, &result) {
                        let confirmed = serde_json::from_value::<Ticket>(value.clone()).ok().is_some_and(|ticket|
                            ticket.draft && ticket.project_id == task.draft.project
                                && ticket.prompt.as_deref() == Some(task.draft.prompt.as_str())
                                && ticket.images == task.draft.images && ticket.files == task.draft.files);
                        if !confirmed {
                            result = Err(Failure { message: crate::tr!("Offline-Übertragung nicht eindeutig als Server-Entwurf bestätigt.").into(), uncertain: true });
                        }
                    }
                    if let Some(request) = self.settings.pending.iter().find(|r| r.id == id &&
                        r.path.starts_with("/v1/devices/") && r.path.ends_with("/revoke")).cloned()
                    {
                        if let Ok(value) = &result {
                            match serde_json::from_value::<Device>(value.clone()) {
                                Ok(device) if device.revoked && request.path == format!("/v1/devices/{}/revoke", device.id) => {
                                    for cached in self.responses.values_mut() {
                                        if let Ok(serde_json::Value::Array(entries)) = cached {
                                            for entry in entries {
                                                if entry["id"] == serde_json::json!(device.id) && entry.get("revoked").is_some() {
                                                    *entry = serde_json::json!(device);
                                                }
                                            }
                                        }
                                    }
                                    task_notice = Some(crate::tr!("Gerätezugang widerrufen."));
                                }
                                _ => result = Err(Failure { message: crate::tr!("Gerätewiderruf nicht eindeutig bestätigt.").into(), uncertain: true }),
                            }
                        }
                        if let Err(error) = &result {
                            self.notice(crate::tr!("Gerätezugang konnte nicht bestätigt widerrufen werden. Details stehen in der Diagnose."));
                            self.action_errors.push((crate::overview::now_ms(), crate::tr!("Gerätezugang: {}", error.message)));
                        }
                    }
                    if let Some(request) = self.settings.pending.iter().find(|r| r.id == id &&
                        r.path.starts_with("/v1/commands/") && r.path.ends_with("/prompt")).cloned()
                    {
                        if let Ok(value) = &result {
                            match serde_json::from_value::<Ticket>(value.clone()) {
                                Ok(ticket) if ticket.requested_session.is_some_and(|root|
                                    request.path == format!("/v1/commands/{root}/prompt")) => {
                                    self.settings.session_prompts.retain(|saved|
                                        saved.draft.server != request.server || serde_json::json!(saved.id) != request.body["request_id"]);
                                    task_notice = Some(crate::tr!("Prompt in die Session eingereiht."));
                                }
                                _ => result = Err(Failure { message: crate::tr!("Session-Prompt nicht eindeutig bestätigt.").into(), uncertain: true }),
                            }
                        }
                        if let Err(error) = &result {
                            self.notice(if error.uncertain {
                                crate::tr!("Prompt-Zustellung unklar. Bitte dieselbe Anfrage in der Diagnose über „Zustellungen prüfen“ abgleichen.")
                            } else { crate::tr!("Prompt konnte nicht eingereiht werden. Der Entwurf bleibt erhalten.") });
                            self.action_errors.push((crate::overview::now_ms(), format!("Session-Prompt ({} · {}): {}", request.server, request.path, error.message)));
                            if self.action_errors.len() > 50 { self.action_errors.remove(0); }
                        }
                    }
                    let project_update = self.settings.pending.iter().any(|r| r.id == id
                        && crate::dialogs::is_project_update(&r.method, &r.path));
                    let batch_update = matches!(&self.dialog, Some(Dialog::ProjectSettings(settings)) if settings.owns_request(id));
                    if project_update {
                        match &result {
                            Ok(value) => {
                                if let (Ok(project), Some(snapshot)) = (
                                    serde_json::from_value::<Project>(value.clone()), &mut self.snapshot,
                                ) {
                                    if let Some(current) = std::sync::Arc::make_mut(snapshot).projects.iter_mut()
                                        .find(|p| p.id == project.id && p.revision.0 < project.revision.0)
                                    {
                                        *current = project;
                                    }
                                }
                                if !batch_update { self.notice(crate::tr!("Projekteinstellungen gespeichert.")); }
                            }
                            Err(error) if !batch_update && error.uncertain => self.notice(crate::tr!("Speichern nicht bestätigt. Bitte Anfrage abgleichen.")),
                            Err(_) if !batch_update => self.notice(crate::tr!("Projekteinstellungen konnten nicht gespeichert werden.")),
                            Err(_) => {}
                        }
                    }
                    if let Ok(value) = &result {
                        if let Some(request) = self.settings.pending.iter().find(|r| r.id == id) {
                            self.settings.form_drafts.retain(|draft| {
                                draft.server != request.server
                                    || draft.path != request.path
                                    || draft.method != request.method
                                    || !crate::dialogs::same_form_body(&request.method, &request.path, &draft.body, &request.body)
                            });
                             if let Some((submitted, implementer, local_mcps)) =
                                crate::model_settings::submitted(&request.path, &request.body) {
                                    self.settings.task_drafts.retain(|d| {
                                        !(d.server == request.server
                                            && d.project == submitted.task.project_id
                                            && d.ticket == submitted.task.id
                                            && d.prompt == submitted.task.prompt
                                            && d.images == submitted.task.images
                                            && d.files == submitted.files
                                         && d.implementer == implementer
                                         && d.local_mcps == local_mcps)
                                    });
                                    task_notice = Some(if submitted.draft {
                                        crate::tr!("Entwurf gespeichert.")
                                    } else {
                                        crate::tr!("Aufgabe eingereiht.")
                                    });
                            }
                            if request.path.starts_with("/v1/questions/") && request.path.ends_with("/answer") {
                                if let Ok(question) =
                                    serde_json::from_value::<Question>(value.clone())
                                {
                                    if request.path == format!("/v1/questions/{}/answer", question.id)
                                        && question.answer_request.is_some_and(|id|
                                            serde_json::json!(id) == request.body["request_id"])
                                    {
                                        self.settings.question_drafts.retain(|d| {
                                            d.server != request.server || d.question != question.id
                                        });
                                        if let Some(snapshot) = self.snapshot.as_mut().filter(|_| request.server == self.server) {
                                            if let Some(current) = std::sync::Arc::make_mut(snapshot)
                                                .questions.iter_mut().find(|q| q.id == question.id
                                                    && q.revision.0 < question.revision.0)
                                            {
                                                *current = question;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(local) = offline_task {
                        match &result {
                            Ok(value) => {
                                self.settings.offline_tasks.retain(|task| task.id != local.id);
                                if let (Ok(ticket), Some(snapshot)) = (serde_json::from_value::<Ticket>(value.clone()), &mut self.snapshot) {
                                    let snapshot = std::sync::Arc::make_mut(snapshot);
                                    if !snapshot.tickets.iter().any(|t| t.id == ticket.id) { snapshot.tickets.push(ticket); }
                                }
                                task_notice = Some(crate::tr!("Offline-Aufgabe als Server-Entwurf übertragen."));
                            }
                            Err(error) => {
                                if let Some(task) = self.settings.offline_tasks.iter_mut().find(|t| t.id == local.id) {
                                    task.sync_error = Some(error.message.clone());
                                    if !error.uncertain { task.sync_request = None; }
                                }
                                self.action_errors.push((crate::overview::now_ms(), crate::tr!("Offline-Aufgabe ({}): {}", local.project_name, error.message)));
                                self.notice(crate::tr!("Offline-Aufgabe konnte noch nicht übertragen werden. Sie bleibt lokal erhalten."));
                            }
                        }
                        self.settings_dirty = true;
                    }
                    if let Some(message) = task_notice {
                        self.notice(message);
                    }
                    if result.is_ok() || result.as_ref().is_err_and(|e| !e.uncertain) {
                        self.settings.pending.retain(|r| r.id != id);
                        self.settings_dirty = true;
                    }
                    self.responses.insert(id, result);
                }
                Event::Archive(id, request, mode, result) => {
                    for tab in &mut self.tabs {
                        if tab.loading.get(&id) != Some(&request) {
                            continue;
                        }
                        tab.loading.remove(&id);
                        match &result {
                            Ok(p) => {
                                let after = if let ArchiveRead::Newer(v) = mode {
                                    v
                                } else {
                                    0
                                };
                                if p.next_after < after
                                    || p.entries
                                        .iter()
                                        .any(|e| e.sequence <= after || e.sample_revision == 0 || (tab.show_versions && matches!(mode, ArchiveRead::Older(before) if e.sequence >= before)))
                                {
                                    tab.error = Some(crate::tr!("Ungültige Archivseite; bisheriger Verlauf bleibt erhalten.").into());
                                    continue;
                                }
                                tab.error = None;
                                tab.initialized.insert(id);
                                if !matches!(mode, ArchiveRead::Older(_)) {
                                    tab.cursors.insert(id, p.next_after);
                                }
                                if !matches!(mode, ArchiveRead::Newer(_)) {
                                    tab.older.insert(id, p.next_before);
                                }
                                if matches!(mode, ArchiveRead::Older(_)) {
                                    tab.scroll_to_start = true;
                                }
                                if let Some(status) = &p.status {
                                    tab.statuses.insert(id, status.clone());
                                }
                                let values = tab.archives.entry(id).or_default();
                                for entry in &p.entries {
                                    let message_id =
                                        entry.message["id"].as_str().unwrap_or("").to_string();
                                    tab.message_positions
                                        .entry(message_id.clone())
                                        .and_modify(|v| *v = (*v).min(entry.sequence))
                                        .or_insert(entry.sequence);
                                    if !tab.show_versions {
                                        if values.iter().any(|old| {
                                            old.message["id"] == message_id
                                                && old.sequence >= entry.sequence
                                        }) {
                                            continue;
                                        }
                                        values.retain(|old| old.message["id"] != message_id);
                                    }
                                    if !values.iter().any(|v| v.sequence == entry.sequence) {
                                        values.push(std::sync::Arc::new(entry.clone()));
                                    }
                                }
                                if p.more_newer {
                                    if p.latest_sequence.saturating_sub(p.next_after) > 128 {
                                        tab.initialized.remove(&id);
                                    }
                                    tab.next_poll = Instant::now();
                                }
                            }
                            Err(e) => tab.error = Some(e.clone()),
                        }
                    }
                }
                Event::Attachment(target, result) => match result {
                    Ok(attachment) => {
                        let dialog_task = match &mut self.dialog {
                            Some(Dialog::Task(task)) if !task.read_only && task.id == target && task.request.is_none() => Some(task.as_mut()),
                            _ => None,
                        };
                        let task = dialog_task.or_else(|| self.session_prompts.values_mut()
                            .find(|task| task.id == target && task.request.is_none()).map(|task| task.as_mut()));
                        if let Some(task) = task {
                            match attachment {
                                InputAttachment::Image(v) => task.draft.images.push(v),
                                InputAttachment::File(v) => task.draft.files.push(v),
                            }
                            task.error = validate_images(&task.draft.images)
                                .and_then(|()| validate_files(&task.draft.files))
                                .err()
                                .map(str::to_string);
                            task.dirty = true;
                        } else {
                            self.notice(crate::tr!("Anhang gehört zu einem anderen oder bereits gesendeten Entwurf und wurde nicht übernommen."));
                        }
                    }
                    Err(e) => self.notice(e),
                },
                Event::TaskHistory(dialog, before, result) => {
                    if let Some(Dialog::Task(task)) = &mut self.dialog {
                        if task.id == dialog {
                            task.receive_history(before, result);
                        }
                    }
                }
                Event::TaskPayload(id, revision, request, result) => {
                    if self.task_loading.get(&id) != Some(&request) {
                        continue;
                    }
                    self.task_loading.remove(&id);
                    match result {
                        Ok(ticket)
                            if ticket.revision == revision
                                && self.snapshot.as_ref().is_some_and(|s| {
                                    s.tickets
                                        .iter()
                                        .any(|t| t.id == id && t.revision == revision)
                                }) =>
                        {
                            self.task_errors.remove(&id);
                            if self.task_payloads.len() >= 24 {
                                if let Some(old) =
                                    self.task_payloads.keys().find(|key| **key != id).copied()
                                {
                                    self.task_payloads.remove(&old);
                                }
                            }
                            self.task_payloads.insert(id, std::sync::Arc::new(ticket));
                        }
                        Ok(_) => {
                            self.task_errors.insert(id, (revision, crate::tr!("Aufgabe während des Ladens geändert. Aktuellen Stand prüfen und erneut laden.").into()));
                        }
                        Err(error) => {
                            self.task_errors.insert(id, (revision, error));
                        }
                    }
                }
                Event::Preview(key, result) => {
                    for cache in self.lists.values_mut() {
                        cache.list.reset();
                    }
                    self.previews.insert(
                        key.clone(),
                        Some(result.map(|pixels| {
                            self.backend.context().load_texture(
                                key,
                                pixels,
                                egui::TextureOptions::LINEAR,
                            )
                        })),
                    );
                }
                Event::Saved(result) => match result {
                    Ok(v) => self.notice(crate::tr!("Gespeichert: {v}", v = v)),
                    Err(e) => self.notice(e),
                },
                Event::OpenNotification(id, target) => self.open_notification(id, target),
                Event::LocalRunners(report) => {
                    self.local_runners_busy = false;
                    self.restarting_runner = None;
                    if let Some(runners) = report.runners { self.local_runners = runners; }
                    match report.result {
                        Ok(Some(message)) => self.notice(message),
                        Ok(None) => {},
                        Err(error) => {
                            self.action_errors.push((crate::overview::now_ms(), error));
                            if self.action_errors.len() > 50 {
                                self.action_errors.remove(0);
                            }
                            self.notice(crate::tr!("Lokale Runner-Aktion fehlgeschlagen. Details stehen in der Diagnose."));
                        }
                    }
                },
                Event::Local(result) => match result {
                    Ok(c) => {
                        if let Err(e) = self.connect(c.server, c.token) {
                            self.notice(e);
                        } else {
                            self.settings.credential_file = Some(c.credential_file);
                            self.settings_dirty = true;
                            self.dialog = None;
                        }
                    }
                    Err(e) => {
                        if let Some(Dialog::LocalFactory { starting }) = &mut self.dialog {
                            *starting = false;
                        }
                        self.notice(e);
                    }
                },
                Event::Access(result) => {
                    self.access = result.ok();
                }
                Event::History(result) => {
                    self.events_loading = false;
                    match result {
                        Ok(p) => {
                            self.event_before = p.next_before;
                            for event in p.events {
                                self.event_cursor = self.event_cursor.max(event.sequence);
                                if !self.events.iter().any(|v| v.sequence == event.sequence) {
                                    self.events.push(event);
                                }
                            }
                            self.events.sort_by_key(|v| v.sequence);
                        }
                        Err(e) => self.notice(e),
                    }
                }
            }
        }
    }
    fn poll_tabs(&mut self) {
        if !self.online {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        // Only poll the visible tab. Other tabs retain their state and refresh on activation.
        if let Some(tab) = self
            .active_tab
            .checked_sub(1)
            .and_then(|i| self.tabs.get_mut(i))
        {
            if Instant::now() < tab.next_poll {
                return;
            }
            tab.next_poll = Instant::now() + Duration::from_secs(2);
            let c = viewed_execution(snapshot, tab.root, tab.selected_execution);
            if let Some(c) = c {
                if tab.archives.len() >= 8 && !tab.archives.contains_key(&c.id) {
                    if let Some(old) = tab.archives.keys().find(|id| **id != c.id).copied() {
                        tab.archives.remove(&old);
                        tab.cursors.remove(&old);
                        tab.older.remove(&old);
                        tab.statuses.remove(&old);
                        tab.loading.remove(&old);
                        tab.initialized.remove(&old);
                    }
                }
                if let std::collections::btree_map::Entry::Vacant(entry) = tab.loading.entry(c.id) {
                    let request = self.next_request;
                    self.next_request += 1;
                    entry.insert(request);
                    self.backend.archive(
                        c.id,
                        if tab.initialized.contains(&c.id) {
                            ArchiveRead::Newer(tab.cursors.get(&c.id).copied().unwrap_or(0))
                        } else {
                            ArchiveRead::Recent
                        },
                        request,
                        tab.show_versions,
                    );
                }
            }
        }
    }
}
impl App {
    pub fn on_exit(&mut self) {
        if let Some((language, ack)) = self.language_save.take() {
            if ack.recv_timeout(Duration::from_secs(5)).unwrap_or(false) {
                self.settings.language = language;
            }
        }
        for (_, composer) in std::mem::take(&mut self.session_prompts) { composer.persist(self); }
        if let Some(settings) = self.factory_settings.take() {
            settings.persist(self);
        }
        if let Some(dialog) = self.dialog.take() {
            dialog.persist_drafts(self);
        }
        if !self.backend.flush(self.settings.clone()) {
            eprintln!("{}", crate::tr!("YoloForge: letzte Desktop-Einstellungen konnten nicht bestätigt gespeichert werden.", ));
        }
    }
    pub fn clipboard_image_target(&self) -> Option<RequestId> {
        match &self.dialog {
            Some(Dialog::Task(task)) if !task.read_only && task.request.is_none() => Some(task.id),
            Some(_) => None,
            None => self.active_tab.checked_sub(1).and_then(|index| self.tabs.get(index))
                .filter(|tab| self.snapshot.as_ref().is_some_and(|s|
                    can_prompt_session(s, tab.project, tab.root)))
                .and_then(|tab| self.session_prompts.get(&tab.root))
                .filter(|composer| composer.request.is_none()).map(|composer| composer.id),
        }
    }

    pub(crate) fn poll_language_save(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let result = self.language_save.as_ref().and_then(|(_, rx)| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Disconnected) => Some(false),
            Err(TryRecvError::Empty) => None,
        });
        if let Some(saved) = result {
            let (language, _) = self.language_save.take().unwrap();
            if saved {
                self.settings.language = language;
                crate::i18n::set_language(language);
                // Persist the latest drafts too, in case another write was queued while saving.
                self.settings_dirty = true;
                self.notice(crate::tr!("Language saved."));
            } else {
                self.notice(crate::tr!("Could not save language. Please try again."));
            }
        }
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        self.poll_language_save();
        crate::i18n::set_language(self.settings.language);
        // Tiling compositors can ignore the requested minimum size. Keep controls
        // reachable even when native DPI scaling leaves fewer than 820 points.
        let native_size = ctx.content_rect().size() * ctx.zoom_factor();
        let zoom = (native_size.x / 820.0)
            .min(native_size.y / 560.0)
            .clamp(0.5, 1.0);
        if (ctx.zoom_factor() - zoom).abs() > 0.01 {
            ctx.set_zoom_factor(zoom);
        }
        self.receive();
        crate::model_settings::tick(self);
        if let Some(snapshot) = &self.snapshot {
            for tab in &mut self.tabs {
                if let Some(command) = snapshot
                    .commands
                    .iter()
                    .find(|c| c.session_root() == tab.root)
                {
                    tab.title = session_title(snapshot, command);
                }
            }
        }
        if self.online && self
            .updated
            .is_some_and(|t| t.elapsed() > Duration::from_secs(10))
        {
            self.connection_lost(crate::tr!("Seit mehr als 10 Sekunden kein Factory-Stand empfangen.").into());
        }
        self.poll_tabs();
        if self.page() == Page::Notifications
            && self.active_tab == 0
            && self.online
            && !self.notification_states_loading
            && Instant::now() >= self.next_notification_states
        {
            self.notification_states_loading = true;
            self.next_notification_states = Instant::now() + Duration::from_secs(3);
            self.backend
                .notification_states(self.notifications.iter().map(|n| n.id).collect());
        }
        if self.page() == Page::Diagnostics
            && self.active_tab == 0
            && self.online
            && !self.events_loading
            && Instant::now() >= self.next_events_poll
        {
            self.events_loading = true;
            self.next_events_poll = Instant::now() + Duration::from_secs(2);
            if self.events.is_empty() {
                self.backend.history(None);
            } else {
                self.backend.events(self.event_cursor);
            }
        }
        self.draw_shell(ctx);
        self.handle_file_drop(ctx);
        if let Some(mut dialog) = self.dialog.take() {
            let close = dialog.show(ctx, self);
            if !close && self.dialog.is_none() {
                self.dialog = Some(dialog);
            }
        }
        crate::design::file_drop_overlay(
            ctx,
            self.clipboard_image_target().is_some(),
        );
        if self.settings_dirty && Instant::now() >= self.next_save {
            self.backend.persist(self.settings.clone());
            self.settings_dirty = false;
            self.next_save = Instant::now() + Duration::from_millis(500);
        }
        self.notices
            .retain(|(_, t)| t.elapsed() < Duration::from_secs(12));
        if !self.notices.is_empty() {
            egui::Area::new("notifications-toast".into())
                .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    for (text, _) in self.notices.iter().rev().take(3) {
                        egui::Frame::popup(ui.style())
                            .inner_margin(14)
                            .show(ui, |ui| {
                                ui.set_max_width(400.0);
                                ui.label(text);
                            });
                    }
                });
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        if self.screenshot.is_some()
            && !self.screenshot_requested
            && self.started.elapsed() > Duration::from_secs(3)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = self.screenshot.take() {
                    self.backend.save_screenshot(path, image);
                }
            }
        }
        if self
            .exit_after_ms
            .is_some_and(|ms| self.started.elapsed() > Duration::from_millis(ms))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "factory-bold".into(),
        egui::FontData::from_static(include_bytes!("../assets/DejaVuSans-Bold.ttf")).into(),
    );
    fonts.families.insert(
        egui::FontFamily::Name("factory-bold".into()),
        vec!["factory-bold".into()],
    );
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("phosphor".into());
    ctx.set_fonts(fonts);
    ctx.set_visuals(egui::Visuals::dark());
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
    style.spacing.interact_size.y = 36.0;
    style.spacing.icon_spacing = 12.0;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.visuals.panel_fill = Color32::from_rgb(22, 25, 31);
    style.visuals.window_fill = Color32::from_rgb(30, 34, 42);
    style.visuals.extreme_bg_color = Color32::from_rgb(18, 21, 27);
    style.visuals.faint_bg_color = Color32::from_rgb(32, 37, 46);
    style.visuals.selection.bg_fill = Color32::from_rgb(35, 86, 142);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(42, 48, 59);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(42, 48, 59);
    style.visuals.widgets.hovered.bg_fill = crate::design::HOVER_FILL;
    style.visuals.widgets.hovered.weak_bg_fill = crate::design::HOVER_FILL;
    style.visuals.widgets.active.bg_fill = crate::design::SELECTED_FILL;
    style.visuals.widgets.active.weak_bg_fill = crate::design::SELECTED_FILL;
    style.visuals.widgets.open.bg_fill = crate::design::HOVER_FILL;
    style.visuals.widgets.open.weak_bg_fill = crate::design::HOVER_FILL;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_stroke = egui::Stroke::NONE;
        widget.expansion = 0.0;
    }
    let divider = egui::Stroke::new(1.0_f32,crate::design::SEPARATOR_COLOR);
    style.visuals.widgets.noninteractive.bg_stroke = divider;
    style.visuals.window_stroke = divider;
    style.visuals.override_text_color = Some(Color32::from_rgb(231, 234, 240));
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(27.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(13.0));
    ctx.set_style(style);
    crate::design::install_interact_cursor(ctx);
}

pub fn badge(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) {
    crate::design::badge_frame(color.gamma_multiply(0.15)).show(ui, |ui| {
        ui.label(RichText::new(text.into()).color(color).strong());
    });
}
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgb(29, 33, 41))
        .stroke(egui::Stroke::new(1.0_f32, crate::design::SEPARATOR_COLOR))
        .corner_radius(10)
        .inner_margin(20)
        .outer_margin(egui::Margin::symmetric(0, 6))
}
