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

pub(crate) const ABOUT_GITHUB: &str = "https://github.com/farion/yoloforge";

fn show_about(ctx: &egui::Context) -> bool {
    let available = ctx.content_rect().size() - egui::Vec2::splat(2.0 * design::DIALOG_OUTER_GAP);
    let width = available.x.min(available.y * 4.0 / 3.0).clamp(1.0, 840.0);
    let size = egui::vec2(width, width * 3.0 / 4.0);
    let response = egui::Modal::new(egui::Id::new("yoloforge-about"))
        .frame(egui::Frame::window(&ctx.style()).inner_margin(0))
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            egui::Image::from_bytes(
                "bytes://yoloforge-about.jpg",
                include_bytes!("../../../yoloforge.jpg").as_slice(),
            )
            .show_loading_spinner(false)
            .paint_at(ui, rect);

            let padding = design::DIALOG_PADDING as f32;
            let content = rect.shrink(padding);
            let header_band = egui::Rect::from_min_max(
                rect.min, egui::pos2(rect.right(), content.top() + 36.0),
            );
            let shade = egui::Color32::from_black_alpha(185);
            ui.painter().rect_filled(header_band, 0, shade);
            let mut header = ui.new_child(egui::UiBuilder::new().id_salt("header").max_rect(content));
            header.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
            let close = design::dialog_header(&mut header, crate::tr!("About")).clicked();

            let mut footer = ui.new_child(
                egui::UiBuilder::new().id_salt("info").max_rect(content)
                    .layout(egui::Layout::bottom_up(egui::Align::Max)),
            );
            footer.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
            footer.visuals_mut().hyperlink_color = egui::Color32::WHITE;
            let backing = footer.painter().add(egui::Shape::Noop);
            let link = footer.hyperlink_to("GitHub", ABOUT_GITHUB);
            let license = footer.label(crate::tr!("License GPL v3.0"));
            let version = footer.label(concat!("v", env!("CARGO_PKG_VERSION")));
            let name = footer.heading("YoloForge");
            let info = link.rect.union(license.rect).union(version.rect).union(name.rect);
            footer.painter().set(backing, egui::Shape::rect_filled(info.expand(12.0), 6, shade));
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(
                egui::Id::new("test-about-layout"), (rect, info, link.rect),
            ));
            close
        });
    response.inner || response.should_close()
}

pub enum Dialog {
    About,
    ConnectionInfo,
    Connection {
        server: String,
        token: String,
    },
    LocalFactory { starting: bool },
    RemoveLocalRunner { id: RunnerId, name: String },
    RestartLocalRunner { id: RunnerId, name: String },
    RemoveOfflineTask(RequestId),
    Task(Box<TaskDialog>),
    TaskEditing {
        id: TicketId,
        request: u64,
    },
    Questions(QuestionDialog),
    QuestionView { questions: Vec<Question>, page: usize },
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
}
pub struct TaskDialog {
    pub(crate) advanced: bool,
    pub(crate) local_task: Option<RequestId>,
    local_done: bool,
    session_prompt: bool,
    pub read_only: bool,
    pub payloads_complete: bool,
    pub(super) return_to_questions: Option<Box<Dialog>>,
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
    SaveDraftAndNew,
    Queue,
    QueueAndNew,
}
pub struct QuestionDialog {
    pub questions: Vec<QuestionAnswer>,
    pub error: Option<String>,
    step: usize,
}
pub struct QuestionAnswer {
    pub question: Question,
    values: BTreeMap<String, Value>,
    decision: String,
    request: Option<u64>,
    restored: bool,
}
pub struct FormDialog {
    pub(crate) settings_baseline: Option<Value>,
    pub id: RequestId,
    pub title: String,
    pub help: String,
    pub method: String,
    pub path: String,
    pub body: Value,
    pub request: Option<u64>,
    pub error: Option<String>,
    folder_picker: Option<std::sync::mpsc::Receiver<Option<String>>>,
    pub(crate) logo_picker: crate::project_logo::Picker,
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
        }
    }
    pub fn task(server: String, project: ProjectId, ticket: Option<&Ticket>) -> Self {
        Self::Task(Box::new(TaskDialog {
            advanced: false,
            local_task: None,
            local_done: false,
            session_prompt: false,
            read_only: false,
            payloads_complete: true,
            return_to_questions: None,
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
                local_mcps: None,
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
                .map(QuestionAnswer::new)
                .collect(),
            error: None,
            step: 0,
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
            settings_baseline: None,
            id: RequestId::new(),
            title: title.into(),
            help: help.into(),
            method: method.into(),
            path,
            body,
            request: None,
            error: None,
            folder_picker: None,
            logo_picker: Default::default(),
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
        if matches!(self, Self::About) {
            return show_about(ctx);
        }
        if let Self::Task(task) = self {
            task.restore_submission(app);
            if !task.read_only
                && (task.local_done || task.request.is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)))
            {
                if matches!(task.submit_action, TaskSubmitAction::QueueAndNew | TaskSubmitAction::SaveDraftAndNew) {
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
        if let Self::Form(form) = self {
            if form.intent_confirmation().is_some() {
                content_size.x = content_size.x.min(if form.method == "DELETE" { 440.0 } else { 520.0 });
            }
        }
        if matches!(self, Self::Read { path, .. } if is_project_usage(path)) {
            content_size.x = content_size.x.min(460.0);
        }
        if matches!(self, Self::ModelSetupRequired | Self::ModelDefaultsConfigured { .. }) {
            content_size.x = content_size.x.min(480.0);
        }
        if matches!(self, Self::RestartLocalRunner { .. }) {
            content_size.x = content_size.x.min(480.0);
        }
        let width = content_size.x;
        let body_height = (content_size.y - 62.0).max(40.0);
        let height = (body_height - 160.0).max(80.0);
        let title = match &*self {
            Self::About => unreachable!("About uses its own full-bleed layout"),
            Self::Connection { .. } => crate::tr!("Mit Factory verbinden").into(),
            Self::LocalFactory { .. } => crate::tr!("Lokale Factory").into(),
            Self::RemoveLocalRunner { .. } => crate::tr!("Lokalen Runner entfernen").into(),
            Self::RestartLocalRunner { .. } => crate::tr!("Lokalen Runner neu starten").into(),
            Self::RemoveOfflineTask(_) => crate::tr!("Lokale Aufgabe entfernen").into(),
            Self::Task(task) => if task.read_only {
                crate::tr!("Aufgabe anzeigen · schreibgeschützt")
            } else if task.draft.ticket.is_some() {
                crate::tr!("Aufgabe bearbeiten")
            } else {
                crate::tr!("Neue Aufgabe")
            }
            .into(),
            Self::TaskEditing { .. } => crate::tr!("Aufgabe zum Bearbeiten sichern").into(),
            Self::Questions(group) => if group.questions.iter().any(|entry| needs_answer(&entry.question)) {
                crate::tr!("Fragen beantworten")
            } else {
                crate::tr!("Fragen und Antworten")
            }.into(),
            Self::QuestionView { .. } => crate::tr!("Fragen und Antworten").into(),
            Self::Form(form) => crate::i18n::label(&form.title).into(),
            Self::ProjectSettings(settings) => settings.title(app),
            Self::ConnectionInfo => crate::tr!("Verbindungsübersicht").into(),
            Self::FactorySettings => crate::tr!("Factory-Einstellungen").into(),
            Self::ModelSetupRequired => crate::tr!("OpenCode einrichten").into(),
            Self::ModelDefaultsConfigured { .. } => crate::tr!("Default-Modelle eingerichtet").into(),
            Self::Read { title, .. } => crate::i18n::label(title).into(),
            Self::Image { image, .. } => image.name.clone(),
            Self::Pending => crate::tr!("Offene Zustellungen").into(),
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
            Self::RestartLocalRunner { id, .. } => egui::Id::new(("factory-runner-restart", *id)),
            _ => egui::Id::new("factory-dialog"),
        };
        let frame = egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING);
        let response = egui::Modal::new(modal_id).frame(frame).show(ctx, |ui| {
                ui.set_width(width);
                // Give the auto-sized Area the full available limit, not a fixed
                // height or its previous small content height.
                 ui.set_max_height(content_size.y);
                  if matches!(self, Self::ProjectSettings(settings) if settings.is_saving())
                      || matches!(self, Self::Task(task) if task.is_transferring(app))
                      || matches!(self, Self::Questions(group) if group.is_transferring(app))
                  {
                     ui.disable();
                 }
                let header_top = ui.cursor().top();
                header_close = design::dialog_header(ui, &title).clicked();
                 ui.separator();
                 let body_height = (content_size.y - (ui.cursor().top() - header_top)).max(40.0);
                  if let Self::Questions(group) = self {
                      close = group.show(ui, app, body_height);
                      return;
                  }
                  if let Self::QuestionView { questions, page } = self {
                      show_question_pages(ui, app, questions.iter(), page, body_height);
                      return;
                  }
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
                     ui.label(crate::tr!("Du bist mit einer Factory verbunden, für die noch kein Standardmodell eingerichtet ist."));
                     if app.access.as_ref().is_some_and(|access| access.role != DeviceRole::Admin) {
                         ui.label(crate::tr!("Ein Factory-Admin muss einen Anbieter verbinden und ein Standardmodell auswählen, bevor Aufgaben eingereiht werden können."));
                     } else {
                         ui.label(crate::tr!("Verbinde in den Einstellungen einen Anbieter. Die Factory richtet dann automatisch ein Standardmodell ein."));
                     }
                     ui.add_space(8.0);
                     ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                         let settings = design::accent_button(ui, egui::Button::new(crate::tr!("Zu den Modell-Anbietern")), true, BLUE.gamma_multiply(0.7));
                         #[cfg(test)]
                         ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-model-setup-continue"), settings.rect));
                         if settings.clicked() {
                             crate::model_settings::open_setup(app);
                             close = true;
                         } else if ui.button(crate::tr!("Später")).clicked() {
                             close = true;
                         }
                     }));
                     return;
                 }
                 if let Self::ModelDefaultsConfigured { model } = self {
                     ui.label(crate::tr!("Die Factory hat deine Default-Modelle automatisch eingerichtet."));
                     ui.label(crate::tr!("Standardmodell: {model}", model = model));
                     ui.label(crate::tr!("Alle Rollen verwenden dieses Modell, solange kein eigenes Rollenmodell gewählt ist."));
                     ui.add_space(8.0);
                     ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                         if design::accent_button(ui, egui::Button::new(crate::tr!("Zu den Default-Modellen")), true, BLUE.gamma_multiply(0.7)).clicked() {
                             close = true;
                         }
                     }));
                     return;
                 }
                  if matches!(self, Self::ConnectionInfo) {
                      egui::ScrollArea::vertical().id_salt((modal_id, "body"))
                         .max_height((body_height - 60.0).max(40.0))
                         .auto_shrink([false, true])
                         .show(ui, |ui| crate::connection_info::show(ui, app));
                     ui.add_space(12.0);
                     crate::connection_info::actions(ui, app);
                     return;
                 }
               if let Self::RestartLocalRunner { id, name } = self {
                  let blocked = app.snapshot.as_ref().is_none_or(|s| crate::local_runners::restart_blocked(s, *id));
                  let recovery_start = blocked && app.snapshot.as_ref()
                      .is_some_and(|s| crate::local_runners::recovery_start_available(s, *id));
                  let admin = app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin);
                   let incompatible = app.snapshot.as_deref()
                       .is_some_and(|s| !server_supports(s, RUNNER_RESTART_CAPABILITY));
                   let _body = egui::ScrollArea::vertical().id_salt((modal_id, "body"))
                      .max_height((body_height - 72.0).max(24.0)).auto_shrink([false, true])
                      .show(ui, |ui| {
                          ui.label(crate::tr!("„{name}“ mit der installierten Runner-Version neben dieser GUI neu starten?", name = name));
                          ui.label(crate::tr!("Runner-Identität, Zugang, Journal und Arbeitsdaten bleiben erhalten. Eingereihte Aufträge werden danach fortgesetzt."));
                          if recovery_start {
                              ui.label(crate::tr!("Der Runner wird nur gestartet, wenn er lokal bereits beendet ist. Offene Aufträge bleiben zur Wiederaufnahme erhalten."));
                          } else if blocked {
                              ui.label(crate::tr!("Laufende oder ungeklärte Aufträge müssen zuerst abgeschlossen werden."));
                          }
                          if !admin { ui.label(crate::tr!("Für den Runner-Neustart ist ein Admin-Zugang erforderlich.")); }
                       });
                   #[cfg(test)]
                   ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-runner-restart-body"), (_body.inner_rect, _body.content_size)));
                  ui.add_space(12.0);
                  ui.separator();
                   let footer_bottom = ui.cursor().top() + design::SETTINGS_FOOTER_HEIGHT;
                   design::settings_footer(ui, footer_bottom, |ui| {
                       let confirm = design::server_compatibility_tooltip(
                            ui.add_enabled(!incompatible && (!blocked || recovery_start) && admin && app.online && !app.local_runners_busy,
                                egui::Button::new(if recovery_start { crate::tr!("Runner starten") } else { crate::tr!("Runner neu starten") })),
                            incompatible,
                        );
                      #[cfg(test)]
                      ui.ctx().data_mut(|d| {
                          d.insert_temp(egui::Id::new("test-runner-restart-confirm"), confirm.rect);
                          d.insert_temp(egui::Id::new("test-runner-restart-confirm-enabled"), confirm.enabled());
                      });
                       if confirm.clicked() {
                          app.manage_local_runners(crate::local_runners::Action::Restart { id: *id, recovery_start });
                          close = true;
                      }
                      if ui.button(crate::tr!("Abbrechen")).clicked() { close = true; }
                   });
                   return;
              }
             match self {
                 Self::Form(form) if form.is_project_delete() => {
                     let body = egui::ScrollArea::vertical().id_salt((modal_id, "body"))
                         .max_height((body_height - design::SETTINGS_FOOTER_HEIGHT - 12.0).max(0.0))
                         .auto_shrink([false, true])
                         .show(ui, |ui| form.show(ui, app, height));
                     close = body.inner;
                     if !close {
                         ui.separator();
                         form.show_delete_footer(ui, app);
                     }
                     #[cfg(test)]
                     ui.ctx().data_mut(|data| data.insert_temp(modal_id.with("test-body-scroll"), (body.inner_rect, body.content_size)));
                     return;
                 }
                 Self::Form(form) if form.intent_confirmation().is_some() => {
                     let (action, color, test_id) = form.intent_confirmation().unwrap();
                     form.restore_pending(app);
                     if form.request.is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)) {
                         close = true;
                         return;
                     }
                      const DIVIDER_HEIGHT: f32 = 6.0;
                      let text_limit = (body_height - design::SETTINGS_FOOTER_HEIGHT
                          - DIVIDER_HEIGHT - 2.0 * ui.spacing().item_spacing.y).max(0.0);
                      let text = egui::WidgetText::from(form.help.as_str()).into_galley(
                          ui, Some(egui::TextWrapMode::Wrap), ui.available_width(), egui::TextStyle::Body,
                      );
                      // A short confirmation is just text and actions. Only give
                      // the message a scroll viewport when it would crowd out the footer.
                      if text.size().y <= text_limit {
                          let _label = ui.label(text);
                          #[cfg(test)]
                          ui.ctx().data_mut(|d| d.insert_temp(modal_id.with("test-confirmation-body"), (_label.rect, false)));
                      } else {
                          let _body = egui::ScrollArea::vertical().id_salt((modal_id, "body"))
                              .max_height(text_limit).min_scrolled_height(0.0).auto_shrink([false, true])
                              .show(ui, |ui| { ui.label(&form.help); });
                          #[cfg(test)]
                          ui.ctx().data_mut(|d| d.insert_temp(modal_id.with("test-confirmation-body"), (_body.inner_rect, true)));
                      }
                      ui.add(egui::Separator::default().spacing(DIVIDER_HEIGHT));
                     close = form.show_intent_confirmation(ui, app, action, color, test_id);
                     return;
                 }
                 _ => {}
             }
             let _body = egui::ScrollArea::vertical().id_salt((modal_id, "body")).max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
             match self {
                 Self::About => unreachable!("About uses its own full-bleed layout"),
                 Self::RestartLocalRunner { .. } => unreachable!("Restart uses a fixed footer"),
                Self::ConnectionInfo=>crate::connection_info::show(ui,app),
                Self::Connection { server, token } => {
                    ui.label(crate::tr!("Serveradresse (HTTPS oder Loopback)")); ui.add(design::singleline(server).desired_width(f32::INFINITY));
                    ui.label(crate::tr!("Zugangstoken")); ui.add(design::singleline(token).password(true).desired_width(f32::INFINITY));
                    ui.small(crate::tr!("Die Verbindung wird für den nächsten Start im Benutzerverzeichnis gespeichert."));
                    ui.horizontal(|ui| {
                        if design::accent_button(ui,egui::Button::new(crate::tr!("Verbinden")),!app.connection_active(),BLUE.gamma_multiply(0.7)).clicked() {
                            match app.connect(server.clone(), token.clone()) {
                                Ok(()) => close = true,
                                Err(e) => app.notice_error(e),
                            }
                        }
                    });
                }
                Self::LocalFactory { starting } => {
                    if *starting {
                        ui.horizontal(|ui| { ui.spinner(); ui.label(crate::tr!("Lokale Dienste starten …")); });
                    } else if ui.button(crate::tr!("Erneut versuchen")).clicked() {
                        *starting = true;
                        app.backend.start_local();
                    }
                    ui.label(crate::tr!("Laufende Aufgaben bleiben beim Schließen der GUI erhalten."));
                }
                Self::RemoveLocalRunner { id, name } => {
                    ui.label(crate::tr!("„{name}“ stoppen und aus den gespeicherten lokalen Runnern entfernen?", name = name));
                    ui.label(crate::tr!("Der Runner wird beim nächsten GUI-Start nicht mehr gestartet. Journale und Arbeitsdaten bleiben erhalten."));
                    let busy = app.snapshot.as_ref().is_none_or(|s| crate::local_runners::busy(s, *id));
                    if busy {
                        ui.label(crate::tr!("Dieser Runner hat laufende oder ungeklärte Aufträge. Diese zuerst abschließen."));
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy && app.online && !app.local_runners_busy,
                            egui::Button::new(crate::tr!("Runner entfernen"))).clicked() {
                            app.manage_local_runners(crate::local_runners::Action::Remove(*id));
                            close = true;
                        }
                        if ui.button(crate::tr!("Abbrechen")).clicked() { close = true; }
                    });
                }
                Self::RemoveOfflineTask(id) => {
                    ui.label(crate::tr!("Diese lokal vorbereitete Aufgabe samt Anhängen entfernen?"));
                    ui.horizontal(|ui| {
                        let allowed = app.settings.offline_tasks.iter().any(|t| t.id == *id && t.sync_request.is_none());
                        if ui.add_enabled(allowed, egui::Button::new(crate::tr!("Entfernen"))).clicked() {
                            app.settings.offline_tasks.retain(|t| t.id != *id);
                            app.settings_dirty = true;
                            close = true;
                        }
                        if ui.button(crate::tr!("Abbrechen")).clicked() { close = true; }
                    });
                }
                Self::Task(_) => {},
                Self::ProjectSettings(_)=>{},
                 Self::FactorySettings=>{},
                 Self::ModelSetupRequired=>{},
                 Self::ModelDefaultsConfigured { .. }=>{},
                Self::TaskEditing {id,request} => {
                    ui.label(crate::tr!("Die Aufgabe wird serverseitig gegen Einplanung gesichert. Erst nach Bestätigung öffnet sich der Editor mit allen Anhängen."));
                    ui.label(crate::tr!("Beim Schließen bleibt die Aufgabe als Draft gesichert. Erst „Einreihen“ gibt sie wieder frei."));
                    request_status(ui,app,*request);
                    ui.small(crate::tr!("Aufgabe {id}", id = id));
                    close = ui.button(crate::tr!("Schließen")).clicked();
                }
                 Self::Questions(_) => unreachable!("Questions use a fixed footer"),
                  Self::QuestionView { .. } => unreachable!("Question views use a fixed footer"),
                Self::Form(form) => close = form.show(ui, app, if form.is_project_editor() { body_height } else { height }),
                Self::Read { request, path, .. } if is_project_usage(path) => {
                    match app.responses.get(request) {
                        Some(Ok(value)) => match serde_json::from_value::<ProjectUsage>(value.clone()) {
                            Ok(report) => {
                                let created = app.snapshot.as_ref()
                                    .and_then(|s| s.projects.iter().find(|p| p.id == report.project_id))
                                    .map(|p| p.created_unix_ms);
                                project_usage_summary(ui, &report, created, chrono::Local::now().date_naive());
                            }
                            Err(_) => usage_load_failed(ui, app, modal_id),
                        },
                        Some(Err(_)) => usage_load_failed(ui, app, modal_id),
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(crate::tr!("Verbrauch wird geladen …"));
                            });
                        }
                    }
                }
                Self::Read { request, path, .. } => {
                    egui::ScrollArea::both().id_salt("read-dialog").max_height(height).show(ui, |ui| {
                        match app.responses.get(request).cloned() {
                            Some(Ok(value)) => { read_content(ui, app, &value, path); },
                            Some(Err(e)) => { ui.colored_label(RED, &e.message); },
                            None => { ui.spinner(); ui.label(crate::tr!("Wird geladen …")); },
                        }
                    });
                    ui.horizontal(|ui| { if ui.button(crate::tr!("Schließen")).clicked() { close = true; }
                        if path != "/v1/devices" {
                            if let Some(Ok(v)) = app.responses.get(request) { if ui.button(crate::tr!("Exportieren …")).clicked() { app.backend.save_json("yoloforge-details.json", v.clone()); } }
                        }
                    });
                }
                Self::Image { image, key } => {
                    egui::ScrollArea::both().max_height(height).show(ui, |ui| {
                        app.preview_image(ui, format!("full/{key}"), image, egui::vec2(width, height), true);
                    });
                    ui.small(crate::tr!("Vorschau an die maximale Texturgröße des Geräts angepasst. Originalbytes bleiben unverändert."));
                    if ui.button(crate::tr!("Originalbild speichern …")).clicked() { app.backend.save_input_file(image.name.clone(), image.data.clone()); }
                    close = ui.button(crate::tr!("Schließen")).clicked();
                }
                Self::Pending => {
                    close=pending_requests(ui,app,height,true);
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
        if let Self::ProjectSettings(settings) = self {
            if !close && !settings.confirmation_open() && !settings.is_saving()
                && (header_close || response.should_close()) {
                close = settings.request_close(app);
            }
            if !close {
                close = settings.overlay(ctx, app);
            }
        } else if matches!(self, Self::Task(task) if task.is_transferring(app))
            || matches!(self, Self::Questions(group) if group.is_transferring(app))
        {
            design::dialog_loading_overlay(&response.response, frame.corner_radius);
        } else if header_close || response.should_close() {
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

fn is_project_usage(path: &str) -> bool {
    path.strip_prefix("/v1/projects/")
        .and_then(|path| path.strip_suffix("/usage"))
        .is_some_and(|id| id.parse::<ProjectId>().is_ok())
}

fn usage_load_failed(ui: &mut egui::Ui, app: &mut App, modal_id: egui::Id) {
    let first = ui.ctx().data_mut(|data| {
        let key = modal_id.with("usage-error-notified");
        let notified = data.get_temp::<bool>(key).unwrap_or(false);
        data.insert_temp(key, true);
        !notified
    });
    if first {
        app.notice_error(crate::tr!("Verbrauch konnte nicht geladen werden."));
    }
    ui.label(crate::tr!("Keine Verbrauchsdaten verfügbar."));
}

fn project_usage_summary(
    ui: &mut egui::Ui,
    report: &ProjectUsage,
    project_created_unix_ms: Option<u64>,
    today: chrono::NaiveDate,
) {
    if !report.groups.iter().any(|group| group.assistant_messages > group.messages_missing_tokens) {
        ui.label(crate::tr!("Noch kein Tokenverbrauch erfasst."));
        return;
    }
    // Match the session counters: use input and output, not the separate
    // cache/reasoning counters, whose overlap depends on the provider.
    let total: u128 = report.groups.iter().map(|group| {
        u128::from(group.tokens.input) + u128::from(group.tokens.output)
    }).sum();
    let local_date = |ms| {
        i64::try_from(ms).ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|date| date.with_timezone(&chrono::Local).date_naive())
            .filter(|date| *date <= today)
    };
    let observed = report.first_observed_unix_ms.and_then(local_date);
    let created = project_created_unix_ms.filter(|ms| *ms != 0).and_then(local_date);
    let start = observed.or(created);
    let since_label = if observed.is_none() && created.is_some() {
        crate::tr!("Berechnet seit Projektanlage")
    } else {
        crate::tr!("Erfasst seit")
    };
    let average = start.map(|start| {
        let days = (today - start).num_days() as u128 + 1;
        (total + days / 2) / days
    });
    for (label, value) in [
        (crate::tr!("Tokens insgesamt"), german_count(total)),
        (since_label, start.map_or_else(|| crate::tr!("Unbekannt").into(), |date| date.format("%d.%m.%Y").to_string())),
        (crate::tr!("Durchschnitt pro Tag"), average.map_or_else(|| crate::tr!("Nicht verfügbar").into(), |count| format!("{} Tokens", german_count(count)))),
    ] {
        design::settings_field(ui, label, |ui| { ui.strong(value); });
    }
}

fn german_count(value: u128) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.bytes().enumerate() {
        if index != 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push('.');
        }
        formatted.push(char::from(digit));
    }
    formatted
}

