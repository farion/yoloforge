use crate::design;
use crate::{
    app::{self, App, AMBER, BLUE, MUTED, RED},
    model::*,
    persistence::TaskDraft,
};
use factory_core::*;
use factory_protocol::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub enum Dialog {
    ConnectionInfo,
    Connection {
        server: String,
        token: String,
        error: Option<String>,
        local_starting: bool,
    },
    Task(Box<TaskDialog>),
    TaskEditing {
        id: TicketId,
        request: u64,
    },
    Questions(QuestionDialog),
    QuestionView(Vec<Question>),
    Form(FormDialog),
    ProjectSettings(Box<crate::project_settings::ProjectSettings>),
    FactorySettings,
    ModelSetupRequired,
    ModelDefaultsConfigured { model: String },
    Read {
        title: String,
        request: u64,
        path: String,
    },
    Image {
        image: PromptImage,
        key: RequestId,
    },
    Pending,
    Settings,
    Enroll {
        server: String,
        code: String,
        name: String,
        sending: bool,
    },
}
pub struct TaskDialog {
    pub read_only: bool,
    pub payloads_complete: bool,
    pub id: RequestId,
    image_keys: Vec<String>,
    focused: bool,
    pub draft: TaskDraft,
    pub dirty: bool,
    pub error: Option<String>,
    pub request: Option<u64>,
    submit_action: TaskSubmitAction,
    history: TaskHistory,
    model: crate::model_settings::Load<Option<factory_protocol::models::TaskModels>>,
}
#[derive(Default)]
struct TaskHistory {
    entries: Vec<TaskHistoryEntry>,
    next_before: Option<u64>,
    cursor: Option<u64>,
    loading: bool,
    error: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskSubmitAction {
    SaveDraft,
    Queue,
    QueueAndNew,
}
pub struct QuestionDialog {
    pub questions: Vec<QuestionAnswer>,
    pub error: Option<String>,
}
pub struct QuestionAnswer {
    pub question: Question,
    values: BTreeMap<String, Value>,
    decision: String,
    request: Option<u64>,
    restored: bool,
}
pub struct FormDialog {
    pub id: RequestId,
    pub title: String,
    pub help: String,
    pub method: String,
    pub path: String,
    pub body: Value,
    pub request: Option<u64>,
    pub error: Option<String>,
    folder_picker: Option<std::sync::mpsc::Receiver<Option<String>>>,
}
impl Dialog {
    pub fn project_settings(project: ProjectId) -> Self {
        Self::ProjectSettings(Box::new(crate::project_settings::ProjectSettings::new(
            project,
        )))
    }
    pub(crate) fn persist_drafts(&self, app: &mut App) {
        match self {
            Self::Task(task) => task.persist(app),
            Self::Form(form) => form.persist(app),
            Self::ProjectSettings(settings) => settings.persist(app),
            _ => {}
        }
    }
    pub fn image(image: PromptImage) -> Self {
        Self::Image {
            image,
            key: RequestId::new(),
        }
    }
    pub fn connection(server: String) -> Self {
        Self::Connection {
            server,
            token: String::new(),
            error: None,
            local_starting: false,
        }
    }
    pub fn task(server: String, project: ProjectId, ticket: Option<&Ticket>) -> Self {
        Self::Task(Box::new(TaskDialog {
            read_only: false,
            payloads_complete: true,
            id: RequestId::new(),
            image_keys: vec![],
            focused: false,
            draft: TaskDraft {
                edit_token: None,
                server,
                project,
                ticket: ticket.map(|t| t.id),
                revision: ticket.map_or(Revision(0), |t| t.revision),
                prompt: ticket.map_or_else(String::new, |t| {
                    t.prompt.clone().unwrap_or_else(|| t.description.clone())
                }),
                images: ticket.map_or_else(Vec::new, |t| t.images.clone()),
                files: ticket.map_or_else(Vec::new, |t| t.files.clone()),
                requested_session: ticket.and_then(|t| t.requested_session),
                implementer: None,
                model_loaded: false,
            },
            dirty: false,
            error: None,
            request: None,
            submit_action: TaskSubmitAction::Queue,
            history: TaskHistory::default(),
            model: Default::default(),
        }))
    }
    pub fn questions(qs: Vec<Question>) -> Self {
        Self::Questions(QuestionDialog {
            questions: qs
                .into_iter()
                .map(|question| {
                    let values = question.external.payload["fields"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|f| {
                            Some((
                                f["key"].as_str()?.into(),
                                question
                                    .answer
                                    .as_ref()
                                    .and_then(|a| a.get(f["key"].as_str()?))
                                    .or(f.get("default"))
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            ))
                        })
                        .collect();
                    QuestionAnswer {
                        question,
                        values,
                        decision: String::new(),
                        request: None,
                        restored: false,
                    }
                })
                .collect(),
            error: None,
        })
    }
    pub fn task_read_only(
        server: String,
        project: ProjectId,
        ticket: &Ticket,
        complete: bool,
    ) -> Self {
        let mut dialog = Self::task(server, project, Some(ticket));
        if let Self::Task(task) = &mut dialog {
            task.read_only = true;
            task.payloads_complete = complete;
        }
        dialog
    }
    pub fn form(title: &str, help: &str, method: &str, path: String, body: Value) -> Self {
        Self::Form(FormDialog {
            id: RequestId::new(),
            title: title.into(),
            help: help.into(),
            method: method.into(),
            path,
            body,
            request: None,
            error: None,
            folder_picker: None,
        })
    }
    pub fn read(title: String, request: u64, path: String) -> Self {
        Self::Read {
            title,
            request,
            path,
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, app: &mut App) -> bool {
        if let Self::Task(task) = self {
            if !task.read_only
                && task.request.is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok))
            {
                if task.submit_action == TaskSubmitAction::QueueAndNew {
                    // A new ID isolates late attachments and gives the fresh prompt focus.
                    *self = Self::task(task.draft.server.clone(), task.draft.project, None);
                } else {
                    return true;
                }
            }
        }
        let mut close = false;
        let mut content_size = design::dialog_content_size(ctx.content_rect().size());
        if matches!(self, Self::ProjectSettings(_) | Self::FactorySettings) {
            content_size.x = (ctx.content_rect().width()
                - 2.0 * (design::DIALOG_OUTER_GAP + design::DIALOG_PADDING as f32))
                .clamp(80.0, 1100.0);
        }
        if matches!(self, Self::Form(form) if form.is_project_editor()) {
            content_size.x = content_size.x.min(520.0);
        }
        if matches!(self, Self::ModelSetupRequired | Self::ModelDefaultsConfigured { .. }) {
            content_size.x = content_size.x.min(480.0);
        }
        let width = content_size.x;
        let body_height = (content_size.y - 62.0).max(40.0);
        let height = (body_height - 160.0).max(80.0);
        let title = match &*self {
            Self::Connection { .. } => "Mit Factory verbinden".into(),
            Self::Enroll { .. } => "Gerät anmelden".into(),
            Self::Task(task) => if task.read_only {
                "Aufgabe anzeigen · schreibgeschützt"
            } else if task.draft.ticket.is_some() {
                "Aufgabe bearbeiten"
            } else {
                "Neue Aufgabe"
            }
            .into(),
            Self::TaskEditing { .. } => "Aufgabe zum Bearbeiten sichern".into(),
            Self::Questions(group) => if group.questions.iter().any(|entry| needs_answer(&entry.question)) {
                "Fragen beantworten"
            } else {
                "Fragen und Antworten"
            }.into(),
            Self::QuestionView(_) => "Fragen und Antworten".into(),
            Self::Form(form) => form.title.clone(),
            Self::ProjectSettings(settings) => if settings.project.is_some() {
                "Erweiterte Projekteinstellungen"
            } else {
                "Factory-Einstellungen"
            }
            .into(),
            Self::ConnectionInfo => "Verbindungsübersicht".into(),
            Self::FactorySettings => "Factory-Einstellungen".into(),
            Self::ModelSetupRequired => "OpenCode einrichten".into(),
            Self::ModelDefaultsConfigured { .. } => "Default-Modelle eingerichtet".into(),
            Self::Read { title, .. } => title.clone(),
            Self::Image { image, .. } => image.name.clone(),
            Self::Pending => "Offene Zustellungen".into(),
            Self::Settings => "Desktop-Einstellungen".into(),
        };
        let title=if let Self::Task(task)=&*self {
            task.draft.ticket.and_then(|id|app.snapshot.as_ref()?.tickets.iter().find(|t|t.id==id))
                .filter(|ticket|!ticket.key.is_empty())
                .map_or(title.clone(),|ticket|format!("{} · {title}",ticket.key))
        } else {title};
        let mut header_close = false;
        let modal_id = match self {
            Self::Form(f) => egui::Id::new(("factory-form", f.id)),
            Self::Task(t) => egui::Id::new(("factory-task", t.id)),
            Self::Image { key, .. } => egui::Id::new(("factory-image", *key)),
            Self::Read { request, .. } => egui::Id::new(("factory-read", *request)),
            Self::ProjectSettings(settings) => {
                egui::Id::new(("factory-project-settings", settings.project))
            }
            Self::FactorySettings => egui::Id::new("factory-settings-dialog"),
            Self::ModelSetupRequired => egui::Id::new("factory-model-setup-required"),
            Self::ModelDefaultsConfigured { .. } => egui::Id::new("factory-model-defaults-configured"),
            _ => egui::Id::new("factory-dialog"),
        };
        let response = egui::Modal::new(modal_id).frame(
            egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING)
        ).show(ctx, |ui| {
                ui.set_width(width);
                // Give the auto-sized Area the full available limit, not a fixed
                // height or its previous small content height.
                 ui.set_max_height(content_size.y);
                let header_top = ui.cursor().top();
                header_close = design::dialog_header(ui, &title).clicked();
                ui.separator();
                let body_height = (content_size.y - (ui.cursor().top() - header_top)).max(40.0);
                if let Self::Task(task) = self {
                    close = task.show_layout(ui,app,body_height,modal_id);
                    return;
                }
                 if let Self::ProjectSettings(settings)=self {
                    close=settings.show(ui,app,body_height);
                    return;
                 }
                 if let Self::FactorySettings = self {
                     let mut settings = app.factory_settings.take()
                         .unwrap_or_else(crate::project_settings::ProjectSettings::factory);
                     close = settings.show(ui, app, body_height);
                     app.factory_settings = Some(settings);
                      return;
                 }
                 if let Self::ModelSetupRequired = self {
                     ui.add_space(4.0);
                     ui.label("Du bist mit einer Factory verbunden, für die noch kein Standardmodell eingerichtet ist.");
                     if app.access.as_ref().is_some_and(|access| access.role != DeviceRole::Admin) {
                         ui.label("Ein Factory-Admin muss einen Anbieter verbinden und ein Standardmodell auswählen, bevor Aufgaben eingereiht werden können.");
                     } else {
                         ui.label("Verbinde in den Einstellungen einen Anbieter. Die Factory richtet dann automatisch ein Standardmodell ein.");
                     }
                     ui.add_space(8.0);
                     ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                         let settings = design::accent_button(ui, egui::Button::new("Zu den Modell-Anbietern"), true, BLUE.gamma_multiply(0.7));
                         #[cfg(test)]
                         ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-model-setup-continue"), settings.rect));
                         if settings.clicked() {
                             crate::model_settings::open_setup(app);
                             close = true;
                         } else if ui.button("Später").clicked() {
                             close = true;
                         }
                     }));
                     return;
                 }
                 if let Self::ModelDefaultsConfigured { model } = self {
                     ui.label("Die Factory hat deine Default-Modelle automatisch eingerichtet.");
                     ui.label(format!("Standardmodell: {model}"));
                     ui.label("Orchestrator, Implementer und Supervisor verwenden dieses Modell, solange kein eigenes Rollenmodell gewählt ist.");
                     ui.add_space(8.0);
                     ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                         if design::accent_button(ui, egui::Button::new("Zu den Default-Modellen"), true, BLUE.gamma_multiply(0.7)).clicked() {
                             close = true;
                         }
                     }));
                     return;
                 }
            let _body = egui::ScrollArea::vertical().id_salt((modal_id, "body")).max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
            match self {
                Self::ConnectionInfo=>crate::connection_info::show(ui,app),
                Self::Connection { server, token, error, local_starting } => {
                    ui.label("Vorhandenen Server verwenden oder eine eigene lokale Factory starten. Laufende Aufgaben bleiben beim Schließen der GUI erhalten.");
                    ui.separator(); ui.label("Serveradresse (HTTPS oder Loopback)"); ui.add(design::singleline(server).desired_width(f32::INFINITY));
                    ui.label("Zugangstoken"); ui.add(design::singleline(token).password(true).desired_width(f32::INFINITY));
                    ui.small("Token bleiben nur im Arbeitsspeicher. Alternativ FACTORY_TOKEN_FILE beim Start verwenden.");
                    if let Some(e) = error { ui.colored_label(RED, e.as_str()); }
                    ui.horizontal(|ui| {
                        if design::accent_button(ui,egui::Button::new("Verbinden"),!*local_starting,BLUE.gamma_multiply(0.7)).clicked() {
                            match app.connect(server.clone(), token.clone()) { Ok(()) => close = true, Err(e) => *error = Some(e) }
                        }
                        if ui.add_enabled(!*local_starting, egui::Button::new("Lokale Factory starten")).clicked() { *local_starting = true; app.backend.start_local(); }
                        if *local_starting { ui.spinner(); ui.label("Lokale Dienste starten …"); if ui.button("Erneut versuchen").clicked() { *local_starting = false; } }
                    });
                    if ui.button("Stattdessen Einladungscode einlösen …").clicked() { app.dialog = Some(Self::Enroll { server: server.clone(), code: String::new(), name: "Desktop".into(), sending: false }); }
                }
                Self::Enroll { server, code, name, sending } => {
                    ui.label("Einladungscode von einem Administrator verwenden. Der neue Zugang wird in einer privaten Tokendatei gespeichert.");
                    ui.label("Server"); ui.add(design::singleline(server)); ui.label("Einladungscode"); ui.add(design::singleline(code).password(true)); ui.label("Gerätename"); ui.add(design::singleline(name));
                    if ui.add_enabled(!*sending, egui::Button::new("Einladung einlösen")).clicked() { *sending = true; app.backend.enroll(server.clone(), code.clone(), name.clone()); }
                    if *sending { ui.label("Anmeldung läuft. Bei unklarem Ausgang Einladung nicht blind wiederholen; Geräteverwaltung und private Tokendateien prüfen."); }
                    close = ui.button("Schließen").clicked();
                }
                Self::Task(_) => {},
                Self::ProjectSettings(_)=>{},
                 Self::FactorySettings=>{},
                 Self::ModelSetupRequired=>{},
                 Self::ModelDefaultsConfigured { .. }=>{},
                Self::TaskEditing {id,request} => {
                    ui.label("Die Aufgabe wird serverseitig gegen Einplanung gesichert. Erst nach Bestätigung öffnet sich der Editor mit allen Anhängen.");
                    ui.label("Beim Schließen bleibt die Aufgabe als Draft gesichert. Erst „Einreihen“ gibt sie wieder frei.");
                    request_status(ui,app,*request);
                    ui.small(format!("Aufgabe {id}"));
                    close = ui.button("Schließen").clicked();
                }
                Self::Questions(group) => close = group.show(ui, app, height),
                Self::QuestionView(questions) => question_answers(ui, questions),
                Self::Form(form) => close = form.show(ui, app, height),
                Self::Read { request, path, .. } => {
                    egui::ScrollArea::both().id_salt("read-dialog").max_height(height).show(ui, |ui| {
                        match app.responses.get(request).cloned() {
                            Some(Ok(value)) => { read_content(ui, app, &value, path); },
                            Some(Err(e)) => { ui.colored_label(RED, &e.message); },
                            None => { ui.spinner(); ui.label("Wird geladen …"); },
                        }
                    });
                    ui.horizontal(|ui| { if ui.button("Schließen").clicked() { close = true; }
                        if let Some(Ok(v)) = app.responses.get(request) { if ui.button("Exportieren …").clicked() { app.backend.save_json("yoloforge-details.json", v.clone()); } }
                    });
                }
                Self::Image { image, key } => {
                    egui::ScrollArea::both().max_height(height).show(ui, |ui| {
                        app.preview_image(ui, format!("full/{key}"), image, egui::vec2(width, height), true);
                    });
                    ui.small("Vorschau auf maximal 4096 × 4096 Pixel skaliert. Originalbytes bleiben unverändert.");
                    if ui.button("Originalbild speichern …").clicked() { app.backend.save_input_file(image.name.clone(), image.data.clone()); }
                    close = ui.button("Schließen").clicked();
                }
                Self::Pending => {
                    close=pending_requests(ui,app,height,true);
                }
                Self::Settings => {
                    close=desktop_settings(ui,app,height,true);
                }
            }
            });
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(modal_id.with("test-body-scroll"), (_body.inner_rect,_body.content_size)));
        });
        #[cfg(test)]
        if matches!(self, Self::ModelSetupRequired) {
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("test-model-setup-rect"), response.response.rect));
        }
        if header_close || response.should_close() {
            close = true;
        }
        if !close && matches!(self, Self::FactorySettings) {
            crate::model_settings::show_add_dialog(ctx, app);
        }
        // Closing always preserves task drafts, including unconfirmed requests in the journal.
        if let Self::Task(task) = self {
            if task.dirty || close {
                task.persist(app);
                task.dirty = false;
            }
        }
        if close {
            if matches!(self, Self::ModelDefaultsConfigured { .. }) {
                crate::model_settings::open_defaults(app);
            }
            if let Self::FactorySettings = self {
                if let Some(settings) = app.factory_settings.take() {
                    settings.persist(app);
                    app.factory_settings = Some(settings);
                }
            }
            if let Self::ProjectSettings(settings) = self {
                settings.persist(app);
            }
            if let Self::Form(form) = self {
                form.persist(app);
            }
        }
        close
    }
}