fn history_line(ui: &mut egui::Ui, at: u64, message: impl Into<egui::RichText>, color: egui::Color32) {
    ui.horizontal_top(|ui| {
        ui.colored_label(MUTED, timestamp(at));
        ui.add(egui::Label::new(message.into().color(color)).wrap());
    });
}

impl TaskDialog {
    pub(crate) fn refocus_prompt(&mut self) {
        self.focused = false;
    }

    fn is_transferring(&self, app: &App) -> bool {
        !self.read_only && self.request.is_some_and(|id| app.in_flight.contains(&id))
    }

    fn restore_submission(&mut self, app: &App) {
        if self.read_only || self.request.is_some() || self.session_prompt || self.local_task.is_some() {
            return;
        }
        self.request = app.settings.pending.iter().find(|r| {
            r.server == app.server
                && crate::model_settings::pending_task(&r.path, &r.body).is_some_and(|task|
                    task["project_id"] == json!(self.draft.project) && task["id"] == json!(self.draft.ticket)
                        && !app.settings.offline_tasks.iter().any(|local| task["request_id"] == json!(local.id))
                        && (self.draft.ticket.is_some() || task["prompt"] == json!(self.draft.prompt)
                            && task["images"] == json!(self.draft.images)))
        }).map(|r| r.id);
    }

    fn has_prompt_content(&self) -> bool {
        !self.draft.prompt.trim().is_empty()
            || !self.draft.images.is_empty()
            || !self.draft.files.is_empty()
    }

    pub(crate) fn offline(draft: TaskDraft, id: RequestId) -> Box<Self> {
        let Dialog::Task(mut task) = Dialog::task(draft.server.clone(), draft.project, None) else { unreachable!() };
        task.draft = draft;
        task.local_task = Some(id);
        task
    }

    fn prepare_offline(&mut self, app: &mut App, action: TaskSubmitAction) {
        if self.draft.ticket.is_some() {
            app.notice(crate::tr!("Bestehende Serveraufgaben können erst nach Wiederverbindung geändert werden."));
            return;
        }
        if !self.has_prompt_content() {
            self.error = Some(crate::tr!("Bitte Text oder Anhänge angeben.").into());
            return;
        }
        if let Err(error) = validate_images(&self.draft.images).and_then(|()| validate_files(&self.draft.files)) {
            self.error = Some(error.into());
            return;
        }
        match app.save_offline_task(self.local_task, self.draft.clone()) {
            Ok(id) => {
                self.local_task = Some(id);
                self.local_done = true;
                self.submit_action = action;
                self.error = None;
                self.dirty = false;
                if app.online { app.sync_offline_tasks(false); }
            }
            Err(error) => app.project_notice(crate::app::ToastKind::Error, Some(self.draft.project), error),
        }
    }
    pub(crate) fn session(app: &App, project: ProjectId, root: CommandId) -> Box<Self> {
        let Dialog::Task(mut composer) = Dialog::task(app.server.clone(), project, None) else { unreachable!() };
        composer.session_prompt = true;
        composer.draft.requested_session = Some(root);
        if let Some(saved) = app.settings.session_prompts.iter().find(|saved|
            saved.draft.server == app.server && saved.draft.requested_session == Some(root))
        {
            composer.id = saved.id;
            composer.draft = saved.draft.clone();
        }
        composer
    }

    pub(crate) fn show_session_prompt(&mut self, ui: &mut egui::Ui, app: &mut App) -> Option<u64> {
        self.show_prompt_in_session(ui, app, None)
    }

    pub(crate) fn show_prompt_in_session(
        &mut self, ui: &mut egui::Ui, app: &mut App,
        direct: Option<(bool, Option<factory_protocol::directs::DirectSelection>)>,
    ) -> Option<u64> {
        let root = self.draft.requested_session.expect("Session composer target");
        let incompatible = direct.is_some() && app.snapshot.as_deref()
            .is_some_and(|s| !server_supports(s, factory_protocol::directs::CAPABILITY));
        let path = if direct.is_some() { format!("/v1/directs/{root}/prompt") } else { format!("/v1/commands/{root}/prompt") };
        if self.request.is_none() {
            self.request = app.settings.pending.iter().find(|r|
                r.server == self.draft.server && r.path == path && r.body["request_id"] == json!(self.id)).map(|r| r.id);
        }
        if let Some(id) = self.request {
            match app.responses.get(&id) {
                Some(Ok(_)) => {
                    self.draft.prompt.clear();
                    self.draft.images.clear();
                    self.draft.files.clear();
                    self.image_keys.clear();
                    self.id = RequestId::new();
                    self.request = None;
                    self.focused = false;
                    self.dirty = true;
                }
                Some(Err(error)) if !error.uncertain => {
                    self.request = None;
                    self.id = RequestId::new();
                    self.dirty = true;
                }
                _ => {},
            }
        }
        let editor_height = (ui.ctx().content_rect().height() * 0.35).clamp(100.0, 280.0);
        let submit_from_prompt = egui::ScrollArea::vertical().id_salt(("session-prompt-editor", root))
            .max_height(editor_height)
            // Measure against the editor budget, not the bottom panel's cached
            // height. Auto-shrink still fits short prompts to their content.
            .min_scrolled_height(editor_height)
            .auto_shrink([false, true])
            .show(ui, |ui| self.show_body(ui, app)).inner;
        let mut submitted = None;
        ui.horizontal(|ui| {
            let editable = self.request.is_none();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                 let submit = design::server_compatibility_tooltip(
                     design::accent_button(ui, egui::Button::new(crate::tr!("Prompten")),
                         !incompatible && app.online && editable && direct.as_ref().is_none_or(|(ready, _)| *ready), BLUE.gamma_multiply(0.7)),
                     incompatible,
                 );
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-session-prompt-submit", root)), submit.rect));
                if submit.clicked() || (submit.enabled() && submit_from_prompt && app.dialog.is_none()) {
                    if !self.has_prompt_content() {
                        self.error = Some(crate::tr!("Bitte Text oder Anhänge angeben.").into());
                    } else if let Err(error) = validate_images(&self.draft.images).and_then(|()| validate_files(&self.draft.files)) {
                        self.error = Some(error.into());
                    } else {
                        self.error = None;
                        self.persist(app);
                         let body = if let Some((_, selection)) = &direct {
                             json!(factory_protocol::directs::PromptDirect {
                                 request_id: self.id, prompt: self.draft.prompt.clone(),
                                 selection: selection.clone(), images: self.draft.images.clone(), files: self.draft.files.clone(),
                             })
                         } else { json!(PromptSession {
                             request_id: self.id, prompt: self.draft.prompt.clone(),
                             images: self.draft.images.clone(), files: self.draft.files.clone(),
                         }) };
                         self.request = Some(app.write(crate::tr!("Session prompten"), "POST", path.clone(), body));
                        submitted = self.request;
                        if let Some(tab) = app.tabs.iter_mut().find(|tab| tab.root == root) {
                            tab.selected_execution = None;
                            tab.scroll_to_start = false;
                            tab.follow = true;
                        }
                        self.dirty = true;
                    }
                }
                if let Some(id) = self.request {
                    if app.responses.get(&id).is_some_and(|r| r.as_ref().is_err_and(|e| e.uncertain))
                        || (!app.in_flight.contains(&id) && !app.responses.contains_key(&id)) {
                         if design::server_compatibility_tooltip(
                             ui.add_enabled(!incompatible, egui::Button::new(crate::tr!("Zustellung prüfen"))), incompatible,
                         ).clicked() { app.retry(id); }
                    } else { ui.spinner(); }
                }
            });
        });
        if self.dirty {
            self.persist(app);
            self.dirty = false;
        }
        submitted
    }

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

    fn show_history(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        let Some(ticket) = self.draft.ticket else { return false };
        if self.draft.server != app.server {
            return false;
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
        design::section_heading(ui, crate::tr!("Verlauf"));
        ui.scope(|ui| {
            let mut font = egui::TextStyle::Body.resolve(ui.style());
            font.size += 1.0;
            ui.style_mut().text_styles.insert(egui::TextStyle::Body, font);
            self.show_history_entries(ui, app, ticket)
        }).inner
    }

    fn show_history_entries(&mut self, ui: &mut egui::Ui, app: &mut App, ticket: TicketId) -> bool {
        let snapshot = app.snapshot.clone();
        let intent = snapshot.as_ref().and_then(|s| s.tickets.iter().find(|t| t.id == ticket));
        let mut linked = std::collections::BTreeSet::new();
        if let Some((s, intent)) = snapshot.as_deref().zip(intent) {
            if self.read_only && intent.execution_command.is_none() {
                let status = task_display_status(s, intent);
                history_line(ui, intent.updated_unix_ms, status, crate::views::status_color(status));
            }
            if let Some(reason) = task_runner_blocker(s, intent) {
                design::routing_note_spacing(ui);
                history_line(ui, intent.updated_unix_ms, reason, AMBER);
            }
            // The current execution and queued/running review should be reachable
            // even before the paginated journal has returned.
            if self.read_only {
                for c in s.commands.iter().rev().filter(|c| {
                    Some(c.id) == intent.execution_command
                        || (matches!(c.state, CommandState::Pending | CommandState::Accepted)
                            && c.agent_spec.as_ref().is_some_and(|a| a.review_of == Some(ticket)))
                }) {
                    if self.history.entries.iter().any(|entry| entry.command_id == Some(c.id)) {
                        continue;
                    }
                    linked.insert(c.id);
                    if self.show_intent_run(ui, app, c, None) {
                        return true;
                    }
                }
            }
        }
        for entry in &self.history.entries {
            let open = ui.push_id(entry.sequence, |ui| {
                if self.read_only {
                    if let Some(c) = entry.command_id.filter(|id| linked.insert(*id))
                        .and_then(|id| snapshot.as_ref()?.commands.iter().find(|c| c.id == id))
                    {
                        return self.show_intent_run(ui, app, c, Some(entry));
                    }
                }
                let level = if entry.level == TaskHistoryLevel::Info {
                    task_routing_level(&entry.message)
                } else {
                    entry.level
                };
                history_line(ui, entry.created_unix_ms, crate::i18n::label(&entry.message),
                    design::history_color(level, ui.visuals().text_color()));
                false
            });
            if open.inner { return true; }
        }
        if self.history.entries.is_empty() {
            if let Some(ticket) = app.snapshot.as_ref().and_then(|s| s.tickets.iter().find(|t| t.id == ticket))
                .filter(|t| !t.routing_note.is_empty())
            {
                design::routing_note_spacing(ui);
                history_line(ui, ticket.updated_unix_ms, &ticket.routing_note,
                    design::history_color(task_routing_level(&ticket.routing_note), ui.visuals().text_color()),
                );
            } else if app.online && !self.history.loading && self.history.error.is_none() {
                ui.colored_label(MUTED, crate::tr!("Noch keine Einträge."));
            }
        }
        if !app.online {
            return false;
        }
        if self.history.loading {
            ui.label(crate::tr!("Verlauf wird geladen …"));
        } else if self.history.error.is_some() {
            if ui.button(crate::tr!("Verlauf erneut laden")).clicked() {
                self.history.error = None;
                self.history.cursor = None;
            }
        } else if let Some(before) = self.history.next_before {
            if ui.button(crate::tr!("Ältere Einträge laden")).clicked() {
                self.history.loading = true;
                app.backend.task_history(self.id, ticket, Some(before));
            }
        }
        false
    }

    fn show_intent_run(&self, ui: &mut egui::Ui, app: &mut App, c: &RunnerCommand, entry: Option<&TaskHistoryEntry>) -> bool {
        ui.push_id(c.id, |ui| {
            ui.horizontal_top(|ui| {
                ui.colored_label(MUTED, timestamp(entry.map_or(c.created_unix_ms, |entry| entry.created_unix_ms)));
                ui.strong(intent_run_role(c));
                let status = command_status(c);
                let (summary, level) = intent_run_summary(c, entry);
                let color = level.map_or_else(|| crate::views::status_color(status),
                    |level| design::history_color(level, ui.visuals().text_color()));
                let link = (app.online && c.kind == CommandKind::RunAgent)
                    .then(|| crate::tr!("OpenCode anzeigen"));
                let font = egui::TextStyle::Body.resolve(ui.style());
                let link_space = link.map_or(0.0, |text| {
                    ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.into(), font.clone(), color).size().x)
                        + ui.spacing().item_spacing.x
                });
                let job = design::elided_text_job(
                    design::elide_first_line(&summary, 88),
                    font, color, (ui.available_width() - link_space).max(0.0), 1,
                );
                let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                ui.label(galley).on_hover_text(summary);
                if let Some(link) = link {
                    let open = ui.link(link);
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-intent-run-open", c.id)), open.rect));
                    if open.clicked() {
                        app.open_intent_execution(self.draft.project, c.session_root(), c.id);
                        return true;
                    }
                }
                false
            }).inner
        }).inner
    }

    fn show_read_only_body(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
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
                    egui::Button::new(crate::tr!("Anhänge erneut laden")),
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
                        crate::tr!("{} · vollständige Anhangdaten noch nicht verfügbar", image.mime)
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
                        crate::tr!("{} · vollständige Anhangdaten noch nicht verfügbar", file.mime)
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
        self.show_run_assignment(ui, app);
        self.show_history(ui, app)
    }

    fn show_run_assignment(&mut self, ui: &mut egui::Ui, app: &mut App) {
        let Some(id) = self.draft.ticket else { return };
        if app.models.managed() && app.online && self.draft.server == app.server {
            self.model.load(app, &format!("/v1/tasks/{id}/models"));
        }
        let snapshot = app.snapshot.as_deref();
        let ticket = snapshot.and_then(|s| s.tickets.iter().find(|t| t.id == id));
        let command = snapshot.zip(ticket).and_then(|(s, t)| task_command(s, t));
        let model = self.model.value.as_ref().and_then(Option::as_ref)
            .and_then(|models| models.implementer.clone())
            .or_else(|| {
                let profile = &command?.agent_spec.as_ref()?.profile;
                (!profile.model_id.is_empty()).then(|| factory_protocol::models::ModelSelection {
                    access_id: profile.provider_id.clone(),
                    model_id: profile.model_id.clone(),
                })
            });
        if let Some(model) = model {
            let accesses = app.models.factory.value.as_ref().map_or(&[][..], |catalog| catalog.accesses.as_slice());
            let text = crate::tr!("Modell: {}", crate::model_settings::label(accesses, &model));
            ui.add(egui::Label::new(&text).truncate()).on_hover_text(text);
        }
        let assigned = command.or_else(|| {
            let root = ticket?.requested_session?;
            snapshot?.commands.iter().find(|c| c.id == root)
        });
        let runner = assigned.map_or("Auto Runner", |c| {
            snapshot.and_then(|s| s.runners.iter().find(|r| r.id == c.runner_id))
                .map_or(crate::tr!("Runner nicht verfügbar"), |r| r.name.as_str())
        });
        let text = crate::tr!("Zuordnung: {title}", title = runner);
        let detail = snapshot.zip(assigned)
            .map_or_else(|| text.clone(), |(s, c)| format!("{text}\n{}", session_title(s, c)));
        ui.add(egui::Label::new(text).truncate()).on_hover_text(detail);
    }

    fn show_layout(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut App,
        height: f32,
        modal_id: egui::Id,
    ) -> bool {
        let expanded = !self.read_only && self.advanced;
        let rows = if expanded { 2.0 } else { 1.0 };
        let footer_height = rows * ui.spacing().interact_size.y + (rows + 1.0) * ui.spacing().item_spacing.y
            + if expanded { 44.0 } else { 20.0 };
        let body = egui::ScrollArea::vertical()
            .id_salt((modal_id, "body"))
            .max_height((height - footer_height).max(40.0))
            .auto_shrink([false, true])
            .show(ui, |ui| self.show_body(ui, app));
        ui.separator();
        let close = if self.read_only {
            body.inner || ui.horizontal(|ui| self.show_footer(ui, app, false)).inner
        } else {
            self.show_footer(ui, app, body.inner)
        };
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                modal_id.with("test-body-scroll"),
                (
                    egui::Rect::from_min_size(
                        body.inner_rect.min,
                        egui::vec2(
                            body.inner_rect.width(),
                            body.inner_rect.height().min(body.content_size.y),
                        ),
                    ),
                    body.content_size,
                ),
            )
        });
        close
    }
    pub(crate) fn persist(&self, app: &mut App) {
        if self.read_only {
            return;
        }
        if let Some(id) = self.local_task {
            if !self.local_done {
                if let Some(task) = app.settings.offline_tasks.iter_mut().find(|t| t.id == id) {
                    task.edit = Some(self.draft.clone());
                    app.settings_dirty = true;
                }
            }
            return;
        }
        if self.session_prompt {
            app.settings.session_prompts.retain(|saved|
                saved.draft.server != self.draft.server || saved.draft.requested_session != self.draft.requested_session);
            if self.request.is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)) {
                app.settings_dirty = true;
                return;
            }
            if !self.draft.prompt.is_empty() || !self.draft.images.is_empty() || !self.draft.files.is_empty() || self.request.is_some() {
                app.settings.session_prompts.push(crate::persistence::SessionPromptDraft {
                    id: self.id, draft: self.draft.clone(),
                });
            }
            app.settings_dirty = true;
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
             || self.draft.local_mcps.is_some()
            || (self.draft.edit_token.is_some()
                && !self
                    .request
                    .is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)))
        {
            app.settings.task_drafts.push(self.draft.clone());
        }
        app.settings_dirty = true;
    }
    fn show_body(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        ui.spacing_mut().item_spacing.y = 6.0;
        if !self.read_only && !self.session_prompt && app.online {
            self.load_model(app);
        }
        if self.read_only {
            return self.show_read_only_body(ui, app);
        }
        if self.draft.edit_token.is_some() && self.request.is_none() {
            if let Some(current) = app
                .snapshot
                .as_ref()
                .and_then(|s| s.tickets.iter().find(|t| Some(t.id) == self.draft.ticket))
                .filter(|t| t.revision != self.draft.revision)
            {
                ui.colored_label(AMBER,crate::tr!("Der Serverstand hat sich geändert. Eigene Eingaben bleiben erhalten; vor dem Speichern vergleichen."));
                ui.collapsing(crate::tr!("Serverstand vergleichen"), |ui| {
                    ui.add(
                        egui::Label::new(current.prompt.as_deref().unwrap_or(&current.description))
                            .wrap(),
                    );
                    if ui
                        .button(crate::tr!("Verglichenen Stand als Basis übernehmen"))
                        .clicked()
                    {
                        self.draft.revision = current.revision;
                        self.dirty = true;
                        self.error = None;
                    }
                });
            }
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
        let submit_from_prompt = ui.add_enabled_ui(!locked, |ui| {
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
            let (prompt, submit) = design::task_prompt(ui, &mut self.draft.prompt, 3);
            if !self.focused && prompt.enabled() && ui.is_visible() && !ui.is_sizing_pass()
                && (!self.session_prompt || app.dialog.is_none())
                && ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
            {
                prompt.request_focus();
                self.focused = true;
            }
            if prompt.changed() {
                self.dirty = true;
            }
            submit
        }).inner;
        if !self.session_prompt { self.show_history(ui, app); }
        if let Some(error) = &self.error {
            ui.colored_label(RED, error);
        }
        if let Some(id) = self.request.filter(|id| !self.session_prompt && !app.in_flight.contains(id)) {
            request_status(ui, app, id);
            if app
                .responses
                .get(&id)
                .is_some_and(|v| v.as_ref().is_err_and(|e| !e.uncertain))
                && ui.button(crate::tr!("Entwurf korrigieren")).clicked()
            {
                self.request = None;
            }
        }
        submit_from_prompt
    }
    fn load_model(&mut self, app: &mut App) {
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
    }
    fn show_model(&mut self, ui: &mut egui::Ui, app: &mut App) {
        if !app.models.managed() { return; }
        if let Some(error) = &self.model.error {
            ui.colored_label(RED, error);
            if let Some(ticket) = self.draft.ticket {
                if ui.button(crate::tr!("Modellauswahl erneut laden")).clicked() && app.online {
                    self.model.fetch(app, &format!("/v1/tasks/{ticket}/models"));
                }
            }
        }
        let Some(catalog) = app.models.factory.value.as_ref() else { return; };
        let default_label = crate::model_settings::effective_implementer(app, self.draft.project)
            .map_or_else(|| "Default".into(), |model| format!("Default ({})",
                crate::model_settings::label(&catalog.accesses, model)));
        let available = app.models.ready_catalog();
        ui.add_enabled_ui(self.draft.ticket.is_none() || self.draft.model_loaded, |ui| {
            if crate::model_settings::choice(ui, ("task-implementer", self.id), &available,
                &mut self.draft.implementer, &default_label)
            {
                self.dirty = true;
                self.draft.model_loaded = true;
            }
        });
    }

    fn submit(&mut self, app: &mut App, action: TaskSubmitAction) {
        if !app.online || self.local_task.is_some() {
            self.prepare_offline(app, action);
            return;
        }
        if app.models.managed() && self.draft.ticket.is_some() && !self.draft.model_loaded {
            self.error = Some(crate::tr!("Bitte die gespeicherte Modellauswahl abwarten.").into());
            return;
        }
        let save_draft = matches!(action, TaskSubmitAction::SaveDraft | TaskSubmitAction::SaveDraftAndNew);
        if app.models.needs_setup() && !save_draft {
            self.error = Some(crate::tr!("OpenCode zuerst in den Factory-Einstellungen einrichten.").into());
            return;
        }
        if !save_draft && app.models.managed() {
            let effective = self.draft.implementer.as_ref()
                .or_else(|| crate::model_settings::effective_implementer(app, self.draft.project));
            if let Some((root, model)) = self.draft.requested_session.zip(effective) {
                if app.snapshot.as_ref().and_then(|s| s.commands.iter().find(|c| c.id == root))
                    .and_then(|c| c.agent_spec.as_ref()).is_some_and(|agent|
                        agent.profile.provider_id != model.access_id || agent.profile.model_id != model.model_id)
                {
                    self.error = Some(crate::tr!("Die ausgewählte Session nutzt einen anderen Zugang oder ein anderes Modell. Bitte Auto Runner wählen, damit eine neue Session möglich ist.").into());
                    return;
                }
            }
        }
        if self.draft.requested_session.is_some_and(|root| {
            app.snapshot.as_ref().is_none_or(|snapshot| {
                !assignable_sessions(snapshot, self.draft.project).iter().any(|c| c.session_root() == root)
            })
        }) {
            self.error = Some(crate::tr!("Die zugeordnete Session ist nicht mehr verfügbar. Bitte eine offene Session oder Auto Runner wählen.").into());
            return;
        }
        if !self.has_prompt_content() {
            self.error = Some(crate::tr!("Bitte Text oder Anhänge angeben.").into());
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
            draft: save_draft,
            files: self.draft.files.clone(),
        };
        let label = if body.draft { crate::tr!("Entwurf speichern") } else { crate::tr!("Aufgabe einreihen") };
        let (path, value) = if app.models.managed() || self.draft.implementer.is_some() || self.draft.local_mcps.is_some() {
            ("/v1/tasks/submit-with-model", json!(factory_protocol::models::SubmitTaskWithModel {
                submission: body, implementer: self.draft.implementer.clone(),
                local_mcps: self.draft.local_mcps.clone(),
            }))
        } else { ("/v1/tasks/submit", json!(body)) };
        self.request = Some(app.write(label, "POST", path.into(), value));
    }

    fn show_footer(&mut self, ui: &mut egui::Ui, app: &mut App, submit_from_prompt: bool) -> bool {
        if self.read_only {
            let execution = app.snapshot.as_ref().and_then(|s| {
                let ticket = s.tickets.iter().find(|t| Some(t.id) == self.draft.ticket)?;
                task_session_command(s, ticket).map(|c| (c.session_root(), c.id))
            });
            return ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut close = ui.button(crate::tr!("Schließen")).clicked();
                    if let Some(ticket) = app.snapshot.as_ref().and_then(|s|
                        s.tickets.iter().find(|t| Some(t.id) == self.draft.ticket && retryable_task(s, t))).cloned()
                    {
                        let path = format!("/v1/tasks/{}/retry", ticket.id);
                        let pending = app.settings.pending.iter().any(|r| r.server == app.server && r.path == path);
                        let retry = ui.add_enabled(app.online && !pending, egui::Button::new(crate::tr!("Erneut einreihen")));
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-retry", ticket.id)), retry.rect));
                        if retry.clicked() { app.requeue_task(&ticket); }
                    }
                    if let Some((root, id)) = execution {
                        let open = ui.button(crate::tr!("OpenCode anzeigen"));
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(("test-task-dialog-opencode", self.id)), open.rect));
                        if open.clicked() {
                            app.open_intent_execution(self.draft.project, root, id);
                            close = true;
                        }
                    }
                    close
                })
                .inner;
        }
        ui.add_space(8.0);
        if self.advanced {
        ui.horizontal(|ui| {
            let editable=self.request.is_none();
            if app.online && app.models.managed() {
                let model_width = (ui.available_width() - 292.0).clamp(220.0, 420.0);
                ui.add_enabled_ui(editable, |ui| {
                    let _model_control = ui.scope(|ui| {
                        ui.set_width(model_width);
                        self.show_model(ui, app);
                    });
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-model", self.id)), _model_control.response.rect));
                });
            }
            if app.online && self.local_task.is_none() {
            let choices=app.snapshot.as_ref().map(|s|assignable_sessions(s,self.draft.project)).unwrap_or_default();
            let selected=match self.draft.requested_session {
                None=>"Auto Runner".to_string(),
                Some(root)=>choices.iter().find(|c|c.session_root()==root).and_then(|c|app.snapshot.as_ref().map(|s|session_title(s,c))).unwrap_or_else(||crate::tr!("Nicht verfügbar").into()),
            };
             let width=156.0;
            ui.add_enabled_ui(editable,|ui| {
                let combo=egui::ComboBox::from_id_salt(("task-target",self.id)).selected_text(&selected).width(width)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .height(320.0)
                    .show_ui(ui,|ui| {
                        ui.set_width((ui.ctx().content_rect().width()-64.0).clamp(160.0,296.0));
                        ui.spacing_mut().item_spacing.y=2.0;
                        if design::session_choice(ui,"Auto Runner",None,self.draft.requested_session.is_none()).on_hover_text(crate::tr!("Automatische Zuordnung durch den Orchestrator")).clicked() {
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
                    }).response.on_hover_text(crate::tr!("Zuordnung: {selected}", selected = selected))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                #[cfg(test)]
                ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new(("test-task-target",self.id)),combo.rect));
                #[cfg(not(test))]
                let _=combo;
            });
            }
             let attachments = design::icon_button(ui,egui_phosphor::regular::PAPERCLIP,crate::tr!("Bilder und Dateien hinzufügen"),editable);
             #[cfg(test)]
             ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-attachments", self.id)), attachments.rect));
             if attachments.clicked() {app.backend.pick_attachments(self.id);}
              if app.online {
              ui.add_enabled_ui(editable, |ui| {
                 let (button, _) = egui::containers::menu::MenuButton::new("MCPs")
                     .config(egui::containers::menu::MenuConfig::new()
                         .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                     .ui(ui, |ui| {
                         ui.set_min_width(220.0);
                         let changed = egui::ScrollArea::vertical()
                             .max_height((ui.ctx().content_rect().height() - 128.0).clamp(80.0, 320.0))
                             .show(ui, |ui| crate::mcp_settings::task_choices(ui, app, self.draft.project, &mut self.draft.local_mcps)).inner;
                         if changed {
                             self.dirty = true;
                         }
                      });
                 #[cfg(test)]
                 ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-mcps", self.id)), button.rect));
                   button.on_hover_text(crate::tr!("MCPs für diese Aufgabe auswählen"))
                       .on_hover_cursor(egui::CursorIcon::PointingHand);
               });
              }
        });
        ui.add_space(8.0);
        let _separator = ui.separator();
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-footer-divider", self.id)), _separator.rect));
        ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            let advanced = design::icon_button(ui, egui_phosphor::regular::SLIDERS_HORIZONTAL,
                if self.advanced { crate::tr!("Erweiterte Optionen ausblenden") } else { crate::tr!("Erweiterte Optionen anzeigen") }, true);
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-advanced", self.id)), advanced.rect));
            if advanced.clicked() { self.advanced = !self.advanced; }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let local = !app.online || self.local_task.is_some();
            let enabled = (app.online || self.draft.ticket.is_none()) && self.request.is_none();
            let keep = app.settings.task_keep_open;
            let draft = !local && app.settings.task_as_draft;
            let label = match (local, draft, keep) {
                (true, _, false) => crate::tr!("Lokal vorbereiten"),
                (true, _, true) => crate::tr!("Vorbereiten & neu"),
                (false, true, false) => crate::tr!("Entwurf"),
                (false, true, true) => crate::tr!("Entwurf & neu"),
                (false, false, false) => crate::tr!("Einreihen"),
                (false, false, true) => crate::tr!("Einreihen & neu"),
            };
            let submit = design::accent_button(ui, egui::Button::new(label).min_size(egui::vec2(200.0, 36.0)), enabled, BLUE.gamma_multiply(0.7));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-action", self.id, false)), submit.rect));
            if submit.clicked() || (submit.enabled() && submit_from_prompt) {
                self.submit(app, match (draft, keep) {
                    (false, false) => TaskSubmitAction::Queue,
                    (false, true) => TaskSubmitAction::QueueAndNew,
                    (true, false) => TaskSubmitAction::SaveDraft,
                    (true, true) => TaskSubmitAction::SaveDraftAndNew,
                });
            }
            ui.add_enabled_ui(enabled && self.request.is_none(), |ui| {
                if !local {
                    let draft = ui.checkbox(&mut app.settings.task_as_draft, crate::tr!("Entwurf"));
                    if draft.changed() { app.settings_dirty = true; }
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-as-draft", self.id)), draft.rect));
                }
                let keep = ui.checkbox(&mut app.settings.task_keep_open, crate::tr!("Nicht schließen"));
                if keep.changed() { app.settings_dirty = true; }
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-keep-open", self.id)), keep.rect));
            });
            });
        });
        false
    }
}