impl TaskDialog {
    pub(crate) fn receive_history(&mut self, before: Option<u64>, result: Result<TaskHistoryPage, String>) {
        self.history.loading = false;
        match result {
            Ok(page) => {
                self.history.error = None;
                let empty = self.history.entries.is_empty();
                let oldest = self.history.entries.last().map(|e| e.sequence);
                self.history.entries.extend(page.entries);
                self.history.entries.sort_by_key(|entry| std::cmp::Reverse(entry.sequence));
                self.history.entries.dedup_by_key(|entry| entry.sequence);
                if before.is_some() || empty {
                    self.history.next_before = page.next_before;
                } else if let (Some(oldest), Some(next_before)) = (oldest, page.next_before) {
                    // If the newest page jumped over our cached timeline, reset
                    // older pagination so there is no invisible gap.
                    if next_before > oldest {
                        self.history.entries.retain(|entry| entry.sequence >= next_before);
                        self.history.next_before = Some(next_before);
                    }
                }
            }
            Err(error) => self.history.error = Some(error),
        }
    }

    fn show_history(&mut self, ui: &mut egui::Ui, app: &App) {
        let Some(ticket) = self.draft.ticket else { return };
        if self.draft.server != app.server {
            return;
        }
        let cursor = app.snapshot.as_ref().map_or(0, |s| s.event_cursor);
        if app.online && !self.history.loading && self.history.error.is_none()
            && self.history.cursor != Some(cursor)
        {
            self.history.loading = true;
            self.history.cursor = Some(cursor);
            app.backend.task_history(self.id, ticket, None);
        }
        ui.add_space(8.0);
        ui.separator();
        ui.strong("Verlauf");
        for entry in &self.history.entries {
            ui.horizontal_top(|ui| {
                ui.colored_label(MUTED, timestamp(entry.created_unix_ms));
                ui.add(egui::Label::new(egui::RichText::new(&entry.message)
                    .color(design::history_color(entry.level, ui.visuals().text_color()))).wrap());
            });
        }
        if self.history.entries.is_empty() {
            if let Some(ticket) = app.snapshot.as_ref().and_then(|s| s.tickets.iter().find(|t| t.id == ticket))
                .filter(|t| !t.routing_note.is_empty())
            {
                ui.colored_label(
                    design::history_color(task_routing_level(&ticket.routing_note), ui.visuals().text_color()),
                    format!("{} · {}", timestamp(ticket.updated_unix_ms), ticket.routing_note),
                );
            } else if app.online && !self.history.loading && self.history.error.is_none() {
                ui.colored_label(MUTED, "Noch keine Einträge.");
            }
        }
        if !app.online {
            return;
        }
        if self.history.loading {
            ui.label("Verlauf wird geladen …");
        } else if self.history.error.is_some() {
            if ui.button("Verlauf erneut laden").clicked() {
                self.history.error = None;
                self.history.cursor = None;
            }
        } else if let Some(before) = self.history.next_before {
            if ui.button("Ältere Einträge laden").clicked() {
                self.history.loading = true;
                app.backend.task_history(self.id, ticket, Some(before));
            }
        }
    }