pub(crate) fn answer_text(value: &Value, schema: Option<&Value>) -> String {
    if let Some(label) = schema
        .and_then(|s| s["options"].as_array())
        .and_then(|options| options.iter().find(|option| option["value"] == *value))
        .and_then(|option| option["label"].as_str())
    {
        return label.to_string();
    }
    match value {
        Value::Null => crate::tr!("Noch nicht beantwortet.").into(),
        Value::String(text) => text.clone(),
        Value::Bool(value) => if *value { crate::tr!("Ja") } else { crate::tr!("Nein") }.into(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => values.iter().map(|v| answer_text(v, schema)).collect::<Vec<_>>().join("\n"),
        Value::Object(values) => values.iter()
            .map(|(key, value)| format!("{key}: {}", answer_text(value, None)))
            .collect::<Vec<_>>().join("\n"),
    }
}

pub(crate) fn question_answer_text(question: &Question, field: Option<&Value>) -> String {
    let answer = question.answer.as_ref().and_then(|answer| {
        field.map_or(Some(answer), |field| field["key"].as_str().and_then(|key| answer.get(key)))
    });
    if question.external.kind == QuestionKind::Permission {
        match answer.and_then(|answer| answer["decision"].as_str()) {
            Some("once") => crate::tr!("Einmal erlaubt").into(),
            Some("always") => crate::tr!("Dauerhaft erlaubt").into(),
            Some("reject") => crate::tr!("Abgelehnt").into(),
            _ => answer.map_or_else(|| crate::tr!("Noch nicht beantwortet.").into(), |v| answer_text(v, field)),
        }
    } else {
        answer.map_or_else(|| crate::tr!("Noch nicht beantwortet.").into(), |v| answer_text(v, field))
    }
}

fn permission_details(ui: &mut egui::Ui, question: &Question) {
    if question.external.kind != QuestionKind::Permission {
        return;
    }
    // Keep the requested operation visible without exposing the raw payload.
    for (key, label) in [
        ("action", crate::tr!("Aktion")), ("permission", crate::tr!("Berechtigung")),
        ("resource", crate::tr!("Ziel")), ("patterns", crate::tr!("Geltungsbereich")), ("command", crate::tr!("Befehl")),
    ] {
        if let Some(value) = question.external.payload.get(key).filter(|v| !v.is_null()) {
            read_text(ui, &format!("{label}: {}", answer_text(value, None)), false);
        }
    }
    if let Some(command) = question.external.payload["metadata"]["command"].as_str() {
        read_text(ui, &crate::tr!("Befehl: {command}", command = command), false);
    }
}

fn question_pairs(question: &Question) -> impl Iterator<Item = (&Question, Option<&Value>)> {
    let fields: Vec<_> = question.external.payload["fields"].as_array().into_iter().flatten()
        .filter(|field| field["hidden"] != true).map(Some).collect();
    let fields = if fields.is_empty() { vec![None] } else { fields };
    fields.into_iter().map(move |field| (question, field))
}

/// Human-readable question and answer only: no raw payloads or export controls.
fn question_pair(ui: &mut egui::Ui, question: &Question, field: Option<&Value>) {
    let title = question.external.payload["title"].as_str().unwrap_or(
        if question.external.kind == QuestionKind::Permission { crate::tr!("Berechtigung") } else { crate::tr!("Rückfrage") },
    );
    design::question_heading(ui, crate::tr!("Frage"));
    // Prefer the actual field question over generic container titles such as "Questions".
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
    design::question_heading(ui, crate::tr!("Antwort"));
    let text = question_answer_text(question, field);
    read_text(ui, &text, false);
}

fn show_question_pages<'a>(
    ui: &mut egui::Ui, app: &mut App, questions: impl Iterator<Item = &'a Question>,
    page: &mut usize, height: f32,
) {
    let pairs: Vec<_> = questions.flat_map(question_pairs).collect();
    *page = (*page).min(pairs.len().saturating_sub(1));
    let Some(&(question, field)) = pairs.get(*page) else { return };
    let multiple = pairs.len() > 1;
    let intent = app.snapshot.as_deref().and_then(|s| question_intent(s, question))
        .map(|intent| (intent.id, intent.key.clone(), task_intent_text(intent)));
    let has_footer = multiple || intent.is_some();
    let content_height = if has_footer {
        (height - design::SETTINGS_FOOTER_HEIGHT - 24.0).max(20.0)
    } else {
        height
    };
    egui::ScrollArea::vertical()
        .id_salt(("question-view-page", question.id, *page))
        .max_height(if multiple { content_height.min(480.0) } else { content_height })
        .auto_shrink([false, !multiple])
        .show(ui, |ui| {
            if let Some((_, _, summary)) = &intent {
                if !summary.trim().is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(summary)
                        .color(MUTED)).truncate()).on_hover_text(summary);
                    ui.add_space(8.0);
                }
            }
            question_pair(ui, question, field);
        });
    if has_footer {
        ui.add_space(8.0);
        ui.separator();
        design::dialog_footer(ui, |ui| {
            if multiple {
                let next = ui.add_enabled(*page + 1 < pairs.len(), egui::Button::new(crate::tr!("Weiter")));
                let back = ui.add_enabled(*page > 0, egui::Button::new(crate::tr!("Zurück")));
                ui.colored_label(MUTED, crate::tr!("Frage {current} von {total}", current = *page + 1, total = pairs.len()));
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new("test-question-view-next"), next.rect);
                    d.insert_temp(egui::Id::new("test-question-view-back"), back.rect);
                });
                if next.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    *page += 1;
                } else if back.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    *page -= 1;
                }
            }
            if let Some((id, key, _)) = intent {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    question_intent_link(ui, app, id, &key);
                });
            }
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-question-view-footer"), ui.max_rect()));
        });
    }
}

fn question_intent_link(ui: &mut egui::Ui, app: &mut App, id: TicketId, key: &str) {
    let label = if key.is_empty() { crate::tr!("Intent anzeigen").into() }
        else { crate::tr!("Intent {key} anzeigen", key = key) };
    let button = ui.button(label).on_hover_cursor(egui::CursorIcon::PointingHand);
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-question-intent", id)), button.rect));
    if button.clicked() {
        app.view_task(id);
    }
}

impl QuestionDialog {
    fn is_transferring(&self, app: &App) -> bool {
        self.questions.iter().any(|entry| needs_answer(&entry.question)
            && entry.request.or_else(|| entry.pending_request(app))
                .is_some_and(|id| app.in_flight.contains(&id)))
    }

    fn steps(&self) -> Vec<(usize, Option<usize>)> {
        let mut steps = Vec::new();
        for (index, entry) in self.questions.iter().enumerate() {
            if !needs_answer(&entry.question) {
                continue;
            }
            let start = steps.len();
            if entry.question.external.kind == QuestionKind::Form {
                if let Some(fields) = entry.question.external.payload["fields"].as_array() {
                    steps.extend(fields.iter().enumerate()
                        .filter(|(_, field)| field["hidden"] != true && field_enabled(field, &entry.values))
                        .map(|(field, _)| (index, Some(field))));
                }
            }
            if steps.len() == start {
                steps.push((index, None));
            }
        }
        steps
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32) -> bool {
        if let Some(snapshot) = &app.snapshot {
            for entry in &mut self.questions {
                if let Some(current) = snapshot.questions.iter().find(|current|
                    current.id == entry.question.id && current.update_seq.0 > entry.question.update_seq.0
                ) {
                    if !needs_answer(current) {
                        entry.question = current.clone();
                    } else if current.external == entry.question.external {
                        if !needs_answer(&entry.question) {
                            entry.request = None;
                        }
                        entry.question = current.clone();
                    }
                }
            }
        }
        if self.questions.iter().all(|entry| !needs_answer(&entry.question)) {
            if self.questions.iter().any(|entry| entry.request.is_some())
                && self.questions.iter().all(|entry| matches!(
                    entry.question.state, QuestionState::AnswerQueued | QuestionState::Answered))
            {
                return true;
            }
            show_question_pages(ui, app, self.questions.iter().map(|entry| &entry.question), &mut self.step, height);
            return false;
        }
        for entry in &mut self.questions {
            if !entry.restored {
                entry.restore_draft(app);
            }
            entry.request = entry.pending_request(app).or(entry.request);
            if entry.request.and_then(|id| app.responses.get(&id))
                .is_some_and(|result| result.as_ref().is_err_and(|error| !error.uncertain))
            {
                entry.request = None;
            }
        }
        let steps = self.steps();
        self.step = self.step.min(steps.len().saturating_sub(1));
        let (index, field) = steps[self.step];
        let entry = &mut self.questions[index];
        let question_id = entry.question.id;
        let height = if let Some((id, key)) = app.snapshot.as_deref()
            .and_then(|snapshot| question_intent(snapshot, &entry.question))
            .map(|intent| (intent.id, intent.key.clone()))
        {
             let top = ui.cursor().top();
             question_intent_link(ui, app, id, &key);
             ui.add_space(8.0);
            height - (ui.cursor().top() - top)
        } else { height };
        let mut submit_from_editor = false;
        egui::ScrollArea::vertical()
            .id_salt(("questions-dialog-scroll", question_id, field))
            .max_height((height - design::SETTINGS_FOOTER_HEIGHT - 26.0).max(20.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                    if steps.len() > 1 {
                        ui.colored_label(MUTED, crate::tr!("Frage {current} von {total}", current = self.step + 1, total = steps.len()));
                    }
                    ui.push_id(entry.question.id, |ui| {
                        ui.vertical(|ui| {
                            let q = &entry.question;
                             let title = q.external.payload["title"].as_str().unwrap_or(
                                if q.external.kind == QuestionKind::Permission {
                                    crate::tr!("Berechtigung")
                                } else {
                                    crate::tr!("Rückfrage")
                                },
                             );
                             if field.is_none() {
                                 design::question_heading(ui, title);
                                 if let Some(text) = q.external.payload["description"].as_str() {
                                     read_text(ui, text, false);
                                 }
                             }
                            permission_details(ui, q);
                            if let Some(id) = entry.request {
                                if app.in_flight.contains(&id) {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(crate::tr!("Wird übertragen …"));
                                    });
                                }
                            }
                             ui.add_enabled_ui(entry.request.is_none(), |ui| {
                                if q.external.kind == QuestionKind::Permission {
                                    permission_options(ui, &mut entry.decision);
                                 } else if let Some(schema) = field.and_then(|field| q.external.payload["fields"].get(field))
                                 {
                                         let Some(key) = schema["key"].as_str() else {
                                             ui.colored_label(RED, crate::tr!("Feld ohne Schlüssel"));
                                             return;
                                        };
                                          submit_from_editor = ui.push_id(key, |ui| {
                                              question_field(
                                                  ui,
                                                  schema,
                                                  q.external.payload["description"].as_str(),
                                                  entry.values.entry(key.into()).or_insert(Value::Null),
                                              )
                                          }).inner;
                                 } else if q.external.payload["fields"].is_array() {
                                     if q.external.payload["fields"].as_array().is_some_and(|fields| fields.is_empty()) {
                                         ui.colored_label(RED, crate::tr!("Dieses Frageformat kann hier nicht beantwortet werden."));
                                     }
                                } else {
                                    ui.colored_label(
                                        RED,
                                        crate::tr!("Dieses Frageformat kann hier nicht beantwortet werden."),
                                    );
                                }
                            });
                        });
                    });
                if let Some(error) = &self.error {
                    ui.colored_label(RED, error);
                }
            });
        ui.add_space(12.0);
        ui.separator();
        let footer_bottom = ui.cursor().top() + design::SETTINGS_FOOTER_HEIGHT;
        let steps = self.steps();
        let current = steps.iter().position(|step| *step == (index, field))
            .unwrap_or_else(|| self.step.min(steps.len().saturating_sub(1)));
        self.step = current;
        let last = current + 1 >= steps.len();
        let submit = design::settings_footer(ui, footer_bottom, |ui| {
            let enabled = !last || (app.online && self.questions.iter().any(|q|
                q.request.is_none() && needs_answer(&q.question)));
            let button = design::accent_button(
                ui, egui::Button::new(if last { crate::tr!("Antwort abgeben") } else { crate::tr!("Weiter") }),
                enabled,
                BLUE.gamma_multiply(0.7),
            );
            let forward = button.clicked() || (enabled && submit_from_editor);
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-question-submit"), button.rect));
            let back = if current > 0 {
                let back = ui.button(crate::tr!("Zurück")).on_hover_cursor(egui::CursorIcon::PointingHand);
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-question-back"), back.rect));
                back.clicked()
            } else { false };
            if back {
                self.step -= 1;
                self.error = None;
            } else if !last && forward {
                self.step += 1;
                self.error = None;
            }
            last && forward && !back
        });
        if submit {
            let mut bodies = Vec::new();
            self.error = None;
            for (index, q) in self.questions.iter().enumerate().filter(|(_, q)| {
                q.request.is_none() && needs_answer(&q.question)
            }) {
                if !app.snapshot.as_ref().is_some_and(|s| s.questions.iter().any(|current|
                    current.id == q.question.id && current.external == q.question.external && needs_answer(current)))
                {
                    app.project_notice(crate::app::ToastKind::Warning, None, crate::tr!("Die Frage ist nicht mehr offen. Dein Entwurf bleibt erhalten."));
                    bodies.clear();
                    break;
                }
                match q.payload() {
                    Ok(body) => bodies.push((index, body)),
                    Err(e) => {
                        if let Some(step) = steps.iter().position(|(entry, _)| *entry == index) {
                            self.step = step;
                        }
                        self.error = Some(e);
                        break;
                    }
                }
            }
            if self.error.is_none() {
                for (index, body) in bodies {
                    let q = &mut self.questions[index];
                    q.send(app, body);
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
            entry.save_draft(app);
        }
        false
    }
}
impl QuestionAnswer {
    fn new(question: Question) -> Self {
        let values = question.external.payload["fields"].as_array().into_iter().flatten()
            .filter_map(|field| {
                let key = field["key"].as_str()?;
                let value = question.answer.as_ref().and_then(|answer| answer.get(key))
                    .or(field.get("default")).cloned().unwrap_or(Value::Null);
                Some((key.to_owned(), value))
            }).collect();
        Self { question, values, decision: String::new(), request: None, restored: false }
    }