    fn show_read_only_body(&mut self, ui: &mut egui::Ui, app: &mut App) {
        if !self.payloads_complete {
            if let Some(ticket) = app
                .snapshot
                .as_ref()
                .and_then(|s| s.tickets.iter().find(|t| Some(t.id) == self.draft.ticket))
                .cloned()
            {
                let data = app.task_data(&ticket);
                let source = data.as_deref().unwrap_or(&ticket);
                self.draft.revision = source.revision;
                self.draft.prompt = source
                    .prompt
                    .clone()
                    .unwrap_or_else(|| source.description.clone());
                self.draft.images = source.images.clone();
                self.draft.files = source.files.clone();
                self.draft.requested_session = source.requested_session;
                self.payloads_complete = data.is_some();
            }
        }
        if !self.payloads_complete && app.online {
            if ui
                .add_enabled(
                    self.draft.ticket.is_some_and(|id| !app.task_loading.contains_key(&id)),
                    egui::Button::new("Anhänge erneut laden"),
                )
                .clicked()
            {
                if let Some(id) = self.draft.ticket {
                    app.task_errors.remove(&id);
                }
            }
        }
        if !self.draft.images.is_empty() || !self.draft.files.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (i, image) in self.draft.images.iter().enumerate() {
                    let detail = if self.payloads_complete {
                        design::attachment_details(&image.mime, &image.data)
                    } else {
                        format!(
                            "{} · vollständige Anhangdaten noch nicht verfügbar",
                            image.mime
                        )
                    };
                    design::attachment_preview_tile(ui, &image.name, &detail, |ui| {
                        if self.payloads_complete {
                            app.preview_image_surface(
                                ui,
                                format!(
                                    "task-view/{:?}/{}/{i}",
                                    self.draft.ticket, self.draft.revision.0
                                ),
                                image,
                                design::ATTACHMENT_SIZE - egui::vec2(8.0, 8.0),
                                false,
                            )
                        } else {
                            ui.label(egui::RichText::new(egui_phosphor::regular::IMAGE).size(32.0))
                        }
                    });
                }
                for file in &self.draft.files {
                    let detail = if self.payloads_complete {
                        design::attachment_details(&file.mime, &file.data)
                    } else {
                        format!(
                            "{} · vollständige Anhangdaten noch nicht verfügbar",
                            file.mime
                        )
                    };
                    design::attachment_preview_tile(ui, &file.name, &detail, |ui| {
                        if self.payloads_complete {
                            design::file_thumbnail(ui, file)
                        } else {
                            ui.label(egui::RichText::new(egui_phosphor::regular::FILE).size(32.0))
                        }
                    });
                }
            });
        }
        design::task_prompt_read_only(ui, &self.draft.prompt, 3);
        self.show_history(ui, app);
    }

    fn show_layout(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut App,
        height: f32,
        modal_id: egui::Id,
    ) -> bool {
        let footer_height = ui.spacing().interact_size.y + 2.0 * ui.spacing().item_spacing.y + 8.0;
        let _body = egui::ScrollArea::vertical()
            .id_salt((modal_id, "body"))
            .max_height((height - footer_height).max(40.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                self.show_body(ui, app);
            });
        ui.separator();
        let close = ui.horizontal(|ui| self.show_footer(ui, app)).inner;
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                modal_id.with("test-body-scroll"),
                (
                    egui::Rect::from_min_size(
                        _body.inner_rect.min,
                        egui::vec2(
                            _body.inner_rect.width(),
                            _body.inner_rect.height().min(_body.content_size.y),
                        ),
                    ),
                    _body.content_size,
                ),
            )
        });
        close
    }
    pub(crate) fn persist(&self, app: &mut App) {
        if self.read_only {
            return;
        }
        app.settings.task_drafts.retain(|d| {
            !(d.server == self.draft.server
                && d.project == self.draft.project
                && d.ticket == self.draft.ticket)
        });
        if !self.draft.prompt.is_empty()
            || !self.draft.images.is_empty()
            || !self.draft.files.is_empty()
            || self.draft.implementer.is_some()
            || (self.draft.edit_token.is_some()
                && !self
                    .request
                    .is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)))
        {
            app.settings.task_drafts.push(self.draft.clone());
        }
        app.settings_dirty = true;
    }
    fn show_body(&mut self, ui: &mut egui::Ui, app: &mut App) {
        ui.spacing_mut().item_spacing.y = 6.0;
        if self.read_only {
            if app.models.managed() {
                if let Some(ticket) = self.draft.ticket {
                    self.model.load(app, &format!("/v1/tasks/{ticket}/models"));
                    if let Some((models, catalog)) = self.model.value.as_ref().and_then(Option::as_ref)
                        .zip(app.models.factory.value.as_ref())
                    {
                        if let Some(model) = &models.implementer {
                            ui.label(format!("Implementer: {}", crate::model_settings::label(&catalog.accesses, model)));
                        }
                    }
                }
            }
            self.show_read_only_body(ui, app);
            return;
        }
        if self.draft.edit_token.is_some() && self.request.is_none() {
            if let Some(current) = app
                .snapshot
                .as_ref()
                .and_then(|s| s.tickets.iter().find(|t| Some(t.id) == self.draft.ticket))
                .filter(|t| t.revision != self.draft.revision)
            {
                ui.colored_label(AMBER,"Der Serverstand hat sich geändert. Eigene Eingaben bleiben erhalten; vor dem Speichern vergleichen.");
                ui.collapsing("Serverstand vergleichen", |ui| {
                    ui.add(
                        egui::Label::new(current.prompt.as_deref().unwrap_or(&current.description))
                            .wrap(),
                    );
                    if ui
                        .button("Verglichenen Stand als Basis übernehmen")
                        .clicked()
                    {
                        self.draft.revision = current.revision;
                        self.dirty = true;
                        self.error = None;
                    }
                });
            }
        }
        if self.request.is_none() {
            self.request = app
                .settings
                .pending
                .iter()
                .find(|r| {
                    r.server == app.server
                        && crate::model_settings::pending_task(&r.path, &r.body).is_some_and(|task|
                            task["project_id"] == json!(self.draft.project) && task["id"] == json!(self.draft.ticket))
                })
                .map(|r| r.id);
        }
        let locked = self.request.is_some();
        if self.image_keys.len() != self.draft.images.len() {
            self.image_keys = self
                .draft
                .images
                .iter()
                .map(|image| image_digest(std::slice::from_ref(image)))
                .collect();
        }
        ui.add_enabled_ui(!locked, |ui| {
            self.show_model(ui, app);
            let mut remove_image = None;
            let mut remove_file = None;
            if !self.draft.images.is_empty() || !self.draft.files.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (i, image) in self.draft.images.iter().enumerate() {
                        if design::attachment_tile(
                            ui,
                            &image.name,
                            &design::attachment_details(&image.mime, &image.data),
                            |ui| {
                                app.preview_image_surface(
                                    ui,
                                    format!("draft/{}", self.image_keys[i]),
                                    image,
                                    design::ATTACHMENT_SIZE - egui::vec2(8.0, 8.0),
                                    false,
                                )
                            },
                        ) {
                            remove_image = Some(i);
                        }
                    }
                    for (i, file) in self.draft.files.iter().enumerate() {
                        if design::attachment_tile(
                            ui,
                            &file.name,
                            &design::attachment_details(&file.mime, &file.data),
                            |ui| design::file_thumbnail(ui, file),
                        ) {
                            remove_file = Some(i);
                        }
                    }
                });
            }
            if let Some(i) = remove_image {
                self.draft.images.remove(i);
                self.image_keys.remove(i);
                self.dirty = true;
            }
            if let Some(i) = remove_file {
                self.draft.files.remove(i);
                self.dirty = true;
            }
            let prompt = design::task_prompt(ui, &mut self.draft.prompt, 3);
            if !self.focused {
                prompt.request_focus();
                self.focused = true;
            }
            if prompt.changed() {
                self.dirty = true;
            }
        });
        self.show_history(ui, app);
        if let Some(error) = &self.error {
            ui.colored_label(RED, error);
        }
        if let Some(id) = self.request {
            request_status(ui, app, id);
            if app
                .responses
                .get(&id)
                .is_some_and(|v| v.as_ref().is_err_and(|e| !e.uncertain))
                && ui.button("Entwurf korrigieren").clicked()
            {
                self.request = None;
            }
        }
    }
    fn show_model(&mut self, ui: &mut egui::Ui, app: &mut App) {
        if !app.models.managed() { return; }
        crate::model_settings::load_project(app, self.draft.project);
        if let Some(ticket) = self.draft.ticket {
            self.model.load(app, &format!("/v1/tasks/{ticket}/models"));
            if !self.draft.model_loaded {
                if let Some(value) = &self.model.value {
                    self.draft.implementer = value.as_ref().and_then(|models| models.implementer_override.clone());
                    self.draft.model_loaded = true;
                }
            }
        }
        ui.label("Implementer-Modell");
        if let Some(error) = &self.model.error {
            ui.colored_label(RED, error);
            if let Some(ticket) = self.draft.ticket {
                if ui.button("Modellauswahl erneut laden").clicked() && app.online {
                    self.model.fetch(app, &format!("/v1/tasks/{ticket}/models"));
                }
            }
        }
        let Some(catalog) = &app.models.factory.value else { return; };
        let available = app.models.ready_catalog();
        ui.add_enabled_ui(self.draft.ticket.is_none() || self.draft.model_loaded, |ui| {
            if crate::model_settings::choice(ui, ("task-implementer", self.id), &available,
                &mut self.draft.implementer, "Default")
            {
                self.dirty = true;
                self.draft.model_loaded = true;
            }
        });
        if self.draft.implementer.is_none() {
            if let Some(model) = crate::model_settings::effective_implementer(app, self.draft.project) {
                ui.colored_label(MUTED, crate::model_settings::label(&catalog.accesses, model));
            }
        }
    }

    fn submit(&mut self, app: &mut App, action: TaskSubmitAction) {
        if app.models.managed() && self.draft.ticket.is_some() && !self.draft.model_loaded {
            self.error = Some("Bitte die gespeicherte Modellauswahl abwarten.".into());
            return;
        }
        if app.models.needs_setup() && action != TaskSubmitAction::SaveDraft {
            self.error = Some("OpenCode zuerst in den Factory-Einstellungen einrichten.".into());
            return;
        }
        if action != TaskSubmitAction::SaveDraft && app.models.managed() {
            let effective = self.draft.implementer.as_ref()
                .or_else(|| crate::model_settings::effective_implementer(app, self.draft.project));
            if let Some((root, model)) = self.draft.requested_session.zip(effective) {
                if app.snapshot.as_ref().and_then(|s| s.commands.iter().find(|c| c.id == root))
                    .and_then(|c| c.agent_spec.as_ref()).is_some_and(|agent|
                        agent.profile.provider_id != model.access_id || agent.profile.model_id != model.model_id)
                {
                    self.error = Some("Die ausgewählte Session nutzt einen anderen Zugang oder ein anderes Modell. Bitte Automatisch wählen, damit eine neue Session möglich ist.".into());
                    return;
                }
            }
        }
        if self.draft.requested_session.is_some_and(|root| {
            app.snapshot.as_ref().is_none_or(|snapshot| {
                !assignable_sessions(snapshot, self.draft.project).iter().any(|c| c.session_root() == root)
            })
        }) {
            self.error = Some("Die zugeordnete Session ist nicht mehr verfügbar. Bitte eine offene Session oder Automatisch wählen.".into());
            return;
        }
        if self.draft.prompt.len() > 12000
            || (self.draft.prompt.trim().is_empty() && self.draft.images.is_empty() && self.draft.files.is_empty())
        {
            self.error = Some("Bitte Text oder Anhänge angeben; Prompt maximal 12000 UTF-8-Bytes.".into());
            return;
        }
        if let Err(error) = validate_images(&self.draft.images).and_then(|()| validate_files(&self.draft.files)) {
            self.error = Some(error.into());
            return;
        }
        self.error = None;
        self.submit_action = action;
        let body = SubmitTask {
            edit_token: self.draft.edit_token,
            task: SaveTask {
                requested_session: self.draft.requested_session,
                request_id: RequestId::new(),
                id: self.draft.ticket,
                expected_revision: self.draft.revision,
                project_id: self.draft.project,
                prompt: self.draft.prompt.clone(),
                images: self.draft.images.clone(),
            },
            draft: action == TaskSubmitAction::SaveDraft,
            files: self.draft.files.clone(),
        };
        let label = if body.draft { "Entwurf speichern" } else { "Aufgabe einreihen" };
        let (path, value) = if app.models.managed() || self.draft.implementer.is_some() {
            ("/v1/tasks/submit-with-model", json!(factory_protocol::models::SubmitTaskWithModel {
                submission: body, implementer: self.draft.implementer.clone()
            }))
        } else { ("/v1/tasks/submit", json!(body)) };
        self.request = Some(app.write(label, "POST", path.into(), value));
    }

    fn show_footer(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        if self.read_only {
            let execution = app.snapshot.as_ref().and_then(|s| {
                let ticket = s.tickets.iter().find(|t| Some(t.id) == self.draft.ticket)?;
                task_session_command(s, ticket).map(|c| c.session_root())
            });
            return ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut close = ui.button("Schließen").clicked();
                    if let Some(root) = execution {
                        let open = ui.button("OpenCode anzeigen");
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("test-task-dialog-opencode", self.id)), open.rect));
                        if open.clicked() {
                            app.open_session(self.draft.project, root);
                            close = true;
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let title = app
                            .snapshot
                            .as_ref()
                            .and_then(|s| {
                                s.tickets
                                    .iter()
                                    .find(|t| Some(t.id) == self.draft.ticket)
                                    .and_then(|t| task_command(s, t))
                                    .map(|c| session_title(s, c))
                            })
                            .unwrap_or_else(|| "Automatisch".into());
                        ui.add(egui::Label::new(format!("Zuordnung: {title}")).truncate())
                            .on_hover_text(title);
                    });
                    close
                })
                .inner;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let enabled = app.online && self.request.is_none();
            let queue = design::accent_button(ui, egui::Button::new("Einreihen"), enabled, BLUE.gamma_multiply(0.7))
                .on_hover_text("Aufgabe einreihen und Dialog schließen");
            let queue_new = ui.add_enabled(enabled, egui::Button::new("Einreihen & neu"))
                .on_hover_text("Aufgabe einreihen und eine neue Aufgabe öffnen");
            let save = design::icon_button(ui, egui_phosphor::regular::FLOPPY_DISK, "Entwurf speichern", enabled);
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new(("test-task-action", self.id, false)), queue.rect);
                data.insert_temp(egui::Id::new(("test-task-action", self.id, true)), save.rect);
                data.insert_temp(egui::Id::new(("test-task-queue-new", self.id)), queue_new.rect);
            });
            if queue.clicked() {
                self.submit(app, TaskSubmitAction::Queue);
            } else if queue_new.clicked() {
                self.submit(app, TaskSubmitAction::QueueAndNew);
            } else if save.clicked() {
                self.submit(app, TaskSubmitAction::SaveDraft);
            }
            let editable=self.request.is_none();
            let left_width=ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(left_width,36.0),egui::Layout::left_to_right(egui::Align::Center),|ui| {
            let choices=app.snapshot.as_ref().map(|s|assignable_sessions(s,self.draft.project)).unwrap_or_default();
            let selected=match self.draft.requested_session {
                None=>"Automatisch".to_string(),
                Some(root)=>choices.iter().find(|c|c.session_root()==root).and_then(|c|app.snapshot.as_ref().map(|s|session_title(s,c))).unwrap_or_else(||"Nicht verfügbar".into()),
            };
            let width=(ui.available_width()-2.0*(design::ICON_BUTTON_SIZE+ui.spacing().item_spacing.x)).clamp(80.0,176.0);
            ui.add_enabled_ui(editable,|ui| {
                let combo=egui::ComboBox::from_id_salt(("task-target",self.id)).selected_text(&selected).width(width)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .height(320.0)
                    .show_ui(ui,|ui| {
                        ui.set_width((ui.ctx().content_rect().width()-64.0).clamp(160.0,296.0));
                        ui.spacing_mut().item_spacing.y=2.0;
                        if design::session_choice(ui,"Automatisch durch Orchestrator",None,self.draft.requested_session.is_none()).clicked() {
                            self.dirty|=self.draft.requested_session.take().is_some();
                            ui.close();
                        }
                        for command in choices {
                            if let Some(snapshot)=&app.snapshot {
                                let status=session_status(snapshot,&command);
                                if design::session_choice(ui,&session_title(snapshot,&command),Some((status,crate::views::status_color(status))),self.draft.requested_session==Some(command.session_root())).on_hover_text(command.session_id()).clicked() {
                                    self.dirty|=self.draft.requested_session!=Some(command.session_root());
                                    self.draft.requested_session=Some(command.session_root());
                                    ui.close();
                                }
                            }
                        }
                    }).response.on_hover_text(format!("Zuordnung: {selected}"))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                #[cfg(test)]
                ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new(("test-task-target",self.id)),combo.rect));
                #[cfg(not(test))]
                let _=combo;
            });
            if design::icon_button(ui,egui_phosphor::regular::PAPERCLIP,"Bilder und Dateien hinzufügen",editable).clicked() {app.backend.pick_attachments(self.id);}
            if design::icon_button(ui,egui_phosphor::regular::CLIPBOARD,"Bild aus Zwischenablage einfügen",editable).clicked() {
                app.clipboard_image_request = Some(self.id);
            }
            });
        });
        false
    }
}

fn answer_text(value: &Value, schema: Option<&Value>) -> String {
    if let Some(label) = schema
        .and_then(|s| s["options"].as_array())
        .and_then(|options| options.iter().find(|option| option["value"] == *value))
        .and_then(|option| option["label"].as_str())
    {
        return label.to_string();
    }
    match value {
        Value::Null => "Noch nicht beantwortet.".into(),
        Value::String(text) => text.clone(),
        Value::Bool(value) => if *value { "Ja" } else { "Nein" }.into(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => values.iter().map(|v| answer_text(v, schema)).collect::<Vec<_>>().join("\n"),
        Value::Object(values) => values.iter()
            .map(|(key, value)| format!("{key}: {}", answer_text(value, None)))
            .collect::<Vec<_>>().join("\n"),
    }
}

fn permission_details(ui: &mut egui::Ui, question: &Question) {
    if question.external.kind != QuestionKind::Permission {
        return;
    }
    // Keep the requested operation visible without exposing the raw payload.
    for (key, label) in [
        ("action", "Aktion"), ("permission", "Berechtigung"),
        ("resource", "Ziel"), ("patterns", "Geltungsbereich"), ("command", "Befehl"),
    ] {
        if let Some(value) = question.external.payload.get(key).filter(|v| !v.is_null()) {
            read_text(ui, &format!("{label}: {}", answer_text(value, None)), false);
        }
    }
    if let Some(command) = question.external.payload["metadata"]["command"].as_str() {
        read_text(ui, &format!("Befehl: {command}"), false);
    }
}

/// Human-readable question/answer pairs only: no raw payloads or export controls.
fn question_answers(ui: &mut egui::Ui, questions: &[Question]) {
    let mut first = true;
    for question in questions {
        let title = question.external.payload["title"].as_str().unwrap_or(
            if question.external.kind == QuestionKind::Permission { "Berechtigung" } else { "Rückfrage" },
        );
        let fields: Vec<_> = question.external.payload["fields"].as_array()
            .into_iter().flatten().filter(|field| field["hidden"] != true).collect();
        let pairs: Vec<Option<&Value>> = if fields.is_empty() { vec![None] } else {
            fields.into_iter().map(Some).collect()
        };
        for field in pairs {
            if !first {
                ui.add_space(8.0);
                let _separator = ui.separator();
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    let id = egui::Id::new("test-question-pair-separators");
                    let count = d.get_temp::<usize>(id).unwrap_or(0);
                    d.insert_temp(id, count + 1);
                });
                ui.add_space(8.0);
            }
            first = false;
            design::question_heading(ui, "Frage");
            // Native forms may have a generic container title ("Questions").
            // Prefer the actual field question instead of repeating that wrapper.
            if !generic_question_title(title) {
                read_text(ui, title, false);
            }
            if let Some(description) = question.external.payload["description"].as_str() {
                read_text(ui, description, false);
            }
            permission_details(ui, question);
            if let Some(field) = field {
                if let Some(label) = field["title"].as_str().filter(|label| *label != title) {
                    read_text(ui, label, false);
                }
                if let Some(description) = field["description"].as_str() {
                    read_text(ui, description, false);
                }
            }
            ui.add_space(8.0);
            design::question_heading(ui, "Antwort");
            let answer = question.answer.as_ref().and_then(|answer| {
                field.map_or(Some(answer), |field| field["key"].as_str().and_then(|key| answer.get(key)))
            });
            let text = if question.external.kind == QuestionKind::Permission {
                match answer.and_then(|answer| answer["decision"].as_str()) {
                    Some("once") => "Einmal erlaubt".into(),
                    Some("always") => "Dauerhaft erlaubt".into(),
                    Some("reject") => "Abgelehnt".into(),
                    _ => answer.map_or_else(|| "Noch nicht beantwortet.".into(), |v| answer_text(v, field)),
                }
            } else {
                answer.map_or_else(|| "Noch nicht beantwortet.".into(), |v| answer_text(v, field))
            };
            read_text(ui, &text, false);
        }
    }
}

impl QuestionDialog {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32) -> bool {
        if self.questions.iter().all(|entry| !needs_answer(&entry.question)) {
            let questions: Vec<_> = self.questions.iter().map(|entry| entry.question.clone()).collect();
            question_answers(ui, &questions);
            return false;
        }
        for entry in &mut self.questions {
            if !entry.restored {
                if let Some(draft) = app
                    .settings
                    .question_drafts
                    .iter()
                    .find(|d| d.server == app.server && d.question == entry.question.id)
                {
                    entry.values = draft.values.clone();
                    entry.decision = draft.decision.clone();
                    if draft.revision != entry.question.revision {
                        self.error = Some("Gespeicherter Antwortentwurf stammt aus einer älteren Fragenrevision. Bitte alle Werte prüfen.".into());
                    }
                }
                entry.request = app
                    .settings
                    .pending
                    .iter()
                    .find(|r| {
                        r.server == app.server
                            && r.path == format!("/v1/questions/{}/answer", entry.question.id)
                    })
                    .map(|r| r.id);
                entry.restored = true;
            }
        }
        egui::ScrollArea::vertical()
            .id_salt("questions-dialog-scroll")
            .max_height(height)
            .show(ui, |ui| {
                for (index, entry) in self.questions.iter_mut().enumerate() {
                    if index > 0 {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }
                    if !needs_answer(&entry.question) {
                        question_answers(ui, std::slice::from_ref(&entry.question));
                        continue;
                    }
                    let mut reload = None;
                    ui.push_id(entry.question.id, |ui| {
                        ui.vertical(|ui| {
                            let q = &entry.question;
                            design::question_heading(ui, "Frage");
                            let title = q.external.payload["title"].as_str().unwrap_or(
                                if q.external.kind == QuestionKind::Permission {
                                    "Berechtigung"
                                } else {
                                    "Rückfrage"
                                },
                            );
                            if !generic_question_title(title) {
                                read_text(ui, title, false);
                            }
                            if let Some(text) = q.external.payload["description"].as_str() {
                                ui.label(text);
                            }
                            permission_details(ui, q);
                            let current = app
                                .snapshot
                                .as_ref()
                                .and_then(|s| s.questions.iter().find(|v| v.id == q.id))
                                .cloned();
                            let changed = current
                                .as_ref()
                                .is_none_or(|v| v.revision != q.revision || !needs_answer(v));
                            if changed {
                                ui.colored_label(
                                    AMBER,
                                    format!(
                                        "Serverzustand geändert: {:?}. Entwurf bleibt erhalten.",
                                        current.as_ref().map(|q| q.state)
                                    ),
                                );
                            }
                            if let Some(id) = entry.request {
                                request_status(ui, app, id);
                                if let Some(v) = current.as_ref() {
                                    ui.label(format!("Zustellung: {:?}", v.state));
                                }
                            }
                            ui.add_space(8.0);
                            design::question_heading(ui, "Antwort");
                            ui.add_enabled_ui(entry.request.is_none() && !changed, |ui| {
                                if q.external.kind == QuestionKind::Permission {
                                    ui.radio_value(
                                        &mut entry.decision,
                                        "once".into(),
                                        "Einmal erlauben",
                                    );
                                    ui.radio_value(
                                        &mut entry.decision,
                                        "always".into(),
                                        "Dauerhaft erlauben (erweiterte Freigabe)",
                                    );
                                    ui.radio_value(
                                        &mut entry.decision,
                                        "reject".into(),
                                        "Ablehnen",
                                    );
                                } else if let Some(fields) = q.external.payload["fields"].as_array()
                                {
                                    let current_values = entry.values.clone();
                                    for schema in fields {
                                        if !field_enabled(schema, &current_values)
                                            || schema["hidden"] == true
                                        {
                                            continue;
                                        }
                                        let Some(key) = schema["key"].as_str() else {
                                            ui.colored_label(RED, "Feld ohne Schlüssel");
                                            continue;
                                        };
                                        question_field(
                                            ui,
                                            schema,
                                            entry.values.entry(key.into()).or_insert(Value::Null),
                                        );
                                    }
                                } else {
                                    ui.colored_label(
                                        RED,
                                        "Dieses Frageformat kann hier nicht beantwortet werden.",
                                    );
                                }
                            });
                            let rejected = entry.request.and_then(|id| app.responses.get(&id)).is_some_and(|v| v.as_ref().is_err_and(|e| !e.uncertain));
                            if changed || rejected {
                                if let Some(current) = current.as_ref().filter(|q| needs_answer(q)) {
                                    if ui.button("Aktuelle Frage übernehmen · Eingaben behalten und prüfen").clicked() { reload = Some(current.clone()); }
                                }
                            }
                        });
                    });
                    if let Some(question) = reload { entry.question = question; entry.request = None; }
                }
            });
        if let Some(e) = &self.error {
            ui.colored_label(RED, e);
        }
        if design::accent_button(
                ui, egui::Button::new("Alle vorbereiteten Antworten senden"),
                app.online && self.questions.iter().any(|q| q.request.is_none() && needs_answer(&q.question)),
                BLUE.gamma_multiply(0.7),
            )
            .clicked()
        {
            let mut bodies = Vec::new();
            self.error = None;
            for (index, q) in self.questions.iter().enumerate().filter(|(_, q)| {
                q.request.is_none()
                    && app
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.questions.iter().find(|v| v.id == q.question.id))
                        .is_some_and(|v| v.revision == q.question.revision && needs_answer(v))
            }) {
                match q.payload(app) {
                    Ok(body) => bodies.push((index, body)),
                    Err(e) => {
                        self.error = Some(e);
                        break;
                    }
                }
            }
            if self.error.is_none() {
                for (index, body) in bodies {
                    let q = &mut self.questions[index];
                    q.request = Some(app.write(
                        "Frage beantworten",
                        "POST",
                        format!("/v1/questions/{}/answer", q.question.id),
                        json!(body),
                    ));
                }
            }
        }
        for entry in &self.questions {
            if !needs_answer(&entry.question) {
                continue;
            }
            if entry
                .request
                .and_then(|id| app.responses.get(&id))
                .is_some_and(Result::is_ok)
            {
                continue;
            }
            let draft = crate::persistence::QuestionDraft {
                server: app.server.clone(),
                question: entry.question.id,
                revision: entry.question.revision,
                values: entry.values.clone(),
                decision: entry.decision.clone(),
            };
            let old = app
                .settings
                .question_drafts
                .iter_mut()
                .find(|d| d.server == draft.server && d.question == draft.question);
            if let Some(old) = old {
                if old.values != draft.values
                    || old.decision != draft.decision
                    || old.revision != draft.revision
                {
                    *old = draft;
                    app.settings_dirty = true;
                }
            } else {
                app.settings.question_drafts.push(draft);
                app.settings_dirty = true;
            }
        }
        ui.button("Schließen · Antwortentwürfe behalten").clicked()
    }
}
impl QuestionAnswer {
    fn payload(&self, app: &App) -> Result<AnswerQuestion, String> {
        let q = &self.question;
        if app
            .snapshot
            .as_ref()
            .and_then(|s| s.questions.iter().find(|v| v.id == q.id))
            .is_none_or(|v| v.revision != q.revision || !needs_answer(v))
        {
            return Err("Frage wurde geändert oder beantwortet. Dialog neu öffnen; alte Eingaben nicht ungeprüft senden.".into());
        }
        let answer = if q.external.kind == QuestionKind::Permission {
            if self.decision.is_empty() {
                return Err("Bitte jede Berechtigung ausdrücklich entscheiden.".into());
            }
            json!({"decision": self.decision})
        } else {
            let mut answer = serde_json::Map::new();
            let fields = q.external.payload["fields"]
                .as_array()
                .ok_or("Unbekanntes Frageformat")?;
            for schema in fields {
                if !field_enabled(schema, &self.values) || schema["type"] == "external" {
                    continue;
                }
                let key = schema["key"].as_str().ok_or("Feldschlüssel fehlt")?;
                let value = self.values.get(key).unwrap_or(&Value::Null);
                if value.is_null() && schema["required"] != true {
                    continue;
                }
                validate_field(schema, value)
                    .map_err(|e| format!("{}: {e}", schema["title"].as_str().unwrap_or(key)))?;
                answer.insert(key.into(), value.clone());
            }
            if !fields.is_empty() && fields.iter().all(|s| s["type"] == "external") {
                return Err(
                    "Externes Formular: Bitte zuerst den angezeigten Link verwenden.".into(),
                );
            }
            Value::Object(answer)
        };
        if answer.to_string().len() > 32768 {
            return Err("Antwort maximal 32 KiB".into());
        }
        Ok(AnswerQuestion {
            request_id: RequestId::new(),
            expected_revision: q.revision,
            answer,
        })
    }
}