    fn restore_draft(&mut self, app: &App) {
        if let Some(draft) = app.settings.question_drafts.iter()
            .find(|d| d.server == app.server && d.question == self.question.id)
        {
            self.values.clone_from(&draft.values);
            self.decision.clone_from(&draft.decision);
        }
        self.restored = true;
    }

    fn save_draft(&self, app: &mut App) {
        let drafts = &mut app.settings.question_drafts;
        if let Some(draft) = drafts.iter_mut()
            .find(|d| d.server == app.server && d.question == self.question.id)
        {
            if draft.values == self.values && draft.decision == self.decision {
                return;
            }
            draft.values.clone_from(&self.values);
            draft.decision.clone_from(&self.decision);
        } else {
            drafts.push(crate::persistence::QuestionDraft {
                server: app.server.clone(), question: self.question.id,
                values: self.values.clone(), decision: self.decision.clone(),
            });
        }
        app.settings_dirty = true;
    }

    fn path(&self) -> String {
        format!("/v1/questions/{}/answer", self.question.id)
    }

    fn pending_request(&self, app: &App) -> Option<u64> {
        let path = self.path();
        app.settings.pending.iter().find(|r| r.server == app.server && r.path == path).map(|r| r.id)
    }

    fn send(&mut self, app: &mut App, body: AnswerQuestion) {
        self.request = Some(app.write(crate::tr!("Frage beantworten"), "POST", self.path(), json!(body)));
    }

    fn payload(&self) -> Result<AnswerQuestion, String> {
        let q = &self.question;
        let answer = if q.external.kind == QuestionKind::Permission {
            if self.decision.is_empty() {
                return Err(crate::tr!("Bitte jede Berechtigung ausdrücklich entscheiden.").into());
            }
            json!({"decision": self.decision})
        } else {
            let mut answer = serde_json::Map::new();
            let fields = q.external.payload["fields"]
                .as_array()
                .ok_or(crate::tr!("Unbekanntes Frageformat"))?;
            for schema in fields {
                if !field_enabled(schema, &self.values) || schema["type"] == "external" {
                    continue;
                }
                let key = schema["key"].as_str().ok_or(crate::tr!("Feldschlüssel fehlt"))?;
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
                    crate::tr!("Externes Formular: Bitte zuerst den angezeigten Link verwenden.").into(),
                );
            }
            Value::Object(answer)
        };
        if answer.to_string().len() > 32768 {
            return Err(crate::tr!("Antwort maximal 32 KiB").into());
        }
        Ok(AnswerQuestion {
            request_id: RequestId::new(),
            legacy_revision: None,
            answer,
        })
    }
}

/// Edit an open question in the session transcript without a modal or another scroll area.
pub(crate) fn inline_question_answer(ui: &mut egui::Ui, app: &mut App, question: &Question) {
    let mut entry = QuestionAnswer::new(question.clone());
    entry.restore_draft(app);
    let pending = entry.pending_request(app);
    let error_id = ui.make_persistent_id(("inline-question-validation", question.update_seq.0));
    let permission = question.external.kind == QuestionKind::Permission;
    let fields = question.external.payload["fields"].as_array();
    if !permission && fields.is_none_or(Vec::is_empty) {
        ui.label(crate::tr!("Dieses Frageformat kann hier nicht beantwortet werden."));
        return;
    }

    let before = (entry.values.clone(), entry.decision.clone());
    let mut submit_from_editor = false;
    ui.add_enabled_ui(pending.is_none(), |ui| {
        if permission {
            permission_options(ui, &mut entry.decision);
        } else {
            for schema in fields.into_iter().flatten() {
                if schema["hidden"] == true || !field_enabled(schema, &entry.values) {
                    continue;
                }
                if let Some(key) = schema["key"].as_str() {
                    ui.add_space(8.0);
                    submit_from_editor |= ui.push_id(key, |ui| question_field(
                        ui, schema, None, entry.values.entry(key.to_owned()).or_insert(Value::Null),
                    )).inner;
                }
            }
        }
    });
    if before.0 != entry.values || before.1 != entry.decision {
        entry.save_draft(app);
        ui.data_mut(|data| data.remove::<String>(error_id));
    }

    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let enabled = app.online && pending.is_none();
        let submit = design::accent_button(
            ui, egui::Button::new(crate::tr!("Antwort abgeben")),
            enabled, BLUE.gamma_multiply(0.7),
        );
        #[cfg(test)]
        ui.data_mut(|data| data.insert_temp(egui::Id::new(("test-inline-question-submit", question.id)), submit.rect));
        if submit.clicked() || (enabled && submit_from_editor) {
            match entry.payload() {
                Ok(body) => {
                    entry.save_draft(app);
                    entry.send(app, body);
                    ui.data_mut(|data| data.remove::<String>(error_id));
                }
                Err(error) => ui.data_mut(|data| data.insert_temp(error_id, error)),
            }
        }
        if pending.is_some_and(|id| app.in_flight.contains(&id)) {
            ui.label(crate::tr!("Wird übertragen …"));
            ui.spinner();
        }
    });
    if let Some(error) = ui.data(|data| data.get_temp::<String>(error_id)) {
        ui.add(egui::Label::new(egui::RichText::new(error).color(RED)).wrap());
    }
}

impl FormDialog {
    pub(crate) fn stage_in_project_settings(&mut self) {
        if self.saves_project_settings() && self.settings_baseline.is_none() {
            self.settings_baseline = Some(self.body.clone());
        }
    }

    pub(crate) fn settings_dirty(&self) -> bool {
        self.settings_baseline.as_ref().is_some_and(|baseline| {
            self.body.as_object().is_some_and(|body| body.iter().any(|(key, value)| {
                !matches!(key.as_str(), "request_id" | "expected_revision") && baseline.get(key) != Some(value)
            }))
        })
    }

    pub(crate) fn settings_payload(&self, app: &App) -> Result<Value, String> {
        if self.path.ends_with("/mirror") && self.folder_picker.is_some() {
            return Err(crate::tr!("Die Ordnerauswahl ist noch geöffnet.").into());
        }
        if self.is_project_editor() {
            if self.logo_picker.busy() {
                return Err(crate::tr!("Die Bildauswahl ist noch geöffnet.").into());
            }
            if self.folder_picker.is_some() {
                return Err(crate::tr!("Die Ordnerauswahl ist noch geöffnet.").into());
            }
            if !self.is_runner_mapping() && self.body["name"].as_str().is_none_or(|name| name.trim().is_empty()) {
                return Err(crate::tr!("Bitte einen Projektnamen eingeben.").into());
            }
            self.project_request(app)
        } else {
            Ok(self.body.clone())
        }
    }

    pub(crate) fn saves_project_settings(&self) -> bool {
        self.is_project_update()
            || self.path.starts_with("/v1/projects/") && ["/session-limit", "/supervisor", "/token-budget", "/mirror"]
                .iter().any(|suffix| self.path.ends_with(suffix))
    }
    pub(crate) fn is_project_update(&self) -> bool {
        is_project_update(&self.method, &self.path)
    }

    pub(crate) fn is_runner_mapping(&self) -> bool {
        self.is_project_update() && self.body.get("name").is_none()
    }

    pub(crate) fn matches_body(&self, other: &Value) -> bool {
        same_form_body(&self.method, &self.path, &self.body, other)
    }

    fn is_project_editor(&self) -> bool {
        (self.method == "POST" && self.path == "/v1/projects") || self.is_project_update()
    }

    fn is_project_delete(&self) -> bool {
        self.method == "DELETE" && self.body.get("confirm_name").is_some()
    }

    fn show_delete_footer(&mut self, ui: &mut egui::Ui, app: &mut App) {
        let confirmed = self.path.strip_prefix("/v1/projects/")
            .and_then(|id| id.parse::<ProjectId>().ok())
            .and_then(|id| app.snapshot.as_ref()?.projects.iter().find(|project| project.id == id))
            .is_some_and(|project| self.body["confirm_name"].as_str() == Some(project.name.as_str()));
        let button = design::dialog_footer(ui, |ui| {
            let button = design::accent_button(ui, egui::Button::new(crate::tr!("Projekt löschen")),
                app.online && self.request.is_none() && confirmed, RED.gamma_multiply(0.7));
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("test-project-delete-confirm"), button.rect);
                d.insert_temp(egui::Id::new("test-project-delete-right"), ui.max_rect().right());
            });
            if let Some(id) = self.request {
                if app.in_flight.contains(&id) {
                    ui.spinner();
                } else if ui.add_enabled(app.online, egui::Button::new(crate::tr!("Zustellung prüfen"))).clicked() {
                    app.retry(id);
                }
            }
            button
        });
        #[cfg(test)]
        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-project-delete-button"), (button.rect, button.enabled())));
        if button.clicked() {
            self.request = Some(app.write(&self.title, &self.method, self.path.clone(), self.body.clone()));
        }
    }

    fn project_fields(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
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
        let mut branch = self.body["base_branch"].as_str().unwrap_or("main").to_owned();
        let mut source = self.body["_repository_kind"].as_str()
            .unwrap_or_else(|| repository_kind(&path)).to_owned();
        let mut changed = false;
        let name_field = design::settings_field(ui, "Name", |ui| {
            ui.add(design::singleline(&mut name).desired_width(f32::INFINITY))
        });
        changed |= name_field.changed();
        let key_field = design::settings_field(ui, crate::tr!("Projektkürzel"), |ui| ui.add(
            design::singleline(&mut key)
                .hint_text(design::input_hint(if editing { crate::tr!("z. B. WOAI") } else { crate::tr!("Leer lassen für ein automatisches Kürzel") }))
                .desired_width(f32::INFINITY),
        ));
        changed |= key_field.changed();
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(
            egui::Id::new("test-project-basic-field-rects"),
            [name_field.rect, key_field.rect],
        ));
        design::settings_field(ui, crate::tr!("Projektquelle"), |ui| {
            let selected = match source.as_str() {
                "local" => crate::tr!("Lokaler Ordner"),
                "runner" => crate::tr!("Pfad auf dem Runner"),
                "git" => crate::tr!("Git-Repository"),
                _ => crate::tr!("Factory-verwaltet"),
            };
            ui.add_enabled_ui(!path_locked, |ui| {
                egui::ComboBox::from_id_salt("project-repository-kind")
                    .selected_text(selected)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for (value, label) in [
                            ("managed", crate::tr!("Factory-verwaltet")),
                            ("local", crate::tr!("Lokaler Ordner")),
                            ("runner", crate::tr!("Pfad auf dem Runner")),
                            ("git", crate::tr!("Git-Repository")),
                        ] {
                            if design::navigation_row(ui, &label, source == value, None).clicked() {
                                source = value.into();
                                path = if value == "managed" { "factory:managed".into() } else { String::new() };
                                branch = "main".into();
                                changed = true;
                                ui.close();
                            }
                        }
                    });
            });
        });
        if source == "local" {
            design::settings_field(ui, crate::tr!("Lokaler Ordner"), |ui| {
                ui.add_enabled_ui(!path_locked && self.folder_picker.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - design::ICON_BUTTON_SIZE
                            - ui.spacing().item_spacing.x - design::INPUT_MARGIN.sum().x).max(1.0);
                        let path_field = ui.add(
                            design::singleline(&mut path)
                                .hint_text(design::input_hint(crate::tr!("/pfad/zum/projekt")))
                                .desired_width(width),
                        );
                        changed |= path_field.changed();
                        let folder = design::icon_button(
                            ui, egui_phosphor::regular::FOLDER_OPEN, crate::tr!("Projektordner auswählen"), true,
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
                                let mut picker = rfd::FileDialog::new().set_title(crate::tr!("Projektordner auswählen"));
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
            });
        } else if source == "runner" {
            design::settings_field(ui, crate::tr!("Pfad auf dem Runner"), |ui| {
                changed |= ui.add_enabled(!path_locked, design::singleline(&mut path)
                    .hint_text(design::input_hint(crate::tr!("/srv/projekte/mein-projekt")))
                    .desired_width(f32::INFINITY)).changed();
            });
            design::settings_field(ui, crate::tr!("Basisbranch"), |ui| {
                changed |= ui.add_enabled(!path_locked, design::singleline(&mut branch)
                    .hint_text(design::input_hint("main"))
                    .desired_width(f32::INFINITY)).changed();
            });
        } else if source == "git" {
            design::settings_field(ui, crate::tr!("Git-Repository"), |ui| {
                changed |= ui.add_enabled(!path_locked, design::singleline(&mut path)
                    .hint_text(design::input_hint("https://github.com/organisation/projekt.git"))
                    .desired_width(f32::INFINITY)).changed();
            });
            design::settings_field(ui, crate::tr!("Basisbranch"), |ui| {
                changed |= ui.add_enabled(!path_locked, design::singleline(&mut branch)
                    .hint_text(design::input_hint("main"))
                    .desired_width(f32::INFINITY)).changed();
            });
        } else {
            ui.colored_label(MUTED, crate::tr!("Das Repository wird vom ausgewählten Runner verwaltet."));
        }
        if path_locked {
            ui.colored_label(MUTED, crate::tr!("Die Projektquelle ist nach dem Anlegen von Workspaces festgeschrieben."));
        } else if editing && path.is_empty() {
            ui.colored_label(MUTED, crate::tr!("Ohne neuen Pfad bleibt das bisherige Repository unverändert."));
        }
        if !editing {
            ui.add_space(8.0);
            design::settings_field(ui, crate::tr!("Runner-Zuordnung"), |ui| {
                self.runner_mapping_field(ui, app);
            });
        }
        if let Some(error) = self.logo_picker.show(ui, &mut self.body, editing) {
            app.notice_error(error);
        }
        if changed {
            self.error = None;
            self.body["name"] = json!(name);
            self.body["key"] = json!(key);
            self.body["repository"] = json!(path);
            self.body["base_branch"] = json!(branch);
            self.body["_repository_kind"] = json!(source);
        }
        !name.trim().is_empty()
            && (source == "managed" || !path.trim().is_empty())
            && self.folder_picker.is_none()
            && !self.logo_picker.busy()
    }

    fn runner_mapping_field(&mut self, ui: &mut egui::Ui, app: &App) {
        let mut mapping = serde_json::from_value::<Option<ProjectRunnerMapping>>(
            self.body["runner_mapping"].clone(),
        ).ok().flatten().unwrap_or(ProjectRunnerMapping::Automatic);
        let current_mapping = mapping;
        let runners = app.snapshot.as_ref().map_or(&[][..], |s| s.runners.as_slice());
        let selected = match mapping {
            ProjectRunnerMapping::Automatic => crate::tr!("Automatisch").into(),
            ProjectRunnerMapping::Runner(id) => runners.iter().find(|runner| runner.id == id)
                .map_or_else(|| crate::tr!("Runner nicht verfügbar").into(), |runner| runner.name.clone()),
        };
        let _runner_select = egui::ComboBox::from_id_salt("project-runner-mapping")
            .selected_text(selected)
            .wrap_mode(egui::TextWrapMode::Truncate)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                if design::navigation_row(ui, crate::tr!("Automatisch"), mapping == ProjectRunnerMapping::Automatic, None).clicked() {
                    mapping = ProjectRunnerMapping::Automatic;
                    ui.close();
                }
                for runner in runners.iter().filter(|runner| !app.runner_removed(runner.id)) {
                    let option = ProjectRunnerMapping::Runner(runner.id);
                    if design::navigation_row(ui, &runner.name, mapping == option, None).clicked() {
                        mapping = option;
                        ui.close();
                    }
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
    }
    fn project_mirror_fields(&mut self, ui: &mut egui::Ui, app: &App) -> bool {
        use std::sync::mpsc::TryRecvError;
        if let Some(picker) = &self.folder_picker {
            match picker.try_recv() {
                Ok(path) => {
                    if let Some(path) = path {
                        self.body["path"] = json!(path);
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
        let project = self.path.strip_prefix("/v1/projects/")
            .and_then(|tail| tail.strip_suffix("/mirror"))
            .and_then(|id| id.parse::<ProjectId>().ok())
            .and_then(|id| app.snapshot.as_ref()?.projects.iter().find(|project| project.id == id));
        if project.is_none_or(|project| project.repository != "factory:managed") {
            ui.label(crate::tr!("Lokale Spiegelkopien sind nur für Factory-verwaltete Projekte verfügbar."));
            return false;
        }
        let runners: Vec<_> = app.local_runners.iter()
            .filter(|runner| runner.visible(&app.server)).collect();
        if runners.is_empty() {
            ui.label(crate::tr!("Auf diesem Desktop ist kein lokaler Runner eingerichtet."));
            return false;
        }
        let mut runner = serde_json::from_value::<RunnerId>(self.body["runner_id"].clone())
            .ok().filter(|id| runners.iter().any(|candidate| candidate.id == *id))
            .unwrap_or(runners[0].id);
        let previous = runner;
        design::settings_field(ui, crate::tr!("Lokaler Runner"), |ui| {
            let selected = runners.iter().find(|candidate| candidate.id == runner)
                .map_or_else(|| crate::tr!("Runner nicht verfügbar").into(), |candidate| candidate.name.clone());
            egui::ComboBox::from_id_salt("project-mirror-runner")
                .selected_text(selected).width(ui.available_width()).show_ui(ui, |ui| {
                    for candidate in &runners {
                        if design::navigation_row(ui, &candidate.name, candidate.id == runner, None).clicked() {
                            runner = candidate.id;
                            ui.close();
                        }
                    }
                });
        });
        if runner != previous {
            self.body["runner_id"] = json!(runner);
            self.body["path"] = json!(project.and_then(|project| project.mirrors.get(&runner))
                .map_or("", |mirror| mirror.path.as_str()));
        }
        let mut path = self.body["path"].as_str().unwrap_or("").to_owned();
        design::settings_field(ui, crate::tr!("Lokaler Testordner"), |ui| {
            ui.add_enabled_ui(self.folder_picker.is_none(), |ui| {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - design::ICON_BUTTON_SIZE
                        - ui.spacing().item_spacing.x - design::INPUT_MARGIN.sum().x).max(1.0);
                    let field = ui.add(design::singleline(&mut path)
                        .hint_text(design::input_hint(crate::tr!("/pfad/zur/testkopie")))
                        .desired_width(width));
                    let folder = design::icon_button(
                        ui, egui_phosphor::regular::FOLDER_OPEN,
                        crate::tr!("Testordner auswählen"), true,
                    );
                    if folder.clicked() {
                        let (send, receive) = std::sync::mpsc::channel();
                        let ctx = ui.ctx().clone();
                        let current = std::path::PathBuf::from(path.trim());
                        std::thread::spawn(move || {
                            let mut picker = rfd::FileDialog::new()
                                .set_title(crate::tr!("Testordner auswählen"));
                            if current.parent().is_some_and(|parent| parent.is_dir()) {
                                picker = picker.set_directory(current.parent().unwrap());
                            }
                            let _ = send.send(picker.pick_folder()
                                .and_then(|path| path.into_os_string().into_string().ok()));
                            ctx.request_repaint();
                        });
                        self.folder_picker = Some(receive);
                    }
                    if field.changed() {
                        self.error = None;
                    }
                });
            });
        });
        self.body["runner_id"] = json!(runner);
        self.body["path"] = json!(path.trim());
        if !path.trim().is_empty() {
            let state = app.snapshot.as_ref().and_then(|snapshot| {
                snapshot.commands.iter()
                    .filter(|command| command.runner_id == runner)
                    .filter(|command| command.workspace_spec.as_ref()
                        .and_then(|spec| spec.setup.as_ref())
                        .is_some_and(|setup| setup.repository_source == CHECKPOINT_RETURN
                            && setup.repository_path == path.trim()))
                    .max_by_key(|command| command.created_unix_ms)
                    .map(|command| command.state)
            });
            let status = match state {
                Some(CommandState::Pending) => crate::tr!("Spiegelung wartet auf den lokalen Runner."),
                Some(CommandState::Accepted) => crate::tr!("Spiegelung läuft."),
                Some(CommandState::Succeeded) => crate::tr!("Spiegelung zuletzt erfolgreich bestätigt."),
                Some(CommandState::Failed) => crate::tr!("Spiegelung ist blockiert. Details stehen in der Diagnose."),
                None => crate::tr!("Spiegelung wird nach dem nächsten bestätigten Projektstand eingeplant."),
            };
            ui.colored_label(MUTED, status);
        }
        ui.colored_label(MUTED, crate::tr!("Leer speichern entfernt die Spiegelkopie. Lokale Änderungen werden nie überschrieben."));
        self.folder_picker.is_none()
    }

    fn project_request(&self, app: &App) -> Result<Value, String> {
        let mut body = self.body.clone();
        if self.is_runner_mapping() {
            return Ok(body);
        }
        let name = body["name"].as_str().unwrap_or("").trim().to_owned();
        let key = body["key"].as_str().unwrap_or("").trim().to_ascii_uppercase();
        let source = body["_repository_kind"].as_str()
            .unwrap_or_else(|| repository_kind(body["repository"].as_str().unwrap_or("")));
        let mut path = body["repository"].as_str().unwrap_or("").trim().to_owned();
        let mut branch = body["base_branch"].as_str().unwrap_or("main").trim().to_owned();
        match source {
            "managed" => {
                path = "factory:managed".into();
                branch = "main".into();
            }
            "local" => branch = local_project_branch(&path)?,
            "runner" => {
                if !absolute_repository_path(&path) {
                    return Err(crate::tr!("Bitte einen absoluten Pfad auf dem Runner angeben.").into());
                }
                validate_base_branch(&branch)?;
            }
            "git" => {
                if path.is_empty() || absolute_repository_path(&path) || path == "factory:managed" {
                    return Err(crate::tr!("Bitte eine Git-Repository-URL angeben.").into());
                }
                validate_base_branch(&branch)?;
            }
            _ => return Err(crate::tr!("Bitte eine gültige Projektquelle auswählen.").into()),
        }
        body.as_object_mut().unwrap().remove("_repository_kind");
        let existing = self.path.strip_prefix("/v1/projects/")
            .and_then(|id| id.parse::<ProjectId>().ok())
            .and_then(|id| app.snapshot.as_ref()?.projects.iter().find(|project| project.id == id));
        if self.is_project_update() {
            body.as_object_mut().unwrap().remove("runner_mapping");
        }
        body["name"] = json!(name);
        if !key.is_empty()
            && (key.len() > 10 || !key.starts_with(|c: char| c.is_ascii_uppercase())
                || !key.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
        {
            return Err(crate::tr!("Das Projektkürzel muss mit einem Buchstaben beginnen und darf höchstens 10 Buchstaben oder Ziffern enthalten.").into());
        }
        if key.is_empty() && existing.is_some_and(|p| !p.key.is_empty()) {
            return Err(crate::tr!("Das Projektkürzel darf nicht leer sein.").into());
        }
        body["key"] = json!((!key.is_empty()).then_some(key));
        if self.method == "PATCH"
            && (path.is_empty() || existing.is_some_and(|p| p.repository == path))
        {
            body["repository"] = Value::Null;
            body["base_branch"] = Value::Null;
        } else {
            body["base_branch"] = json!(branch);
            body["repository"] = json!(path);
        }
        Ok(body)
    }

    pub(crate) fn persist(&self, app: &mut App) {
        app.settings.form_drafts.retain(|d| {
            d.server != app.server
                || d.path != self.path
                || d.method != self.method
                || !self.matches_body(&d.body)
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
        self.show_form(ui, app, height, None, false)
    }

    pub(crate) fn show_in_settings(&mut self, ui: &mut egui::Ui, app: &mut App, bottom: f32, height: f32) -> bool {
        if self.settings_baseline.is_some() {
            self.show_form(ui, app, height, Some(bottom), true);
            return false;
        }
        let saves_settings = self.is_project_update() || self.path == "/v1/projects/setup"
            || self.path.starts_with("/v1/projects/") && ["/profile", "/scheduler", "/session-limit", "/supervisor", "/token-budget", "/mirror"]
                .iter().any(|suffix| self.path.ends_with(suffix));
        self.show_form(ui, app, height, saves_settings.then_some(bottom), false)
    }

    fn restore_pending(&mut self, app: &App) {
        if self.request.is_none() {
            if let Some(pending) = app.settings.pending.iter().find(|r| {
                r.server == app.server
                    && r.path == self.path
                    && r.method == self.method
                    && self.matches_body(&r.body)
            }) {
                self.request = Some(pending.id);
                if self.is_project_editor() {
                    for key in ["request_id", "expected_revision", "name", "key", "repository", "runner_mapping", "logo"] {
                        if let Some(value) = pending.body.get(key).filter(|value| key == "logo" || !value.is_null()) {
                            self.body[key] = value.clone();
                        }
                    }
                } else {
                    self.body = pending.body.clone();
                }
            }
        }
    }

    fn intent_confirmation(&self) -> Option<(&'static str, egui::Color32, &'static str)> {
        if self.method == "PATCH" && self.path.starts_with("/v1/tickets/") {
            if self.body["status"] == "cancelled" {
                return Some((crate::tr!("Intent abbrechen"), RED, "test-intent-cancel-confirm"));
            }
            if self.body["status"] == "done" {
                return Some((crate::tr!("Als umgesetzt bestätigen"), BLUE, "test-intent-complete-confirm"));
            }
        }
        if !self.path.starts_with("/v1/tasks/") { return None; }
        if self.method == "DELETE" {
            Some((crate::tr!("Löschen"), RED, "test-task-delete-confirm"))
        } else if self.method == "POST" && self.path.ends_with("/accept-solution") {
            Some((crate::tr!("Lösung annehmen"), BLUE, "test-inbox-solution-confirm"))
        } else if self.method == "POST" && self.path.ends_with("/return-to-draft") {
            Some((crate::tr!("Session und Worktree verwerfen"), RED, "test-inbox-draft-confirm"))
        } else { None }
    }

    fn show_form(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32, settings_bottom: Option<f32>, staged: bool) -> bool {
        if !staged { self.restore_pending(app); }
        if self.method == "POST" && self.path.starts_with("/v1/devices/") && self.path.ends_with("/revoke") {
            return self.show_device_revoke(ui, app);
        }
        if self.method == "DELETE" && self.body.get("confirm_name").is_some() {
            return self.show_project_delete(ui, app);
        }
        if self.path == "/v1/projects/setup" && self.request.is_none() {
            self.body["setup"]["repository_source"] = self.body["repository"].clone();
        }
        let project_editor = self.is_project_editor();
        if !project_editor && !self.help.is_empty() {
            ui.label(crate::i18n::label(&self.help));
        }
        let mut valid = true;
        if self.path.starts_with("/v1/projects/") && self.path.ends_with("/mirror") {
            valid = self.project_mirror_fields(ui, app);
        } else if self.path.starts_with("/v1/projects/") && self.path.ends_with("/supervisor") {
            let seconds = self.body["interval_seconds"].as_u64().unwrap_or(300);
            let mut minutes = seconds as f64 / 60.0;
            design::settings_field(ui, crate::tr!("Prüfintervall in Minuten"), |ui| {
                let input = ui.add(egui::DragValue::new(&mut minutes)
                    .range(0.5..=1440.0)
                    .speed(0.1)
                    .max_decimals(2))
                    .on_hover_text(crate::tr!("Mindestabstand zwischen zwei Supervisor-Prüfungen."));
                if input.changed() {
                    self.body["interval_seconds"] = json!((minutes * 60.0).round() as u64);
                }
            });
        } else if self.path.starts_with("/v1/projects/") && self.path.ends_with("/session-limit") {
            let mut limit = self.body["max_sessions"].as_u64()
                .unwrap_or_else(|| u64::from(default_session_limit()));
            design::settings_field(ui, crate::tr!("Gleichzeitig arbeitende Sessions"), |ui| {
                let viewport = ui.ctx().content_rect();
                let anchor = ui.cursor().top();
                let room = (anchor - viewport.top())
                    .max(viewport.bottom() - anchor - ui.spacing().interact_size.y);
                let rows = 16.0 * (design::LIST_ROW_HEIGHT + ui.spacing().item_spacing.y);
                let limit_picker = egui::ComboBox::from_id_salt(("project-session-limit", self.id))
                    .selected_text(limit.to_string())
                    .width(ui.available_width())
                    .height(rows.min((room - 24.0).max(design::LIST_ROW_HEIGHT)))
                    .show_ui(ui, |ui| {
                        for value in 1..=16 {
                            let option = design::navigation_row(ui, &value.to_string(), limit == value, None);
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-project-limit-option", value)), option.rect));
                            if option.clicked() {
                                limit = value;
                                ui.close();
                            }
                        }
                    })
                    .response.on_hover_cursor(egui::CursorIcon::PointingHand);
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-project-limit-picker"), limit_picker.rect));
                #[cfg(not(test))]
                let _ = limit_picker;
            });
            self.body["max_sessions"] = json!(limit);
        } else if project_editor {
            let form_id = self.id;
            let fields = |ui: &mut egui::Ui| {
                ui.add_enabled_ui(self.request.is_none(), |ui| {
                    if self.is_runner_mapping() {
                        if !self.help.is_empty() {
                            ui.label(crate::i18n::label(&self.help));
                            ui.add_space(design::SETTINGS_FIELD_GAP);
                        }
                        self.runner_mapping_field(ui, app);
                        valid = self.path.rsplit('/').next().and_then(|id| id.parse::<ProjectId>().ok())
                            .and_then(|id| app.snapshot.as_ref()?.projects.iter().find(|p| p.id == id))
                            .is_some_and(|p| self.body["runner_mapping"] != json!(p.runner_mapping.unwrap_or(ProjectRunnerMapping::Automatic)));
                    } else {
                        valid = self.project_fields(ui, app);
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(RED, error);
                }
            };
            if let Some(bottom) = settings_bottom {
                egui::ScrollArea::vertical()
                    .id_salt(("project-fields", form_id))
                    .max_height(if staged { (bottom - ui.cursor().top()).max(0.0) } else { design::settings_body_height(ui, bottom) })
                    .show(ui, fields);
            } else {
                let footer_height = ui.spacing().interact_size.y.max(
                    ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y
                ) + 12.0;
                egui::ScrollArea::vertical()
                    .id_salt(("project-fields", form_id))
                    .max_height((height - footer_height).max(24.0))
                    .show(ui, fields);
                ui.add_space(12.0);
            }
        } else {
            let height = if staged {
                settings_bottom.map_or(height, |bottom| (bottom - ui.cursor().top()).max(0.0))
            } else {
                height
            };
            egui::ScrollArea::vertical()
                .id_salt("management-dialog-scroll")
                .max_height(height)
                .show(ui, |ui| {
                    ui.add_enabled_ui(self.request.is_none(), |ui| {
                        valid = management_editor(ui, &mut self.body, "body", app, staged);
                    });
                    if !staged {
                        ui.collapsing(
                            crate::tr!("Vollständige Anfragedaten (einschließlich Request-ID)"),
                            |ui| json_read(ui, &self.body),
                        );
                    }
                });
        }
        if staged {
            return false;
        }
        if let Some(e) = self.error.as_ref().filter(|_| !project_editor) {
            ui.colored_label(RED, e);
        }
        if let Some(id) = self.request {
            if !self.is_project_update() || app.in_flight.contains(&id) {
                request_status(ui, app, id);
            } else if app.settings.pending.iter().any(|r| r.id == id)
                && ui.add_enabled(app.online, egui::Button::new(crate::tr!("Identische Anfrage abgleichen"))).clicked()
            {
                app.retry(id);
            }
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
                    ui.colored_label(RED, crate::tr!("Runner-Helfer ist fehlgeschlagen. Ausgabe und Exitcode im Ergebnis prüfen."));
                }
                ui.collapsing(crate::tr!("Bestätigtes Ergebnis"), |ui| json_read(ui, result));
            }
            if app
                .responses
                .get(&id)
                .is_some_and(|v| v.as_ref().is_err_and(|e| !e.uncertain))
                && ui.button(crate::tr!("Eingaben korrigieren")).clicked()
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
                    ui.collapsing(crate::tr!("Aktuellen Serverstand mit meinem Entwurf vergleichen"), |ui| {
                        if self.is_runner_mapping() {
                            let mapping = serde_json::from_value::<ProjectRunnerMapping>(current["runner_mapping"].clone())
                                .unwrap_or(ProjectRunnerMapping::Automatic);
                            let label = match mapping {
                                ProjectRunnerMapping::Automatic => crate::tr!("Automatisch"),
                                ProjectRunnerMapping::Runner(id) => app.snapshot.as_ref()
                                    .and_then(|s| s.runners.iter().find(|runner| runner.id == id))
                                    .map_or(crate::tr!("Runner nicht verfügbar"), |runner| runner.name.as_str()),
                            };
                            ui.label(label);
                        } else if project_editor {
                            for (label, key) in [("Name", "name"), (crate::tr!("Projektkürzel"), "key"), (crate::tr!("Lokaler Pfad"), "repository")] {
                                let value = current[key].as_str().unwrap_or("");
                                if key != "repository" || value.starts_with('/') {
                                    ui.label(format!("{label}: {value}"));
                                }
                            }
                        } else {
                            json_read(ui, &current);
                        }
                    });
                    ui.small(crate::tr!("Revision erst nach Vergleich übernehmen. Eigene Eingaben bleiben erhalten und werden nicht stillschweigend mit fremden Änderungen zusammengeführt."));
                    if ui
                        .button(crate::tr!("Serverrevision {revision} übernehmen und Entwurf erneut prüfen", revision = revision))
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
                ui, egui::Button::new(if project_editor && self.method == "POST" {
                    crate::tr!("Projekt anlegen")
                } else if project_editor {
                    crate::tr!("Speichern")
                } else {
                    crate::tr!("Bestätigen und ausführen")
                }),
                app.online && valid,
                BLUE.gamma_multiply(0.7),
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
                    self.error = Some(crate::tr!("Bitte eine gültige Geräte-ID auswählen.").into());
                    return false;
                }
                path = path.replace("{device_id}", device);
                body.as_object_mut().unwrap().remove("device_id");
            }
            self.request = Some(app.write(&self.title, &self.method, path, body));
        }
        settings_bottom.is_none() && !project_editor && ui.button(crate::tr!("Schließen")).clicked()
    }

    fn show_project_delete(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        if self.request.is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)) {
            return true;
        }
        if self.request.is_some_and(|id| app.responses.get(&id).is_some_and(|result|
            result.as_ref().is_err_and(|error| !error.uncertain)))
        {
            self.request = None;
            self.body["request_id"] = json!(RequestId::new());
        }
        ui.label(crate::i18n::label(&self.help));
        let project = app.snapshot.as_ref().and_then(|snapshot|
            snapshot.projects.iter().find(|project| self.path == format!("/v1/projects/{}", project.id)));
        // Revisions are concurrency guards, not user input. A rejected request
        // keeps the confirmation draft and requires another explicit click.
        if self.request.is_none() {
            if let Some(project) = project {
                self.body["expected_revision"] = json!(project.revision);
            }
        }
        let mut name = self.body["confirm_name"].as_str().unwrap_or("").to_owned();
        design::settings_field(ui, crate::tr!("Projektname zur Bestätigung"), |ui| {
            let input = ui.add_enabled(self.request.is_none(), design::singleline(&mut name).desired_width(f32::INFINITY));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-project-delete-name"), input.rect));
            #[cfg(not(test))]
            let _ = input;
        });
        self.body["confirm_name"] = json!(name);
        false
    }

    fn show_intent_confirmation(&mut self, ui: &mut egui::Ui, app: &mut App, action: &str, color: egui::Color32, _test_id: &str) -> bool {
        // Do not silently retarget the confirmation if the Intent changed.
        let rejected = self.request.is_some_and(|id| app.responses.get(&id)
            .is_some_and(|r| r.as_ref().is_err_and(|error| !error.uncertain)));
        let capability = if self.method != "DELETE" { None }
            else if self.path.ends_with("/intent") { Some(DELETE_TASK_CAPABILITY) }
            else if self.path.ends_with("/inactive") { Some(DELETE_INACTIVE_TASK_CAPABILITY) }
            else { Some(DELETE_FAILED_TASK_CAPABILITY) };
        let incompatible = capability.is_some_and(|cap| app.snapshot.as_deref()
            .is_some_and(|s| !server_supports(s, cap)));
        design::dialog_footer(ui, |ui| {
            let confirm = design::server_compatibility_tooltip(
                design::accent_button(ui, egui::Button::new(action),
                    !incompatible && app.online && self.request.is_none(), color.gamma_multiply(0.7)),
                incompatible,
            );
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(_test_id), confirm.rect));
            if confirm.clicked() {
                self.request = Some(app.write(&self.title, &self.method, self.path.clone(), self.body.clone()));
            }
            let close = ui.button(crate::tr!("Abbrechen")).clicked();
            if let Some(id) = self.request.filter(|_| !rejected) {
                if app.in_flight.contains(&id) {
                    ui.spinner();
                } else if design::server_compatibility_tooltip(
                    ui.add_enabled(app.online && !incompatible, egui::Button::new(crate::tr!("Zustellung prüfen"))), incompatible,
                ).clicked() {
                    app.retry(id);
                }
            }
            close
        })
    }

    fn show_device_revoke(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        if self.request.is_some_and(|id| app.responses.get(&id).is_some_and(Result::is_ok)) { return true; }
        if self.request.is_some_and(|id| app.responses.get(&id).is_some_and(|r|
            r.as_ref().is_err_and(|error| !error.uncertain))) { self.request = None; }
        ui.label(crate::i18n::label(&self.help));
        ui.add_space(12.0);
        let mut close = false;
        ui.horizontal(|ui| {
            let confirm = design::accent_button(ui, egui::Button::new(crate::tr!("Zugang widerrufen")),
                app.online && self.request.is_none(), RED.gamma_multiply(0.7));
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-device-revoke-confirm"), confirm.rect));
            if confirm.clicked() {
                self.request = Some(app.write(crate::tr!("Gerätezugang widerrufen"), "POST", self.path.clone(), Value::Null));
            }
            if ui.button(if self.request.is_some() { crate::tr!("Zurück") } else { crate::tr!("Abbrechen") }).clicked() { close = true; }
            if let Some(id) = self.request {
                if !app.in_flight.contains(&id) {
                    if ui.add_enabled(app.online, egui::Button::new(crate::tr!("Zustellung prüfen"))).clicked() { app.retry(id); }
                } else { ui.spinner(); }
            }
        });
        close
    }
}