impl FormDialog {
    fn is_project_editor(&self) -> bool {
        (self.method == "POST" && self.path == "/v1/projects")
            || (self.method == "PATCH"
                && self.path.strip_prefix("/v1/projects/")
                    .is_some_and(|id| id.parse::<ProjectId>().is_ok()))
    }

    fn project_fields(&mut self, ui: &mut egui::Ui, app: &App) -> bool {
        use std::sync::mpsc::TryRecvError;

        if let Some(picker) = &self.folder_picker {
            match picker.try_recv() {
                Ok(path) => {
                    if let Some(path) = path {
                        self.body["repository"] = json!(path);
                        self.error = None;
                    }
                    self.folder_picker = None;
                }
                Err(TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
                }
                Err(TryRecvError::Disconnected) => self.folder_picker = None,
            }
        }
        let editing = self.method == "PATCH";
        let path_locked = self.path.strip_prefix("/v1/projects/")
            .and_then(|id| id.parse::<ProjectId>().ok())
            .is_some_and(|id| app.snapshot.as_ref().is_some_and(|s|
                s.streams.iter().any(|stream| stream.project_id == id)));
        let mut name = self.body["name"].as_str().unwrap_or("").to_owned();
        let mut key = self.body["key"].as_str().unwrap_or("").to_owned();
        let mut path = self.body["repository"].as_str().unwrap_or("").to_owned();
        let mut changed = false;
        ui.label("Name");
        let name_field = ui.add(design::singleline(&mut name).desired_width(f32::INFINITY));
        changed |= name_field.changed();
        ui.add_space(8.0);
        ui.label("Projektkürzel");
        let key_field = ui.add(
            design::singleline(&mut key)
                .hint_text(design::input_hint(if editing { "z. B. WOAI" } else { "Leer lassen für ein automatisches Kürzel" }))
                .desired_width(f32::INFINITY),
        );
        changed |= key_field.changed();
        ui.add_space(8.0);
        ui.label("Lokaler Pfad");
        ui.add_enabled_ui(!path_locked && self.folder_picker.is_none(), |ui| {
            ui.horizontal(|ui| {
                // TextEdit's desired width excludes its padding; reserve the button
                // from the full row width before converting to an inner width.
                let width = (ui.available_width() - design::ICON_BUTTON_SIZE
                    - ui.spacing().item_spacing.x - design::INPUT_MARGIN.sum().x).max(1.0);
                let path_field = ui.add(
                    design::singleline(&mut path)
                        .hint_text(design::input_hint("/pfad/zum/projekt"))
                        .desired_width(width),
                );
                changed |= path_field.changed();
                let folder = design::icon_button(
                    ui, egui_phosphor::regular::FOLDER_OPEN, "Projektordner auswählen", true,
                );
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(
                    egui::Id::new("test-project-field-rects"),
                    [name_field.rect, key_field.rect, path_field.rect, folder.rect],
                ));
                if folder.clicked() {
                    let (send, receive) = std::sync::mpsc::channel();
                    let ctx = ui.ctx().clone();
                    let current = std::path::PathBuf::from(path.trim());
                    std::thread::spawn(move || {
                        let mut picker = rfd::FileDialog::new().set_title("Projektordner auswählen");
                        if current.is_dir() {
                            picker = picker.set_directory(current);
                        }
                        let _ = send.send(picker.pick_folder().and_then(|path| path.into_os_string().into_string().ok()));
                        ctx.request_repaint();
                    });
                    self.folder_picker = Some(receive);
                }
            });
        });
        if path_locked {
            ui.colored_label(MUTED, "Der Pfad ist nach dem Anlegen von Arbeitsbereichen festgeschrieben.");
        } else if editing && path.is_empty() {
            ui.colored_label(MUTED, "Ohne neuen Pfad bleibt das bisherige Repository unverändert.");
        }
        ui.add_space(8.0);
        ui.label("Runner-Zuordnung");
        let mut mapping = serde_json::from_value::<Option<ProjectRunnerMapping>>(
            self.body["runner_mapping"].clone(),
        ).ok().flatten().unwrap_or(ProjectRunnerMapping::Automatic);
        let current_mapping = mapping;
        let runners = app.snapshot.as_ref().map_or(&[][..], |s| s.runners.as_slice());
        let selected = match mapping {
            ProjectRunnerMapping::Automatic => "Automatisch".into(),
            ProjectRunnerMapping::Runner(id) => runners.iter().find(|runner| runner.id == id)
                .map_or_else(|| "Runner nicht verfügbar".into(), |runner| runner.name.clone()),
        };
        let _runner_select = egui::ComboBox::from_id_salt("project-runner-mapping")
            .selected_text(selected)
            .wrap_mode(egui::TextWrapMode::Truncate)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut mapping, ProjectRunnerMapping::Automatic, "Automatisch");
                for runner in runners {
                    ui.selectable_value(
                        &mut mapping, ProjectRunnerMapping::Runner(runner.id), &runner.name,
                    );
                }
            })
            .response
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(
            egui::Id::new("test-project-runner-select"),
            _runner_select.rect,
        ));
        if mapping != current_mapping {
            self.body["runner_mapping"] = json!(mapping);
            self.error = None;
        }
        if changed {
            self.error = None;
            self.body["name"] = json!(name);
            self.body["key"] = json!(key);
            self.body["repository"] = json!(path);
        }
        !name.trim().is_empty()
            && (editing || !path.trim().is_empty())
            && self.folder_picker.is_none()
    }

    fn project_request(&self, app: &App) -> Result<Value, String> {
        let mut body = self.body.clone();
        let name = body["name"].as_str().unwrap_or("").trim().to_owned();
        let key = body["key"].as_str().unwrap_or("").trim().to_ascii_uppercase();
        let path = body["repository"].as_str().unwrap_or("").trim().to_owned();
        let existing = self.path.strip_prefix("/v1/projects/")
            .and_then(|id| id.parse::<ProjectId>().ok())
            .and_then(|id| app.snapshot.as_ref()?.projects.iter().find(|project| project.id == id));
        if existing.is_some_and(|project| body["runner_mapping"] == json!(project.runner_mapping)) {
            body.as_object_mut().unwrap().remove("runner_mapping");
        }
        body["name"] = json!(name);
        if !key.is_empty()
            && (key.len() > 10 || !key.starts_with(|c: char| c.is_ascii_uppercase())
                || !key.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
        {
            return Err("Das Projektkürzel muss mit einem Buchstaben beginnen und darf höchstens 10 Buchstaben oder Ziffern enthalten.".into());
        }
        if key.is_empty() && existing.is_some_and(|p| !p.key.is_empty()) {
            return Err("Das Projektkürzel darf nicht leer sein.".into());
        }
        body["key"] = json!((!key.is_empty()).then_some(key));
        if self.method == "PATCH"
            && (path.is_empty() || existing.is_some_and(|p| p.repository == path))
        {
            body["repository"] = Value::Null;
            body["base_branch"] = Value::Null;
        } else {
            body["base_branch"] = json!(local_project_branch(&path)?);
            body["repository"] = json!(path);
        }
        Ok(body)
    }

    pub(crate) fn persist(&self, app: &mut App) {
        app.settings.form_drafts.retain(|d| {
            d.server != app.server
                || d.path != self.path
                || d.method != self.method
                || d.body.get("project_id") != self.body.get("project_id")
        });
        if self
            .request
            .and_then(|id| app.responses.get(&id))
            .is_none_or(|r| r.is_err())
        {
            app.settings
                .form_drafts
                .push(crate::persistence::FormDraft {
                    server: app.server.clone(),
                    title: self.title.clone(),
                    help: self.help.clone(),
                    method: self.method.clone(),
                    path: self.path.clone(),
                    body: self.body.clone(),
                });
        }
        app.settings_dirty = true;
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32) -> bool {
        self.show_form(ui, app, height, None)
    }

    pub(crate) fn show_in_settings(&mut self, ui: &mut egui::Ui, app: &mut App, bottom: f32, height: f32) -> bool {
        let saves_settings = self.path == "/v1/projects/setup"
            || self.path.starts_with("/v1/projects/") && ["/profile", "/scheduler", "/session-limit", "/supervisor", "/token-budget"]
                .iter().any(|suffix| self.path.ends_with(suffix));
        self.show_form(ui, app, height, saves_settings.then_some(bottom))
    }

    fn show_form(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32, settings_bottom: Option<f32>) -> bool {
        if self.request.is_none() {
            if let Some(pending) = app.settings.pending.iter().find(|r| {
                r.server == app.server
                    && r.path == self.path
                    && r.method == self.method
                    && r.body.get("project_id") == self.body.get("project_id")
            }) {
                self.request = Some(pending.id);
                if self.is_project_editor() {
                    for key in ["request_id", "expected_revision", "name", "key", "repository", "runner_mapping"] {
                        if let Some(value) = pending.body.get(key).filter(|value| !value.is_null()) {
                            self.body[key] = value.clone();
                        }
                    }
                } else {
                    self.body = pending.body.clone();
                }
            }
        }
        if self.path == "/v1/projects/setup" && self.request.is_none() {
            self.body["setup"]["repository_source"] = self.body["repository"].clone();
        }
        let project_editor = self.is_project_editor();
        if !self.help.is_empty() {
            ui.label(&self.help);
        }
        let mut valid = true;
        let deleting_project = self.method == "DELETE" && self.body.get("confirm_name").is_some();
        if project_editor {
            ui.add_enabled_ui(self.request.is_none(), |ui| {
                valid = self.project_fields(ui, app);
            });
            ui.add_space(12.0);
        } else {
            egui::ScrollArea::vertical()
                .id_salt("management-dialog-scroll")
                .max_height(height)
                .show(ui, |ui| {
                    ui.add_enabled_ui(self.request.is_none(), |ui| {
                        valid = management_editor(ui, &mut self.body, "body", app);
                    });
                    ui.collapsing(
                        "Vollständige Anfragedaten (einschließlich Request-ID)",
                        |ui| json_read(ui, &self.body),
                    );
                });
        }
        if deleting_project {
            let expected = app
                .snapshot
                .as_ref()
                .and_then(|s| {
                    s.projects
                        .iter()
                        .find(|p| self.path == format!("/v1/projects/{}", p.id))
                })
                .map(|p| p.name.as_str());
            valid &= expected.is_some_and(|name| self.body["confirm_name"].as_str() == Some(name));
        }
        if let Some(e) = &self.error {
            ui.colored_label(RED, e);
        }
        if let Some(id) = self.request {
            request_status(ui, app, id);
            if let Some(Ok(result)) = app.responses.get(&id) {
                if project_editor {
                    if self.method == "POST" {
                        if let Ok(project) = serde_json::from_value::<Project>(result.clone()) {
                            app.set_project(Some(project.id));
                        }
                    }
                    return true;
                }
                if result.get("success") == Some(&Value::Bool(false)) {
                    ui.colored_label(RED, "Runner-Helfer ist fehlgeschlagen. Ausgabe und Exitcode im Ergebnis prüfen.");
                }
                ui.collapsing("Bestätigtes Ergebnis", |ui| json_read(ui, result));
            }
            if app
                .responses
                .get(&id)
                .is_some_and(|v| v.as_ref().is_err_and(|e| !e.uncertain))
                && ui.button("Eingaben korrigieren").clicked()
            {
                self.request = None;
                if self.body.get("request_id").is_some() {
                    self.body["request_id"] = json!(RequestId::new());
                }
            }
            if app
                .responses
                .get(&id)
                .is_some_and(|r| r.as_ref().is_err_and(|e| !e.uncertain))
            {
                if let Some((revision, current)) = current_revision(app, &self.path, &self.body) {
                    ui.collapsing("Aktuellen Serverstand mit meinem Entwurf vergleichen", |ui| {
                        if project_editor {
                            for (label, key) in [("Name", "name"), ("Projektkürzel", "key"), ("Lokaler Pfad", "repository")] {
                                let value = current[key].as_str().unwrap_or("");
                                if key != "repository" || value.starts_with('/') {
                                    ui.label(format!("{label}: {value}"));
                                }
                            }
                        } else {
                            json_read(ui, &current);
                        }
                    });
                    ui.small("Revision erst nach Vergleich übernehmen. Eigene Eingaben bleiben erhalten und werden nicht stillschweigend mit fremden Änderungen zusammengeführt.");
                    if ui
                        .button(format!(
                            "Serverrevision {revision} übernehmen und Entwurf erneut prüfen"
                        ))
                        .clicked()
                    {
                        let key = if self.body.get("expected_stream_revision").is_some() {
                            "expected_stream_revision"
                        } else {
                            "expected_revision"
                        };
                        self.body[key] = json!(revision);
                        self.body["request_id"] = json!(RequestId::new());
                        self.request = None;
                    }
                }
            }
        }
        let submit = if let Some(bottom) = settings_bottom {
            design::settings_footer(ui, bottom, |ui|
                design::settings_save_button(ui, self.request.is_none() && app.online && valid).clicked())
        } else if self.request.is_none() {
            design::accent_button(
                ui, egui::Button::new(if deleting_project {
                    "Projekt löschen"
                } else if project_editor && self.method == "POST" {
                    "Projekt anlegen"
                } else if project_editor {
                    "Speichern"
                } else {
                    "Bestätigen und ausführen"
                }),
                app.online && valid,
                if deleting_project {
                    RED.gamma_multiply(0.7)
                } else {
                    BLUE.gamma_multiply(0.7)
                },
            )
            .clicked()
        } else {
            false
        };
        if submit {
            let mut body = if project_editor {
                match self.project_request(app) {
                    Ok(body) => body,
                    Err(error) => {
                        self.error = Some(error);
                        return false;
                    }
                }
            } else {
                self.body.clone()
            };
            let mut path = self.path.clone();
            if let Some(device) = body.get("device_id").and_then(Value::as_str) {
                if device.parse::<DeviceId>().is_err() {
                    self.error = Some("Bitte eine gültige Geräte-ID auswählen.".into());
                    return false;
                }
                path = path.replace("{device_id}", device);
                body.as_object_mut().unwrap().remove("device_id");
            }
            self.request = Some(app.write(&self.title, &self.method, path, body));
        }
        settings_bottom.is_none() && !project_editor && ui.button("Schließen").clicked()
    }
}

/// Read only: preparing or creating a missing repository remains a runner job.
fn local_project_branch(path: &str) -> Result<String, String> {
    if path.len() > 2048 || path.chars().any(char::is_control) {
        return Err("Der lokale Pfad ist zu lang oder enthält ungültige Zeichen.".into());
    }
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err("Bitte einen absoluten lokalen Pfad angeben.".into());
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok("main".into()),
        Ok(_) if path.is_dir() => {}
        _ => return Err("Der lokale Pfad muss ein Ordner sein.".into()),
    }
    let run_git = |args: &[&str]| {
        let mut command = std::process::Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        if let Some(parent) = path.parent() {
            command.env("GIT_CEILING_DIRECTORIES", parent);
        }
        command.arg("-C").arg(path).args(args).output().ok()
            .filter(|output| output.status.success())
    };
    let branch = run_git(&["symbolic-ref", "--quiet", "HEAD"])
        .and_then(|output| String::from_utf8(output.stdout).ok());
    if let Some(branch) = branch.as_deref()
        .and_then(|branch| branch.trim_end().strip_prefix("refs/heads/"))
        .filter(|_| run_git(&["rev-parse", "--verify", "HEAD^{commit}"]).is_some())
    {
        Ok(branch.into())
    } else {
        Err("Bitte ein Git-Repository mit ausgechecktem Branch und mindestens einem Commit oder einen noch nicht vorhandenen Ordner angeben.".into())
    }
}

pub(crate) fn desktop_settings(
    ui: &mut egui::Ui,
    app: &mut App,
    height: f32,
    show_close: bool,
) -> bool {
    if ui
        .checkbox(
            &mut app.settings.desktop_notifications,
            "Desktop-Meldungen bei Fragen, Fehlern und Abschlüssen",
        )
        .changed()
    {
        app.settings_dirty = true;
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Desktop-Meldungen werden nur bei laufender GUI ausgeliefert. Meldungen enthalten Projekt und Ereignis, keine Prompts oder Antworten.").color(MUTED));
    ui.add_space(8.0);
    ui.label("Darstellung: Dark Theme · Sprache: Deutsch");
    if !app.settings.form_drafts.is_empty() {
        ui.separator();
        ui.strong("Gespeicherte Dialogentwürfe");
        egui::ScrollArea::vertical()
            .id_salt("settings-drafts")
            .max_height((height - 140.0).max(40.0))
            .show(ui, |ui| {
                for draft in app
                    .settings
                    .form_drafts
                    .clone()
                    .into_iter()
                    .filter(|d| d.server == app.server)
                {
                    if design::navigation_row(ui, &draft.title, false, None)
                        .on_hover_text("Dialogentwurf öffnen")
                        .clicked()
                    {
                        app.dialog = Some(Dialog::form(
                            &draft.title,
                            &draft.help,
                            &draft.method,
                            draft.path,
                            draft.body,
                        ));
                    }
                }
            });
    }
    show_close && ui.button("Schließen").clicked()
}

pub(crate) fn pending_requests(
    ui: &mut egui::Ui,
    app: &mut App,
    height: f32,
    show_close: bool,
) -> bool {
    ui.label("Diese Anfragen wurden noch nicht eindeutig bestätigt. Ein Abgleich wiederholt nur die identische gespeicherte Anfrage — keine neue Aufgabe.");
    egui::ScrollArea::vertical().id_salt("settings-pending").max_height((height-100.0).max(40.0)).show(ui,|ui| {
        for request in app.settings.pending.clone() {
            ui.push_id(request.id,|ui| {app::card().show(ui,|ui| {
                ui.strong(&request.label);ui.label(&request.server);ui.monospace(&request.path);
                ui.collapsing("Unveränderte gespeicherte Anfrage",|ui|json_read(ui,&request.body));
                if request.server!=app.server {ui.label("Zum zugehörigen Server verbinden, um abzugleichen.");} else {request_status(ui,app,request.id);}
                if request.path=="/v1/devices/enrollment" && !app.in_flight.contains(&request.id) && ui.button("Hinweis verwerfen · unbekannte Einladung bleibt bis Ablauf gültig").clicked() {app.settings.pending.retain(|p|p.id!=request.id);app.settings_dirty=true;}
            });});
        }
    });
    show_close && ui.button("Schließen").clicked()
}

pub fn request_status(ui: &mut egui::Ui, app: &mut App, id: u64) {
    let replayable = app
        .settings
        .pending
        .iter()
        .find(|r| r.id == id)
        .is_none_or(|r| r.path != "/v1/devices/enrollment");
    if !replayable {
        ui.colored_label(AMBER, "Einladungserstellung wird bei unklarem Ausgang nicht wiederholt. Unbekannter Code läuft gemäß Gültigkeit ab; gegebenenfalls Geräteverwaltung prüfen.");
    }
    if app.in_flight.contains(&id) {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Wird übertragen …");
        });
    } else {
        match app.responses.get(&id) {
            Some(Ok(_)) => {
                ui.colored_label(
                    app::GREEN,
                    "Anfrage bestätigt — Ergebnis und Zustellstatus prüfen.",
                );
            }
            Some(Err(e)) => {
                ui.colored_label(if e.uncertain { AMBER } else { RED }, &e.message);
                if e.uncertain {
                    ui.label("Ausgang unklar. Eingaben bleiben gesperrt; Abgleich verwendet dieselbe Request-ID und Nutzlast.");
                }
            }
            None => {
                ui.colored_label(
                    AMBER,
                    "Bestätigung fehlt. Gespeicherte Anfrage kann abgeglichen werden.",
                );
            }
        }
    }
    if replayable
        && app.settings.pending.iter().any(|r| r.id == id)
        && ui
            .add_enabled(
                app.online && !app.in_flight.contains(&id),
                egui::Button::new("Identische Anfrage abgleichen"),
            )
            .clicked()
    {
        app.retry(id);
    }
}

pub fn json_read(ui: &mut egui::Ui, value: &Value) {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    read_text(ui, &text, true);
}
pub fn read_text(ui: &mut egui::Ui, text: &str, monospace: bool) {
    let pages = text.len().div_ceil(16 * 1024).max(1);
    let id = ui.make_persistent_id(("text-page", monospace));
    let mut page = ui
        .data_mut(|d| d.get_temp::<usize>(id))
        .unwrap_or(0)
        .min(pages - 1);
    if pages > 1 {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(page > 0, egui::Button::new("‹ Vorheriger Textabschnitt"))
                .clicked()
            {
                page -= 1;
            }
            ui.label(format!(
                "Abschnitt {} / {pages} · {} Bytes vollständig zugänglich",
                page + 1,
                text.len()
            ));
            if ui
                .add_enabled(
                    page + 1 < pages,
                    egui::Button::new("Nächster Textabschnitt ›"),
                )
                .clicked()
            {
                page += 1;
            }
            if ui.button("Gesamten Text kopieren").clicked() {
                ui.ctx().copy_text(text.to_string());
            }
        });
        ui.data_mut(|d| d.insert_temp(id, page));
    }
    let text = text_page(text, page);
    let label = if monospace {
        egui::RichText::new(text).monospace()
    } else {
        egui::RichText::new(text)
    };
    ui.add(egui::Label::new(label).wrap().selectable(true));
}
fn text_page(text: &str, page: usize) -> &str {
    let mut start = (page * 16 * 1024).min(text.len());
    let mut end = ((page + 1) * 16 * 1024).min(text.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    while !text.is_char_boundary(end) {
        end += 1;
    }
    &text[start..end]
}

fn field_enabled(schema: &Value, values: &BTreeMap<String, Value>) -> bool {
    schema["when"].as_array().is_none_or(|conditions| {
        conditions.iter().all(|c| {
            let equal = c["key"].as_str().and_then(|k| values.get(k)) == c.get("value");
            match c["op"].as_str() {
                Some("eq") => equal,
                Some("neq") => !equal,
                _ => false,
            }
        })
    })
}
fn question_field(ui: &mut egui::Ui, schema: &Value, value: &mut Value) {
    let title = schema["title"]
        .as_str()
        .or(schema["key"].as_str())
        .unwrap_or("Feld");
    ui.strong(format!(
        "{title}{}",
        if schema["required"] == true { " *" } else { "" }
    ));
    if let Some(description) = schema["description"].as_str() {
        ui.label(description);
    }
    if schema["type"] == "external" {
        if let Some(url) = schema["url"].as_str().filter(|u| {
            u.starts_with("https://")
                || u.starts_with("http://localhost")
                || u.starts_with("http://127.0.0.1")
        }) {
            ui.hyperlink_to("Externes Formular öffnen", url);
        } else {
            ui.label("Kein gültiger Link zum externen Formular vorhanden.");
        }
        return;
    }
    let multiple = schema["type"] == "multiselect";
    if let Some(options) = schema["options"].as_array().filter(|v| !v.is_empty()) {
        for option in options {
            let label = option["label"]
                .as_str()
                .or(option["value"].as_str())
                .unwrap_or("Option");
            if multiple {
                let mut values = value.as_array().cloned().unwrap_or_default();
                let mut selected = values.contains(&option["value"]);
                if ui.checkbox(&mut selected, label).changed() {
                    if selected {
                        values.push(option["value"].clone());
                    } else {
                        values.retain(|v| v != &option["value"]);
                    }
                    *value = Value::Array(values);
                }
            } else if ui.radio(*value == option["value"], label).clicked() {
                *value = option["value"].clone();
            }
            if let Some(description) = option["description"].as_str() {
                ui.colored_label(MUTED, description);
            }
        }
        if schema["custom"] != true {
            return;
        }
        ui.label("Eigene Antwort (optional)");
    }
    match schema["type"].as_str() {
        Some("boolean") => {
            ui.horizontal(|ui| {
                if ui.radio(*value == true, "Ja").clicked() {
                    *value = json!(true);
                }
                if ui.radio(*value == false, "Nein").clicked() {
                    *value = json!(false);
                }
            });
        }
        Some("string") => {
            let mut text = value.as_str().unwrap_or("").to_string();
            if ui
                .add(
                    design::multiline(&mut text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                *value = json!(text);
            }
        }
        Some("number" | "integer") => {
            // Preserve invalid text in egui memory rather than silently resetting it.
            let id = ui.make_persistent_id((title, "numeric-answer"));
            let mut text = ui
                .data_mut(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| {
                    if value.is_null() {
                        String::new()
                    } else {
                        value.to_string()
                    }
                });
            if ui.add(design::singleline(&mut text)).changed() {
                *value = serde_json::from_str(&text).unwrap_or_else(|_| json!(text));
                ui.data_mut(|d| d.insert_temp(id, text));
            }
        }
        Some("multiselect") => {
            if value.is_null() {
                *value = json!([]);
            }
            structured_editor(ui, value, title);
        }
        _ => {
            ui.colored_label(
                RED,
                "Dieser Antworttyp wird hier nicht unterstützt.",
            );
        }
    }
}
pub fn validate_field(schema: &Value, value: &Value) -> Result<(), String> {
    let valid = match schema["type"].as_str() {
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("number") => value.is_number(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("multiselect") => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string)),
        _ => false,
    };
    if !valid {
        return Err("Bitte einen gültigen Wert eingeben".into());
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if (schema["required"] == true && text.trim().is_empty())
            || schema["minLength"].as_u64().is_some_and(|n| len < n)
            || schema["maxLength"].as_u64().is_some_and(|n| len > n)
        {
            return Err("Textlänge außerhalb der erlaubten Grenzen".into());
        }
    }
    if let Some(n) = value.as_f64() {
        if schema["minimum"].as_f64().is_some_and(|v| n < v)
            || schema["maximum"].as_f64().is_some_and(|v| n > v)
        {
            return Err("Zahl außerhalb des erlaubten Bereichs".into());
        }
    }
    if let Some(a) = value.as_array() {
        if schema["minItems"]
            .as_u64()
            .is_some_and(|n| (a.len() as u64) < n)
            || schema["maxItems"]
                .as_u64()
                .is_some_and(|n| (a.len() as u64) > n)
        {
            return Err("Anzahl der Auswahlwerte außerhalb der Grenzen".into());
        }
    }
    if let Some(options) = schema["options"].as_array().filter(|o| !o.is_empty()) {
        let values = value
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]);
        if schema["custom"] != true
            && values
                .iter()
                .any(|v| !options.iter().any(|o| o["value"] == *v))
        {
            return Err("Bitte eine angebotene Option auswählen".into());
        }
    }
    Ok(())
}