pub(crate) fn is_project_update(method: &str, path: &str) -> bool {
    method == "PATCH" && path.strip_prefix("/v1/projects/")
        .is_some_and(|id| id.parse::<ProjectId>().is_ok())
}

pub(crate) fn same_form_body(method: &str, path: &str, first: &Value, second: &Value) -> bool {
    if method == "PATCH" && path.starts_with("/v1/tickets/") {
        return first.get("status") == second.get("status") && first.get("expected_revision") == second.get("expected_revision");
    }
    if method == "POST" && path.starts_with("/v1/tasks/")
        && (path.ends_with("/accept-solution") || path.ends_with("/return-to-draft")) {
        return first.get("previous_command") == second.get("previous_command")
            && first.get("expected_revision") == second.get("expected_revision");
    }
    first.get("project_id") == second.get("project_id")
        && (!is_project_update(method, path) || first.get("name").is_some() == second.get("name").is_some())
}

fn absolute_repository_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/') || path.starts_with("\\\\")
        || (bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && matches!(bytes.get(1..3), Some(b":\\" | b":/")))
}

fn repository_kind(repository: &str) -> &'static str {
    if repository == "factory:managed" || repository.is_empty() {
        "managed"
    } else if absolute_repository_path(repository) {
        if std::path::Path::new(repository).exists() { "local" } else { "runner" }
    } else {
        "git"
    }
}

fn validate_base_branch(branch: &str) -> Result<(), String> {
    if branch.is_empty() || branch.len() > 200 || branch.chars().any(char::is_control) {
        Err(crate::tr!("Bitte einen gültigen Basisbranch angeben.").into())
    } else {
        Ok(())
    }
}

/// Read only: preparing or creating a missing repository remains a runner job.
fn local_project_branch(path: &str) -> Result<String, String> {
    if path.len() > 2048 || path.chars().any(char::is_control) {
        return Err(crate::tr!("Der lokale Pfad ist zu lang oder enthält ungültige Zeichen.").into());
    }
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err(crate::tr!("Bitte einen absoluten lokalen Pfad angeben.").into());
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok("main".into()),
        Ok(meta) if path.is_dir() => meta,
        _ => return Err(crate::tr!("Der lokale Pfad muss ein Ordner sein.").into()),
    };
    let mut entries = std::fs::read_dir(path)
        .map_err(|_| crate::tr!("Der lokale Ordner kann nicht gelesen werden."))?;
    if entries.next().transpose().map_err(|_| crate::tr!("Der lokale Ordner kann nicht gelesen werden."))?.is_none() {
        if !metadata.is_dir() {
            return Err(crate::tr!("Bitte den leeren Ordner direkt statt über einen Link angeben.").into());
        }
        return Ok("main".into());
    }
    if !path.join(".git").exists() {
        return Err(crate::tr!("Der Ordner ist nicht leer und enthält kein Git-Repository. Bitte einen leeren Ordner wählen.").into());
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
        command.arg("-C").arg(path).args(args).output()
    };
    let branch = run_git(&["symbolic-ref", "--quiet", "HEAD"]).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            crate::tr!("Git wurde nicht gefunden. Bitte Git installieren und die App neu starten.")
        } else {
            crate::tr!("Git konnte nicht gestartet werden.")
        }
    })?;
    let branch = if branch.status.success() { String::from_utf8(branch.stdout).ok() } else { None };
    if let Some(branch) = branch.as_deref()
        .and_then(|branch| branch.trim_end().strip_prefix("refs/heads/"))
    {
        if !run_git(&["rev-parse", "--verify", "HEAD^{commit}"]).is_ok_and(|out| out.status.success()) {
            return Err(crate::tr!("Der ausgecheckte Branch hat keinen gültigen Commit.").into());
        }
        Ok(branch.into())
    } else {
        Err(crate::tr!("Git konnte keinen ausgecheckten Branch lesen. Bitte den Branch und den Zugriff auf das Repository prüfen.").into())
    }
}

pub(crate) fn pending_requests(
    ui: &mut egui::Ui,
    app: &mut App,
    height: f32,
    show_close: bool,
) -> bool {
    ui.label(crate::tr!("Diese Anfragen wurden noch nicht eindeutig bestätigt. Ein Abgleich wiederholt nur die identische gespeicherte Anfrage — keine neue Aufgabe."));
    egui::ScrollArea::vertical().id_salt("settings-pending").max_height((height-100.0).max(40.0)).show(ui,|ui| {
        for request in app.settings.pending.clone() {
            ui.push_id(request.id,|ui| {app::card().show(ui,|ui| {
                ui.strong(&request.label);ui.label(&request.server);ui.monospace(&request.path);
                ui.collapsing(crate::tr!("Unveränderte gespeicherte Anfrage"),|ui|json_read(ui,&request.body));
                if request.server!=app.server {ui.label(crate::tr!("Zum zugehörigen Server verbinden, um abzugleichen."));} else {request_status(ui,app,request.id);}
                if request.path=="/v1/devices/enrollment" && !app.in_flight.contains(&request.id) && ui.button(crate::tr!("Hinweis verwerfen · unbekannte Einladung bleibt bis Ablauf gültig")).clicked() {app.settings.pending.retain(|p|p.id!=request.id);app.settings_dirty=true;}
            });});
        }
    });
    show_close && ui.button(crate::tr!("Schließen")).clicked()
}

pub fn request_status(ui: &mut egui::Ui, app: &mut App, id: u64) {
    let replayable = app
        .settings
        .pending
        .iter()
        .find(|r| r.id == id)
        .is_none_or(|r| r.path != "/v1/devices/enrollment");
    if !replayable {
        ui.colored_label(AMBER, crate::tr!("Einladungserstellung wird bei unklarem Ausgang nicht wiederholt. Unbekannter Code läuft gemäß Gültigkeit ab; gegebenenfalls Geräteverwaltung prüfen."));
    }
    if app.in_flight.contains(&id) {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(crate::tr!("Wird übertragen …"));
        });
    } else {
        match app.responses.get(&id) {
            Some(Ok(_)) => {
                ui.colored_label(
                    app::GREEN,
                    crate::tr!("Anfrage bestätigt — Ergebnis und Zustellstatus prüfen."),
                );
            }
            Some(Err(e)) => {
                ui.colored_label(if e.uncertain { AMBER } else { RED }, &e.message);
                if e.uncertain {
                    ui.label(crate::tr!("Ausgang unklar. Eingaben bleiben gesperrt; Abgleich verwendet dieselbe Request-ID und Nutzlast."));
                }
            }
            None => {
                ui.colored_label(
                    AMBER,
                    crate::tr!("Bestätigung fehlt. Gespeicherte Anfrage kann abgeglichen werden."),
                );
            }
        }
    }
    if replayable
        && app.settings.pending.iter().any(|r| r.id == id)
        && ui
            .add_enabled(
                app.online && !app.in_flight.contains(&id),
                egui::Button::new(crate::tr!("Identische Anfrage abgleichen")),
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
                .add_enabled(page > 0, egui::Button::new(crate::tr!("‹ Vorheriger Textabschnitt")))
                .clicked()
            {
                page -= 1;
            }
            ui.label(crate::tr!("Abschnitt {} / {pages} · {} Bytes vollständig zugänglich", page + 1,
                text.len(), pages = pages));
            if ui
                .add_enabled(
                    page + 1 < pages,
                    egui::Button::new(crate::tr!("Nächster Textabschnitt ›")),
                )
                .clicked()
            {
                page += 1;
            }
            if ui.button(crate::tr!("Gesamten Text kopieren")).clicked() {
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

fn permission_options(ui: &mut egui::Ui, decision: &mut String) {
    for (value, label) in [
        ("once", crate::tr!("Einmal erlauben")),
        ("always", crate::tr!("Dauerhaft erlauben (erweiterte Freigabe)")),
        ("reject", crate::tr!("Ablehnen")),
    ] {
        if question_option(ui, decision.as_str() == value, false, label, None) {
            *decision = value.into();
        }
    }
}

fn question_option(ui: &mut egui::Ui, selected: bool, multiple: bool, label: &str, description: Option<&str>) -> bool {
    ui.add_space(6.0);
    let width = ui.available_width();
    let row = ui.vertical(|ui| {
        ui.set_min_width(width);
        let label = egui::RichText::new(label).strong();
        let clicked = if multiple {
            let mut checked = selected;
            ui.checkbox(&mut checked, label).on_hover_cursor(egui::CursorIcon::PointingHand).changed()
        } else {
            ui.radio(selected, label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        };
        let description_clicked = description.filter(|text| !text.trim().is_empty()).is_some_and(|text| {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.add(egui::Label::new(egui::RichText::new(text).color(MUTED)).wrap().sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
            }).inner
        });
        clicked || description_clicked
    });
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-question-option", label)), row.response.rect));
    row.inner
}

fn question_field(ui: &mut egui::Ui, schema: &Value, fallback_description: Option<&str>, value: &mut Value) -> bool {
    let title = schema["title"]
        .as_str()
        .or(schema["key"].as_str())
        .unwrap_or(crate::tr!("Feld"));
    design::question_heading(ui, &format!(
        "{title}{}",
        if schema["required"] == true { " *" } else { "" }
    ));
    if let Some(description) = schema["description"].as_str().or(fallback_description) {
        ui.label(description);
    }
    if schema["type"] == "external" {
        if let Some(url) = schema["url"].as_str().filter(|u| {
            u.starts_with("https://")
                || u.starts_with("http://localhost")
                || u.starts_with("http://127.0.0.1")
        }) {
            ui.hyperlink_to(crate::tr!("Externes Formular öffnen"), url);
        } else {
            ui.label(crate::tr!("Kein gültiger Link zum externen Formular vorhanden."));
        }
        return false;
    }
    let multiple = schema["type"] == "multiselect";
    if let Some(options) = schema["options"].as_array().filter(|v| !v.is_empty()) {
        let custom = schema["custom"] == true && schema["type"] == "string";
        let custom_id = ui.make_persistent_id("custom-answer-mode");
        let mut own = custom && ui.data(|d| d.get_temp(custom_id)).unwrap_or_else(||
            value.is_string() && !options.iter().any(|option| option["value"] == *value));
        for (index, option) in options.iter().enumerate() {
            let label = option["label"]
                .as_str()
                .or(option["value"].as_str())
                .unwrap_or(crate::tr!("Option"));
            let description = option["description"].as_str();
            if multiple {
                let mut values = value.as_array().cloned().unwrap_or_default();
                let selected = values.contains(&option["value"]);
                if ui.push_id(index, |ui| question_option(ui, selected, true, label, description)).inner {
                    if selected {
                        values.retain(|v| v != &option["value"]);
                    } else {
                        values.push(option["value"].clone());
                    }
                    *value = Value::Array(values);
                }
            } else if ui.push_id(index, |ui| question_option(ui, !own && *value == option["value"], false, label, description)).inner {
                if own {
                    ui.data_mut(|d| d.insert_temp(custom_id.with("text"), value.as_str().unwrap_or("").to_owned()));
                }
                own = false;
                *value = option["value"].clone();
            }
        }
        if custom {
            if ui.push_id("own", |ui| question_option(ui, own, false, crate::tr!("Eigene Antwort"), None)).inner && !own {
                own = true;
                *value = json!(ui.data(|d| d.get_temp::<String>(custom_id.with("text"))).unwrap_or_default());
            }
            ui.data_mut(|d| d.insert_temp(custom_id, own));
            if !own {
                return false;
            }
            ui.add_space(4.0);
        } else {
            if schema["custom"] != true {
                return false;
            }
            ui.add_space(8.0);
            ui.label(crate::tr!("Eigene Antwort (optional)"));
        }
    }
    match schema["type"].as_str() {
        Some("boolean") => {
            ui.horizontal(|ui| {
                if ui.radio(*value == true, crate::tr!("Ja")).clicked() {
                    *value = json!(true);
                }
                if ui.radio(*value == false, crate::tr!("Nein")).clicked() {
                    *value = json!(false);
                }
            });
        }
        Some("string") => {
            let mut text = value.as_str().unwrap_or("").to_string();
            let (input, submit) = design::question_answer_editor(ui, &mut text);
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-question-text-editor"), input.rect));
            if input.changed() {
                *value = json!(text);
            }
            return submit;
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
                crate::tr!("Dieser Antworttyp wird hier nicht unterstützt."),
            );
        }
    }
    false
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
        return Err(crate::tr!("Bitte einen gültigen Wert eingeben").into());
    }
    if let Some(text) = value.as_str() {
        let len = text.chars().count() as u64;
        if (schema["required"] == true && text.trim().is_empty())
            || schema["minLength"].as_u64().is_some_and(|n| len < n)
            || schema["maxLength"].as_u64().is_some_and(|n| len > n)
        {
            return Err(crate::tr!("Textlänge außerhalb der erlaubten Grenzen").into());
        }
    }
    if let Some(n) = value.as_f64() {
        if schema["minimum"].as_f64().is_some_and(|v| n < v)
            || schema["maximum"].as_f64().is_some_and(|v| n > v)
        {
            return Err(crate::tr!("Zahl außerhalb des erlaubten Bereichs").into());
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
            return Err(crate::tr!("Anzahl der Auswahlwerte außerhalb der Grenzen").into());
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
            return Err(crate::tr!("Bitte eine angebotene Option auswählen").into());
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
            ui.checkbox(v, crate::tr!("Aktiviert"));
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
                        crate::tr!("Ungültiger JSON-Wert. Bitte korrigieren; Senden ist gesperrt."),
                    );
                    false
                }
            }
        }
    })
    .inner
}