fn structured_editor(ui: &mut egui::Ui, value: &mut Value, key: &str) -> bool {
    ui.push_id(key, |ui| match value {
        Value::Object(map) => {
            let mut valid = true;
            for (key, value) in map {
                if key == "request_id" {
                    continue;
                }
                ui.label(field_label(key));
                valid &= structured_editor(ui, value, key);
            }
            valid
        }
        Value::Bool(v) => {
            ui.checkbox(v, "Aktiviert");
            true
        }
        Value::String(v) => {
            let rows = if v.contains('\n') { 4 } else { 1 };
            ui.add(
                design::multiline(v)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY),
            );
            true
        }
        _ => {
            let id = ui.make_persistent_id("json-value");
            let mut text = ui
                .data_mut(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| value.to_string());
            if ui
                .add(
                    design::multiline(&mut text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                ui.data_mut(|d| d.insert_temp(id, text.clone()));
            }
            match serde_json::from_str(&text) {
                Ok(v) => {
                    *value = v;
                    true
                }
                Err(_) => {
                    ui.colored_label(
                        RED,
                        "Ungültiger JSON-Wert. Bitte korrigieren; Senden ist gesperrt.",
                    );
                    false
                }
            }
        }
    })
    .inner
}

fn management_editor(ui: &mut egui::Ui, value: &mut Value, key: &str, app: &App) -> bool {
    if matches!(
        key,
        "expected"
            | "project_id"
            | "command_id"
            | "operation"
            | "runtime"
            | "expected_proof"
            | "repository_source"
            | "kind"
            | "action"
    ) {
        json_read(ui, value);
        return true;
    }
    if let Value::Object(map) = value {
        let mut valid = true;
        let mut names = map.keys().cloned().collect::<Vec<_>>();
        names.sort_by_key(|name| (field_rank(name), name.clone()));
        for name in names {
            if name == "request_id" || (app.models.managed() && key == "profile"
                && matches!(name.as_str(), "provider_id" | "model_id")) {
                continue;
            }
            let v = map.get_mut(&name).expect("existing key");
            ui.push_id(&name, |ui| {
                ui.label(field_label(&name));
                valid &= management_editor(ui, v, &name, app);
            });
        }
        return valid;
    }
    if let Some(s) = &app.snapshot {
        let options: Option<Vec<(Value, String)>> = match key {
            "runner_id" | "target_runner" => Some(
                s.runners
                    .iter()
                    .map(|r| {
                        (
                            json!(r.id),
                            format!("{} · {:?} · {}", r.name, r.connection, r.id),
                        )
                    })
                    .collect(),
            ),
            "stream_id" => Some(
                s.streams
                    .iter()
                    .filter(|w| Some(w.project_id) == app.settings.project)
                    .map(|w| (json!(w.id), format!("{} · {:?}", w.goal, w.status)))
                    .collect(),
            ),
            "checkpoint_command" | "review_command" => Some(
                s.commands
                    .iter()
                    .filter(|c| {
                        command_project(s, c) == app.settings.project
                            && c.kind
                                == if key == "checkpoint_command" {
                                    CommandKind::Checkpoint
                                } else {
                                    CommandKind::ReviewCheckpoint
                                }
                    })
                    .map(|c| (json!(c.id), format!("{} · {}", c.id, command_status(c))))
                    .collect(),
            ),
            "role" => Some(
                [
                    ("operator", "Operator"),
                    ("admin", "Administrator"),
                    ("runner", "Runner"),
                ]
                .into_iter()
                .map(|(v, t)| (json!(v), t.into()))
                .collect(),
            ),
            "transport" => Some(vec![
                (json!("local"), "Auf diesem Rechner".into()),
                (json!("ssh"), "Über das SSH-Profil des Runners".into()),
            ]),
            "status" => Some(
                [
                    ("backlog", "Backlog"),
                    ("ready", "Bereit"),
                    ("in_progress", "In Bearbeitung"),
                    ("blocked", "Blockiert"),
                    ("review", "Review"),
                    ("done", "Abgeschlossen"),
                    ("cancelled", "Abgebrochen"),
                ]
                .into_iter()
                .map(|(v, t)| (json!(v), t.into()))
                .collect(),
            ),
            _ => None,
        };
        if let Some(options) = options {
            let label = options.iter().find(|(v, _)| v == value).map_or_else(
                || {
                    if value.is_null() {
                        "Keine Auswahl".into()
                    } else {
                        value.to_string()
                    }
                },
                |(_, label)| label.clone(),
            );
            egui::ComboBox::from_id_salt(key)
                .selected_text(label)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if key == "runner_id" {
                        ui.selectable_value(
                            value,
                            Value::Null,
                            "Keine Bindung (nur bei gerätebezogener Runner-Rolle erforderlich)",
                        );
                    }
                    for (v, label) in options {
                        ui.selectable_value(value, v, label);
                    }
                }).response.on_hover_cursor(egui::CursorIcon::PointingHand);
            return true;
        }
        if matches!(key, "runner_ids" | "ticket_ids" | "depends_on") {
            let options = if key == "runner_ids" {
                s.runners
                    .iter()
                    .map(|r| (json!(r.id), format!("{} · {:?}", r.name, r.connection)))
                    .collect::<Vec<_>>()
            } else {
                s.tickets
                    .iter()
                    .filter(|t| Some(t.project_id) == app.settings.project && !t.draft)
                    .map(|t| (json!(t.id), task_summary(t)))
                    .collect()
            };
            let mut values = value.as_array().cloned().unwrap_or_default();
            for (id, label) in options {
                let mut checked = values.contains(&id);
                if ui.checkbox(&mut checked, label).changed() {
                    if checked {
                        values.push(id);
                    } else {
                        values.retain(|v| v != &id);
                    }
                }
            }
            *value = json!(values);
            return true;
        }
    }
    if matches!(key, "work" | "orchestration" | "memory") {
        let mut unlimited = value.is_null();
        if ui.checkbox(&mut unlimited, "Unbegrenzt").changed() {
            *value = if unlimited { Value::Null } else { json!(0) };
        }
        if unlimited {
            return true;
        }
    }
    if let Value::Number(number) = value {
        if let Some(mut n) = number.as_u64() {
            if design::number(ui, &mut n).changed() {
                *value = json!(n);
            }
            return true;
        }
    }
    structured_editor(ui, value, key)
}

pub(crate) fn read_content(ui: &mut egui::Ui, app: &mut App, value: &Value, path: &str) {
    if path.starts_with("ssh-profile/") {
        let mut body = value.clone();
        if body["profile"].is_null() {
            body["profile"] = json!({"host":"","server":"http://127.0.0.1:7432","journal":"","opencode_registration":null});
        }
        ui.label("Lokales SSH-Profil für nichtinteraktive Runner-Helfer. Hostschlüssel und Zugangsdaten bleiben in der normalen SSH-Konfiguration.");
        if ui.button("SSH-Profil bearbeiten …").clicked() {
            app.dialog = Some(Dialog::form("SSH-Profil speichern", "Absolute Pfade auf dem Runnerhost angeben. Eine zwischenzeitliche Änderung desselben Profils wird nicht überschrieben.", "LOCAL_SSH_SAVE", path.into(), body));
        }
        if !value["profile"].is_null() && ui.button("SSH-Profil entfernen …").clicked() {
            let mut body = value.clone();
            body["profile"] = Value::Null;
            app.dialog = Some(Dialog::form(
                "SSH-Profil entfernen",
                "Die lokale SSH-Zuordnung entfernen; keine Runner oder Sessions werden beendet.",
                "LOCAL_SSH_SAVE",
                path.into(),
                body,
            ));
        }
    } else if let Ok(question) = serde_json::from_value::<Question>(value.clone()) {
        question_answers(ui, &[question]);
        return;
    } else if let Ok(snapshot) = serde_json::from_value::<Snapshot>(value.clone()) {
        let selected_project = app.settings.project;
        for w in snapshot
            .streams
            .iter()
            .filter(|w| Some(w.project_id) == selected_project)
        {
            app::card().show(ui, |ui| {
                ui.strong(&w.goal);
                ui.label(format!("{:?} · Runner {}", w.status, w.runner_id));
                for (label, kind) in [("Vorbereiten …", CommandKind::PrepareWorkspace), ("OpenCode prüfen …", CommandKind::CheckOpenCode)] {
                    if ui.button(label).clicked() { app.dialog = Some(Dialog::form(label, "Technischen Auftrag an diesen Arbeitsbereich senden. Die Serverrevision schützt vor parallelen Änderungen.", "POST", format!("/v1/streams/{}/commands", w.id), json!(EnqueueCommand { request_id: RequestId::new(), expected_revision: w.revision, kind, prompt: None }))); }
                }
                for c in snapshot.commands.iter().filter(|c| c.stream_id == w.id) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{:?} · {}", c.kind, command_status(c)));
                        ui.menu_button("Verwalten …", |ui| app.command_menu(ui, c));
                    });
                    ui.collapsing(format!("Auftrag {}", c.id), |ui| json_read(ui, &json!(c)));
                }
            });
        }
    } else if path.ends_with("/runtime-binding") {
        if let Ok(binding) = serde_json::from_value::<RuntimeBinding>(value.clone()) {
            ui.label(format!(
                "Container {} · gestartet {}",
                binding.runtime.container_id, binding.runtime.started_at
            ));
            if ui.button("Diese Runtime gezielt beenden …").clicked() {
                let revision = app
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.streams.iter().find(|w| w.id == binding.stream_id))
                    .map_or(Revision(0), |w| w.revision);
                app.dialog = Some(Dialog::form("Runtime unwiderruflich beenden", "Nur die bestätigte dedizierte Runtime wird entfernt. Hintergrundprozesse werden erst nach bestätigtem Stopp als beendet behandelt. Worktree und Volumes bleiben erhalten; Archiv kann unvollständig sein.", "POST", format!("/v1/commands/{}/termination", binding.command_id), json!(RequestTermination { request_id: RequestId::new(), expected_stream_revision: revision, runtime: binding.runtime, previous_request: None, reason: String::new() })));
            }
        } else {
            ui.label("Keine bestätigte dedizierte Container-Runtime verfügbar. Native Sessionsteuerung ist getrennt über den Arbeitsbereich möglich.");
        }
    } else if path.ends_with("/files") {
        if let Ok(index) = serde_json::from_value::<ArchiveBlobIndex>(value.clone()) {
            let command = path
                .split('/')
                .nth(3)
                .and_then(|s| s.parse::<CommandId>().ok());
            if index.files.is_empty() {
                ui.label("Keine separat gesicherten Dateien. Inline-Anhänge und Unverfügbarkeitsgründe stehen zusätzlich bei der Originalnachricht.");
            }
            for file in index.files {
                app::card().show(ui, |ui| {
                    ui.strong(&file.name);
                    ui.label(format!(
                        "{} · {} Bytes · erfasst {}",
                        file.mime,
                        file.bytes,
                        timestamp(file.captured_unix_ms)
                    ));
                    ui.colored_label(MUTED, &file.sha256);
                    if let Some(command) = command {
                        if ui
                            .add_enabled(
                                app.online,
                                egui::Button::new("SHA256-geprüft exportieren …"),
                            )
                            .clicked()
                        {
                            app.backend.save_archive_file(
                                command,
                                file.sha256.clone(),
                                file.name.clone(),
                            );
                        }
                    }
                });
            }
        }
    } else if let Ok(devices) = serde_json::from_value::<Vec<Device>>(value.clone()) {
        for device in devices {
            app::card().show(ui, |ui| {
            ui.strong(&device.name); ui.label(format!("{:?} · {} · {}", device.role, device.id, if device.revoked { "widerrufen" } else { "aktiv" }));
            if !device.revoked && ui.button("Zugang widerrufen …").clicked() { app.dialog = Some(Dialog::form("Gerätezugang widerrufen", &format!("Zugang für „{}“ ({}) unwiderruflich sperren. Laufende Geräteverbindungen verlieren ihre Berechtigung.", device.name, device.id), "POST", format!("/v1/devices/{}/revoke", device.id), Value::Null)); }
        });
        }
    } else if let Ok(notes) = serde_json::from_value::<Vec<KnowledgeNote>>(value.clone()) {
        for note in notes {
            app::card().show(ui, |ui| {
            ui.strong(&note.title); ui.label(&note.content); ui.colored_label(MUTED, format!("Quelle: {} · Branch: {:?} · abgelöst: {}", note.source, note.branch, note.retired));
            if !note.retired && ui.button("Wissen ablösen …").clicked() { app.dialog = Some(Dialog::form("Wissen ablösen", "Der Eintrag bleibt historisch erhalten, wird aber nicht mehr automatisch als aktuelles Wissen verwendet.", "POST", format!("/v1/knowledge/{}/retire", note.id), json!(RetireKnowledge { request_id: RequestId::new(), expected_revision: note.revision }))); }
            ui.collapsing("Metadaten", |ui| json_read(ui, &json!(note)));
        });
        }
    } else if let Ok(policy) = serde_json::from_value::<SupervisorPolicy>(value.clone()) {
        ui.label(format!(
            "{} · {} Auswertungen verbleibend · Intervall {} s",
            if policy.enabled {
                "Aktiv"
            } else {
                "Deaktiviert"
            },
            policy.remaining_evaluations,
            policy.interval_seconds
        ));
        if ui.button("Supervisor ändern …").clicked() {
            app.dialog = Some(Dialog::form("Supervisor konfigurieren", "Budget und Intervall gelten für zukünftige Auswertungen. Aktuelle Revision wird übernommen.", "PUT", format!("/v1/projects/{}/supervisor", policy.project_id), json!(SetSupervisor { request_id: RequestId::new(), expected_revision: policy.revision, stream_id: policy.stream_id, enabled: policy.enabled, evaluation_budget: policy.remaining_evaluations, interval_seconds: policy.interval_seconds })));
        }
    } else if let Ok(budget) = serde_json::from_value::<TokenBudget>(value.clone()) {
        if ui.button("Tokenlimits ändern …").clicked() {
            app.dialog = Some(Dialog::form("Tokenbudget ändern", "Limits gelten für beobachtete Tokenverbräuche. Fehlende historische Angaben sind keine Nullverbräuche. null bedeutet unbegrenzt.", "PUT", format!("/v1/projects/{}/token-budget", budget.project_id), json!(SetTokenBudget { request_id: RequestId::new(), expected_revision: budget.revision, limits: budget.limits })));
        }
    }
    ui.collapsing("Vollständige Daten", |ui| json_read(ui, value));
}
fn field_label(key: &str) -> &str {
    match key {
        "key" => "Projektkürzel (z. B. TEST oder WOAI; neu: leer = automatisch)",
        "confirm_name" => "Projektname zur Bestätigung",
        "expected_revision" | "expected_stream_revision" => {
            "Erwartete Serverrevision (bei Konflikt aktuellen Stand prüfen)"
        }
        "name" => "Name",
        "repository" => "Repository / Quelle",
        "base_branch" => "Basisbranch",
        "provider_id" => "Anbieter-ID",
        "model_id" => "Modell-ID",
        "toolchain" => "Toolchain",
        "start_command" => "Startbefehl",
        "test_command" => "Testbefehl",
        "enabled" => "Automatik aktiv",
        "paused" => "Pausiert",
        "runner_ids" => "Runner-IDs",
        "runner_id" => "Runner-ID",
        "max_parallel" => "Maximal parallele Aufträge",
        "max_sessions" => "Maximal arbeitende Sessions",
        "start_budget" => "Startbudget (4294967295 = unbegrenzt)",
        "evaluation_budget" => "Auswertungsbudget",
        "interval_seconds" => "Intervall in Sekunden",
        "stream_id" => "Arbeitsbereich-ID",
        "reason" => "Begründung",
        "goal" => "Ziel",
        "title" => "Titel",
        "description" => "Beschreibung",
        "content" => "Inhalt",
        "acceptance_criteria" => "Erfolgskriterien",
        "priority" => "Priorität (0–3)",
        "ticket_ids" => "Aufgaben-IDs",
        "depends_on" => "Abhängigkeiten (Aufgaben-IDs)",
        "role" => "Rolle",
        "ttl_seconds" => "Gültigkeit in Sekunden",
        "repository_path" => "Repositorypfad auf dem Runner",
        "service_registration" => "OpenCode-Service-Registrierung auf dem Runner",
        _ => key,
    }
}
fn field_rank(key: &str) -> u8 {
    match key {
        "name" | "title" | "goal" | "operation" => 0,
        "repository" | "description" | "content" | "reason" => 10,
        "base_branch" | "acceptance_criteria" | "priority" | "transport" | "enabled" | "paused" => {
            20
        }
        "profile" | "provider_id" | "role" => 30,
        "model_id" | "setup" | "runner_id" | "runner_ids" => 40,
        "project_id"
        | "command_id"
        | "expected_revision"
        | "expected_stream_revision"
        | "expected" => 99,
        _ => 50,
    }
}
fn current_revision(app: &App, path: &str, body: &Value) -> Option<(u64, Value)> {
    let s = app.snapshot.as_ref()?;
    if body.get("expected_revision").is_none() && body.get("expected_stream_revision").is_none() {
        return None;
    }
    let parts = path.split('/').collect::<Vec<_>>();
    match parts.get(2).copied()? {
        "projects" => {
            let id = parts
                .get(3)?
                .parse::<ProjectId>()
                .ok()
                .or_else(|| serde_json::from_value(body["project_id"].clone()).ok())?;
            match parts.get(4).copied() {
                Some("scheduler") => {
                    let p = s.scheduler_policies.iter().find(|p| p.project_id == id)?;
                    Some((p.revision.0, json!(p)))
                }
                Some("token-budget") => {
                    let p = s.token_budgets.iter().find(|p| p.project_id == id)?;
                    Some((p.revision.0, json!(p)))
                }
                Some("supervisor") => None,
                _ => {
                    let p = s.projects.iter().find(|p| p.id == id)?;
                    Some((p.revision.0, json!(p)))
                }
            }
        }
        "tickets" => {
            let id = parts.get(3)?.parse::<TicketId>().ok()?;
            let t = s.tickets.iter().find(|t| t.id == id)?;
            Some((t.revision.0, json!(t)))
        }
        "streams" => {
            let id = parts.get(3)?.parse::<StreamId>().ok()?;
            let w = s.streams.iter().find(|w| w.id == id)?;
            Some((w.revision.0, json!(w)))
        }
        "commands" => {
            let id = parts.get(3)?.parse::<CommandId>().ok()?;
            let c = s.commands.iter().find(|c| c.id == id)?;
            let w = s.streams.iter().find(|w| w.id == c.stream_id)?;
            Some((w.revision.0, json!(w)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_editor_detects_the_local_branch_and_leaves_runner_mapping_automatic() {
        let mut app = crate::tests::fixture();
        app.project_editor(None);
        let Some(Dialog::Form(mut form)) = app.dialog.take() else { panic!("expected project form") };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C").arg(path).args(args).output().unwrap();
            assert!(output.status.success(), "{output:?}");
        };
        git(&["init", "-b", "develop"]);
        git(&["-c", "user.name=Factory Test", "-c", "user.email=test@example.invalid",
            "commit", "--allow-empty", "-m", "Initial"]);
        form.body["name"] = json!(" Neues Projekt ");
        form.body["key"] = json!(" woai ");
        form.body["repository"] = json!(path);
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["name"], "Neues Projekt");
        assert_eq!(request["key"], "WOAI");
        assert_eq!(request["repository"], path);
        assert_eq!(request["base_branch"], "develop");
        assert!(request.get("setup").is_none());
        assert!(request.get("runner_id").is_none());
        assert!(request.get("runner_mapping").is_none());
        let runner = app.snapshot.as_ref().unwrap().runners[0].id;
        form.body["runner_mapping"] = json!(ProjectRunnerMapping::Runner(runner));
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["runner_mapping"], json!({"runner": runner}));
        let nested = dir.path().join("unterordner");
        assert_eq!(local_project_branch(nested.to_str().unwrap()).unwrap(), "main");
        assert!(!nested.exists());
        std::fs::create_dir(&nested).unwrap();
        assert!(local_project_branch(nested.to_str().unwrap()).is_err());
        assert!(local_project_branch("relative").is_err());
    }

    #[test]
    fn editing_project_details_does_not_reconfigure_an_existing_repository() {
        let mut app = crate::tests::fixture();
        let project = &mut std::sync::Arc::make_mut(app.snapshot.as_mut().unwrap()).projects[0];
        project.key = "OLD".into();
        project.repository = "git@example.test:repo".into();
        project.base_branch = "develop".into();
        let project = project.clone();
        app.project_editor(Some(&project));
        let Some(Dialog::Form(mut form)) = app.dialog.take() else { panic!("expected project form") };
        assert!(form.body["repository"].is_null());
        form.body["name"] = json!("Umbenannt");
        form.body["key"] = json!("new");
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["name"], "Umbenannt");
        assert_eq!(request["key"], "NEW");
        assert!(request["repository"].is_null());
        assert!(request["base_branch"].is_null());
        assert!(request.get("profile").is_none());
        form.persist(&mut app);
        app.project_editor(Some(&project));
        assert!(matches!(&app.dialog, Some(Dialog::Form(form)) if form.body["name"] == "Umbenannt"));
    }

    #[test]
    fn answers_validate_choices_unicode_and_limits() {
        assert!(validate_field(
            &json!({"type":"string","required":true,"minLength":2}),
            &json!("界界")
        )
        .is_ok());
        assert!(validate_field(&json!({"type":"string","required":true}), &json!("")).is_err());
        assert!(validate_field(&json!({"type":"integer","minimum":1}), &json!(0)).is_err());
        assert!(validate_field(
            &json!({"type":"multiselect","minItems":1,"options":[{"value":"a"}]}),
            &json!(["b"])
        )
        .is_err());
        assert!(!field_enabled(
            &json!({"when":[{"key":"x","op":"eq","value":true}]}),
            &BTreeMap::new()
        ));
    }
    #[test]
    fn long_text_pages_reconstruct_every_unicode_byte() {
        let text = "Grüße 界 🦀\n".repeat(10000);
        let reconstructed = (0..text.len().div_ceil(16 * 1024))
            .map(|p| text_page(&text, p))
            .collect::<String>();
        assert_eq!(reconstructed, text);
    }
}