fn management_editor(ui: &mut egui::Ui, value: &mut Value, key: &str, app: &App, staged: bool) -> bool {
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
            if name == "request_id"
                || (staged && matches!(name.as_str(), "expected_revision" | "expected_stream_revision"))
                || (app.models.managed() && key == "profile"
                && matches!(name.as_str(), "provider_id" | "model_id")) {
                continue;
            }
            let v = map.get_mut(&name).expect("existing key");
            ui.push_id(&name, |ui| {
                ui.label(field_label(&name));
                valid &= management_editor(ui, v, &name, app, staged);
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
                (json!("local"), crate::tr!("Auf diesem Rechner").into()),
                (json!("ssh"), crate::tr!("Über das SSH-Profil des Runners").into()),
            ]),
            "status" => Some(
                [
                    ("backlog", "Backlog"),
                    ("ready", crate::tr!("Bereit")),
                    ("in_progress", crate::tr!("In Bearbeitung")),
                    ("blocked", crate::tr!("Blockiert")),
                    ("review", "Review"),
                    ("done", crate::tr!("Abgeschlossen")),
                    ("cancelled", crate::tr!("Abgebrochen")),
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
                        crate::tr!("Keine Auswahl").into()
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
                            crate::tr!("Keine Bindung (nur bei gerätebezogener Runner-Rolle erforderlich)"),
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
                    .map(|t| (json!(t.id), task_summary(s, t)))
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
        if ui.checkbox(&mut unlimited, crate::tr!("Unbegrenzt")).changed() {
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

pub(crate) fn devices(ui: &mut egui::Ui, app: &mut App, devices: &[Device]) {
    let active: Vec<_> = devices.iter().filter(|device| !device.revoked).collect();
    if active.is_empty() { ui.label(crate::tr!("Keine aktiven Gerätezugänge.")); }
    for device in active {
        design::task_row_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let revoke = ui.add_enabled(app.online && app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin),
                        egui::Button::new(crate::tr!("Widerrufen …")));
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-device-revoke", device.id)), revoke.rect));
                    if revoke.clicked() {
                        let own = app.access.as_ref().is_some_and(|a| a.device_id == Some(device.id));
                        app.dialog = Some(Dialog::form(crate::tr!("Gerätezugang widerrufen"),
                            &crate::tr!("Zugang für „{}“ dauerhaft sperren? Das Gerät kann sich danach nicht mehr verbinden.{}", device.name, if own { crate::tr!(" Das betrifft auch diese Desktop-Verbindung.") } else { "" }),
                            "POST", format!("/v1/devices/{}/revoke", device.id), Value::Null));
                    }
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 0.0),
                        egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&device.name).strong()).truncate())
                                .on_hover_text(&device.name);
                            ui.colored_label(MUTED, match device.role {
                                DeviceRole::Admin => "Administration",
                                DeviceRole::Operator => crate::tr!("Bedienung"),
                                DeviceRole::Runner => crate::tr!("Runner-Zugang"),
                            });
                            if app.access.as_ref().is_some_and(|a| a.device_id == Some(device.id)) {
                                ui.label(crate::tr!("Dieser Desktop"));
                            }
                        });
                });
            });
        });
    }
}

pub(crate) fn read_content(ui: &mut egui::Ui, app: &mut App, value: &Value, path: &str) {
    if path.starts_with("ssh-profile/") {
        let mut body = value.clone();
        if body["profile"].is_null() {
            body["profile"] = json!({"host":"","server":"http://127.0.0.1:7432","journal":"","opencode_registration":null});
        }
        ui.label(crate::tr!("Lokales SSH-Profil für nichtinteraktive Runner-Helfer. Hostschlüssel und Zugangsdaten bleiben in der normalen SSH-Konfiguration."));
        if ui.button(crate::tr!("SSH-Profil bearbeiten …")).clicked() {
            app.dialog = Some(Dialog::form(crate::tr!("SSH-Profil speichern"), crate::tr!("Absolute Pfade auf dem Runnerhost angeben. Eine zwischenzeitliche Änderung desselben Profils wird nicht überschrieben."), "LOCAL_SSH_SAVE", path.into(), body));
        }
        if !value["profile"].is_null() && ui.button(crate::tr!("SSH-Profil entfernen …")).clicked() {
            let mut body = value.clone();
            body["profile"] = Value::Null;
            app.dialog = Some(Dialog::form(
                crate::tr!("SSH-Profil entfernen"),
                crate::tr!("Die lokale SSH-Zuordnung entfernen; keine Runner oder Sessions werden beendet."),
                "LOCAL_SSH_SAVE",
                path.into(),
                body,
            ));
        }
    } else if let Ok(question) = serde_json::from_value::<Question>(value.clone()) {
        for (index, (question, field)) in question_pairs(&question).enumerate() {
            if index > 0 { ui.separator(); }
            question_pair(ui, question, field);
        }
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
                for (label, kind) in [(crate::tr!("Vorbereiten …"), CommandKind::PrepareWorkspace), (crate::tr!("OpenCode prüfen …"), CommandKind::CheckOpenCode)] {
                    if ui.button(label).clicked() { app.dialog = Some(Dialog::form(label, crate::tr!("Technischen Auftrag an diesen Arbeitsbereich senden. Die Serverrevision schützt vor parallelen Änderungen."), "POST", format!("/v1/streams/{}/commands", w.id), json!(EnqueueCommand { request_id: RequestId::new(), expected_revision: w.revision, kind, prompt: None }))); }
                }
                for c in snapshot.commands.iter().filter(|c| c.stream_id == w.id) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{:?} · {}", c.kind, command_status(c)));
                        ui.menu_button(crate::tr!("Verwalten …"), |ui| app.command_menu(ui, c));
                    });
                    ui.collapsing(crate::tr!("Auftrag {}", c.id), |ui| json_read(ui, &json!(c)));
                }
            });
        }
    } else if path.ends_with("/runtime-binding") {
        if let Ok(binding) = serde_json::from_value::<RuntimeBinding>(value.clone()) {
            ui.label(crate::tr!("Container {} · gestartet {}", binding.runtime.container_id, binding.runtime.started_at));
            if ui.button(crate::tr!("Diese Runtime gezielt beenden …")).clicked() {
                let revision = app
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.streams.iter().find(|w| w.id == binding.stream_id))
                    .map_or(Revision(0), |w| w.revision);
                app.dialog = Some(Dialog::form(crate::tr!("Runtime unwiderruflich beenden"), crate::tr!("Nur die bestätigte dedizierte Runtime wird entfernt. Hintergrundprozesse werden erst nach bestätigtem Stopp als beendet behandelt. Worktree und Volumes bleiben erhalten; Archiv kann unvollständig sein."), "POST", format!("/v1/commands/{}/termination", binding.command_id), json!(RequestTermination { request_id: RequestId::new(), expected_stream_revision: revision, runtime: binding.runtime, previous_request: None, reason: String::new() })));
            }
        } else {
            ui.label(crate::tr!("Keine bestätigte dedizierte Container-Runtime verfügbar. Native Sessionsteuerung ist getrennt über den Arbeitsbereich möglich."));
        }
    } else if path.ends_with("/files") {
        if let Ok(index) = serde_json::from_value::<ArchiveBlobIndex>(value.clone()) {
            let command = path
                .split('/')
                .nth(3)
                .and_then(|s| s.parse::<CommandId>().ok());
            if index.files.is_empty() {
                ui.label(crate::tr!("Keine separat gesicherten Dateien. Inline-Anhänge und Unverfügbarkeitsgründe stehen zusätzlich bei der Originalnachricht."));
            }
            for file in index.files {
                app::card().show(ui, |ui| {
                    ui.strong(&file.name);
                    ui.label(crate::tr!("{} · {} Bytes · erfasst {}", file.mime,
                        file.bytes,
                        timestamp(file.captured_unix_ms)));
                    ui.colored_label(MUTED, &file.sha256);
                    if let Some(command) = command {
                        if ui
                            .add_enabled(
                                app.online,
                                egui::Button::new(crate::tr!("SHA256-geprüft exportieren …")),
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
    } else if path == "/v1/devices" {
        if let Ok(devices) = serde_json::from_value::<Vec<Device>>(value.clone()) {
            self::devices(ui, app, &devices);
        }
        return;
    } else if let Ok(notes) = serde_json::from_value::<Vec<KnowledgeNote>>(value.clone()) {
        for note in notes {
            app::card().show(ui, |ui| {
            ui.strong(&note.title); ui.label(&note.content); ui.colored_label(MUTED, crate::tr!("Quelle: {} · Branch: {:?} · abgelöst: {}", note.source, note.branch, note.retired));
            if !note.retired && ui.button(crate::tr!("Wissen ablösen …")).clicked() { app.dialog = Some(Dialog::form(crate::tr!("Wissen ablösen"), crate::tr!("Der Eintrag bleibt historisch erhalten, wird aber nicht mehr automatisch als aktuelles Wissen verwendet."), "POST", format!("/v1/knowledge/{}/retire", note.id), json!(RetireKnowledge { request_id: RequestId::new(), expected_revision: note.revision }))); }
            ui.collapsing(crate::tr!("Metadaten"), |ui| json_read(ui, &json!(note)));
        });
        }
    } else if let Ok(policy) = serde_json::from_value::<SupervisorPolicy>(value.clone()) {
        ui.label(crate::tr!("{} · {} Auswertungen verbleibend · Intervall {} s", if policy.enabled {
                crate::tr!("Aktiv")
            } else {
                crate::tr!("Deaktiviert")
            },
            policy.remaining_evaluations,
            policy.interval_seconds));
    } else if let Ok(budget) = serde_json::from_value::<TokenBudget>(value.clone()) {
        if ui.button(crate::tr!("Tokenlimits ändern …")).clicked() {
            app.dialog = Some(Dialog::form(crate::tr!("Tokenbudget ändern"), crate::tr!("Limits gelten für beobachtete Tokenverbräuche. Fehlende historische Angaben sind keine Nullverbräuche. null bedeutet unbegrenzt."), "PUT", format!("/v1/projects/{}/token-budget", budget.project_id), json!(SetTokenBudget { request_id: RequestId::new(), expected_revision: budget.revision, limits: budget.limits })));
        }
    }
    ui.collapsing(crate::tr!("Vollständige Daten"), |ui| json_read(ui, value));
}
fn field_label(key: &str) -> &str {
    match key {
        "key" => crate::tr!("Projektkürzel (z. B. TEST oder WOAI; neu: leer = automatisch)"),
        "confirm_name" => crate::tr!("Projektname zur Bestätigung"),
        "expected_revision" | "expected_stream_revision" => {
            crate::tr!("Erwartete Serverrevision (bei Konflikt aktuellen Stand prüfen)")
        }
        "name" => "Name",
        "repository" => crate::tr!("Repository / Quelle"),
        "base_branch" => crate::tr!("Basisbranch"),
        "provider_id" => crate::tr!("Anbieter-ID"),
        "model_id" => crate::tr!("Modell-ID"),
        "toolchain" => "Toolchain",
        "start_command" => crate::tr!("Startbefehl"),
        "test_command" => crate::tr!("Testbefehl"),
        "enabled" => crate::tr!("Automatik aktiv"),
        "paused" => crate::tr!("Pausiert"),
        "runner_ids" => "Runner-IDs",
        "runner_id" => "Runner-ID",
        "max_parallel" => crate::tr!("Maximal parallele Aufträge"),
        "max_sessions" => crate::tr!("Maximal arbeitende Sessions"),
        "evaluation_budget" => crate::tr!("Auswertungsbudget"),
        "interval_seconds" => crate::tr!("Intervall in Sekunden"),
        "stream_id" => crate::tr!("Arbeitsbereich-ID"),
        "reason" => crate::tr!("Begründung"),
        "goal" => crate::tr!("Ziel"),
        "title" => crate::tr!("Titel"),
        "description" => crate::tr!("Beschreibung"),
        "content" => crate::tr!("Inhalt"),
        "acceptance_criteria" => crate::tr!("Erfolgskriterien"),
        "priority" => crate::tr!("Priorität (0–3)"),
        "ticket_ids" => crate::tr!("Aufgaben-IDs"),
        "depends_on" => crate::tr!("Abhängigkeiten (Aufgaben-IDs)"),
        "role" => crate::tr!("Rolle"),
        "ttl_seconds" => crate::tr!("Gültigkeit in Sekunden"),
        "repository_path" => crate::tr!("Repositorypfad auf dem Runner"),
        "service_registration" => crate::tr!("OpenCode-Service-Registrierung auf dem Runner"),
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
    fn project_usage_counts_input_and_output_across_all_days_since_first_sample() {
        let start = chrono::DateTime::from_timestamp_millis(1_750_000_000_000).unwrap()
            .with_timezone(&chrono::Local).date_naive();
        let report = ProjectUsage {
            project_id: ProjectId::new(),
            groups: vec![UsageGroup {
                assistant_messages: 1,
                tokens: TokenUsage {
                    input: 2400, output: 600, reasoning: 700,
                    cache: CacheUsage { read: 1100, write: 500 },
                },
                ..Default::default()
            }],
            first_observed_unix_ms: Some(1_750_000_000_000),
            token_budget: None,
            source: String::new(),
        };
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                project_usage_summary(ui, &report, None, start + chrono::Days::new(2));
            });
        });
        let mut text = Vec::new();
        for shape in &output.shapes {
            crate::tests::shape_text(&shape.shape, &mut text);
        }
        assert!(text.contains(&"3.000".into()), "{text:?}");
        assert!(text.contains(&start.format("%d.%m.%Y").to_string()), "{text:?}");
        assert!(text.contains(&"1.000 Tokens".into()), "{text:?}");
        let without_observation = ProjectUsage { first_observed_unix_ms: None, ..report };
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                project_usage_summary(ui, &without_observation, Some(1_750_000_000_000), start + chrono::Days::new(2));
            });
        });
        text.clear();
        for shape in &output.shapes {
            crate::tests::shape_text(&shape.shape, &mut text);
        }
        assert!(text.contains(&"Berechnet seit Projektanlage".into()), "{text:?}");
        assert!(text.contains(&start.format("%d.%m.%Y").to_string()), "{text:?}");
        assert!(text.contains(&"1.000 Tokens".into()), "{text:?}");
    }

    #[test]
    fn project_editor_supports_managed_local_runner_and_git_sources() {
        let mut app = crate::tests::fixture();
        app.project_editor(None);
        let Some(Dialog::Form(mut form)) = app.dialog.take() else { panic!("expected project form") };
        form.body["name"] = json!(" Neues Projekt ");
        form.body["key"] = json!(" woai ");
        let managed = form.project_request(&app).unwrap();
        assert_eq!(managed["repository"], "factory:managed");
        assert_eq!(managed["base_branch"], "main");
        assert!(managed.get("_repository_kind").is_none());
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
        form.body["repository"] = json!(path);
        form.body["_repository_kind"] = json!("local");
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["name"], "Neues Projekt");
        assert_eq!(request["key"], "WOAI");
        assert_eq!(request["repository"], path);
        assert_eq!(request["base_branch"], "develop");
        assert!(request.get("setup").is_none());
        assert!(request.get("runner_id").is_none());
        assert!(request.get("runner_mapping").is_none());
        form.body["_repository_kind"] = json!("runner");
        form.body["repository"] = json!("/srv/projects/example");
        form.body["base_branch"] = json!("release");
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["repository"], "/srv/projects/example");
        assert_eq!(request["base_branch"], "release");
        form.body["_repository_kind"] = json!("git");
        form.body["repository"] = json!("git@example.test:team/project.git");
        form.body["base_branch"] = json!("develop");
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["repository"], "git@example.test:team/project.git");
        assert_eq!(request["base_branch"], "develop");
        let runner = app.snapshot.as_ref().unwrap().runners[0].id;
        form.body["runner_mapping"] = json!(ProjectRunnerMapping::Runner(runner));
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["runner_mapping"], json!({"runner": runner}));
        let nested = dir.path().join("unterordner");
        assert_eq!(local_project_branch(nested.to_str().unwrap()).unwrap(), "main");
        assert!(!nested.exists());
        std::fs::create_dir(&nested).unwrap();
        assert_eq!(local_project_branch(nested.to_str().unwrap()).unwrap(), "main");
        assert!(!nested.join(".git").exists(), "Only the runner may initialize Git");
        std::fs::write(nested.join("keep.txt"), "preserve").unwrap();
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
        assert_eq!(form.body["repository"], "git@example.test:repo");
        form.body["name"] = json!("Umbenannt");
        form.body["key"] = json!("new");
        let runner = app.snapshot.as_ref().unwrap().runners[0].id;
        form.body["runner_mapping"] = json!(ProjectRunnerMapping::Runner(runner));
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["name"], "Umbenannt");
        assert_eq!(request["key"], "NEW");
        assert!(request["repository"].is_null());
        assert!(request["base_branch"].is_null());
        assert!(request.get("profile").is_none());
        assert!(request.get("runner_mapping").is_none());
        form.persist(&mut app);
        app.project_editor(Some(&project));
        assert!(matches!(&app.dialog, Some(Dialog::Form(form)) if form.body["name"] == "Umbenannt"));
    }

    #[test]
    fn runner_mapping_request_does_not_update_general_fields() {
        let app = crate::tests::fixture();
        let project = app.project().unwrap();
        let runner = app.snapshot.as_ref().unwrap().runners[0].id;
        let Dialog::Form(form) = Dialog::form(
            "Runner-Zuordnung",
            "",
            "PATCH",
            format!("/v1/projects/{}", project.id),
            json!({
                "request_id": RequestId::new(),
                "expected_revision": project.revision,
                "runner_mapping": ProjectRunnerMapping::Runner(runner),
            }),
        ) else { unreachable!() };
        let request = form.project_request(&app).unwrap();
        assert_eq!(request["runner_mapping"], json!({"runner": runner}));
        assert_eq!(request.as_object().unwrap().len(), 3);
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
