use crate::{
    app::{App, MUTED, RED},
    design,
    model::Page,
    project_settings::{ProjectSection, ProjectSettings},
};
use factory_core::{DeviceRole, ProjectId, RequestId};
use factory_protocol::models::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub struct Load<T> {
    pub value: Option<T>,
    request: Option<u64>,
    pub error: Option<String>,
}
impl<T> Default for Load<T> {
    fn default() -> Self {
        Self {
            value: None,
            request: None,
            error: None,
        }
    }
}

impl<T: DeserializeOwned> Load<T> {
    pub fn fetch(&mut self, app: &mut App, path: &str) {
        self.request = Some(ephemeral(app, "GET", path.into(), Value::Null));
        self.error = None;
    }

    pub fn settle(&mut self, app: &mut App) {
        let Some(result) = self.request.and_then(|id| app.responses.remove(&id)) else {
            return;
        };
        self.request = None;
        match result {
            Ok(value) => match serde_json::from_value(value) {
                Ok(value) => self.value = Some(value),
                Err(_) => self.error = Some("Unerwartete Antwort der Factory.".into()),
            },
            Err(error) => self.error = Some(error.message),
        }
    }

    pub fn load(&mut self, app: &mut App, path: &str) {
        self.settle(app);
        if app.online && self.request.is_none() && self.error.is_none() && self.value.is_none() {
            self.fetch(app, path);
        }
    }
}

#[derive(serde::Deserialize)]
struct Setup {
    managed: bool,
}

#[derive(Default)]
pub struct Models {
    setup: Load<Setup>,
    pub factory: Load<FactoryModelSettings>,
    pub projects: BTreeMap<ProjectId, Load<ProjectModelSettings>>,
    project_panes: BTreeMap<ProjectId, ProjectPane>,
    prompted: bool,
    next_refresh: Option<Instant>,
    next_status: Option<Instant>,
    pane: FactoryPane,
}

impl Models {
    pub fn managed(&self) -> bool {
        self.setup.value.as_ref().is_some_and(|s| s.managed)
    }

    pub fn needs_setup(&self) -> bool {
        self.managed()
            && self
                .factory
                .value
                .as_ref()
                .is_some_and(|s| s.default_model.is_none())
    }

    fn tick(&mut self, app: &mut App) {
        self.setup.load(app, "/v1/model-settings/setup");
        if self.managed() {
            self.factory.load(app, "/v1/model-settings");
            if app.online
                && self.factory.request.is_none()
                && self.next_refresh.is_none_or(|at| at <= Instant::now())
            {
                self.factory.fetch(app, "/v1/model-settings");
                self.next_refresh = Some(Instant::now() + Duration::from_secs(15));
            }
            let mut pane = std::mem::take(&mut self.pane);
            if let Some(settings) = &self.factory.value {
                let refresh = app.online && self.next_status.is_none_or(|at| at <= Instant::now());
                if refresh {
                    self.next_status = Some(Instant::now() + Duration::from_secs(30));
                }
                for access in &settings.accesses {
                    let status = pane.status.entry(access.id.clone()).or_default();
                    let path = format!("/v1/model-settings/accesses/{}/status", access.id);
                    status.load(app, &path);
                    if refresh && status.request.is_none() {
                        status.fetch(app, &path);
                    }
                }
                pane.status
                    .retain(|id, _| settings.accesses.iter().any(|a| &a.id == id));
            }
            self.pane = pane;
        }
        if app.online && !self.prompted && self.needs_setup() && app.dialog.is_none() {
            self.prompted = true;
            app.dialog = Some(crate::dialogs::Dialog::ModelSetupRequired);
        }
    }

    pub fn ready_catalog(&self) -> Vec<ModelAccess> {
        self.factory
            .value
            .iter()
            .flat_map(|s| &s.accesses)
            .filter(|access| {
                self.pane
                    .status
                    .get(&access.id)
                    .and_then(|s| s.value.as_ref())
                    .is_some_and(|status| status.state == "ready")
            })
            .cloned()
            .collect()
    }
}

pub fn tick(app: &mut App) {
    let mut state = std::mem::take(&mut app.models);
    state.tick(app);
    app.models = state;
}

#[cfg(test)]
pub(crate) fn seed(app: &mut App, default: bool) {
    use factory_core::Revision;
    let access = ModelAccess {
        id: "factory-test".into(),
        name: "OpenAI Arbeit".into(),
        models: vec!["demo".into()],
    };
    app.models.setup.value = Some(Setup { managed: true });
    app.models.factory.value = Some(FactoryModelSettings {
        revision: Revision(1),
        accesses: vec![access.clone()],
        roles: RoleModels::default(),
        default_model: default.then(|| ModelSelection {
            access_id: access.id.clone(),
            model_id: "demo".into(),
        }),
    });
    app.models.pane.status.entry(access.id).or_default().value = Some(Status {
        state: "ready".into(),
    });
}

pub fn open_setup(app: &mut App) {
    open_model_section(app, ProjectSection::Models);
}

pub fn open_defaults(app: &mut App) {
    open_model_section(app, ProjectSection::DefaultModels);
}

fn open_model_section(app: &mut App, section: ProjectSection) {
    app.set_page(Page::Settings);
    app.factory_settings
        .get_or_insert_with(ProjectSettings::factory)
        .selected = section;
}

pub fn ephemeral(app: &mut App, method: &str, path: String, body: Value) -> u64 {
    let id = app.next_request;
    app.next_request += 1;
    app.in_flight.insert(id);
    // OAuth attempt responses and codes must never enter the durable outbox.
    app.backend.request(id, method.into(), path, body);
    id
}

pub fn submitted(
    path: &str,
    value: &Value,
) -> Option<(factory_protocol::SubmitTask, Option<ModelSelection>)> {
    match path {
        "/v1/tasks/submit" => serde_json::from_value(value.clone())
            .ok()
            .map(|task| (task, None)),
        "/v1/tasks/submit-with-model" => {
            serde_json::from_value::<SubmitTaskWithModel>(value.clone())
                .ok()
                .map(|body| (body.submission, body.implementer))
        }
        _ => None,
    }
}

pub fn pending_task<'a>(path: &str, value: &'a Value) -> Option<&'a Value> {
    match path {
        "/v1/tasks/submit" => value.get("task"),
        "/v1/tasks/submit-with-model" => value.get("submission")?.get("task"),
        _ => None,
    }
}

pub fn effective_implementer(app: &App, project: ProjectId) -> Option<&ModelSelection> {
    let models = app.models.factory.value.as_ref()?;
    let project = app.models.projects.get(&project)?.value.as_ref()?;
    project
        .roles
        .implementer
        .as_ref()
        .or(models.roles.implementer.as_ref())
        .or(models.default_model.as_ref())
}

pub fn label(catalog: &[ModelAccess], model: &ModelSelection) -> String {
    let name = catalog
        .iter()
        .find(|a| a.id == model.access_id)
        .map_or(model.access_id.as_str(), |a| &a.name);
    format!("{name} · {}", model.model_id)
}

pub fn choice(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    catalog: &[ModelAccess],
    value: &mut Option<ModelSelection>,
    inherited: &str,
) -> bool {
    let mut changed = false;
    let selected = value
        .as_ref()
        .map_or_else(|| inherited.into(), |model| label(catalog, model));
    egui::ComboBox::from_id_salt(id)
        .selected_text(&selected)
        .width(ui.available_width().clamp(132.0, 420.0))
        .wrap_mode(egui::TextWrapMode::Truncate)
        .show_ui(ui, |ui| {
            ui.set_width(ui.available_width().max(180.0));
            if design::navigation_row(ui, inherited, value.is_none(), None).clicked() {
                changed |= value.take().is_some();
                ui.close();
            }
            for access in catalog {
                for model in &access.models {
                    let next = ModelSelection {
                        access_id: access.id.clone(),
                        model_id: model.clone(),
                    };
                    let title = label(catalog, &next);
                    if design::navigation_row(ui, &title, value.as_ref() == Some(&next), None)
                        .on_hover_text(title)
                        .clicked()
                    {
                        changed |= value.as_ref() != Some(&next);
                        *value = Some(next);
                        ui.close();
                    }
                }
            }
        })
        .response
        .on_hover_text(selected)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    changed
}

fn roles(ui: &mut egui::Ui, scope: &str, catalog: &[ModelAccess], models: &mut RoleModels) {
    for (name, selection) in [
        ("Orchestrator", &mut models.orchestrator),
        ("Implementer", &mut models.implementer),
        ("Supervisor", &mut models.supervisor),
        ("Memorizer", &mut models.memorizer),
    ] {
        ui.label(name);
        choice(ui, (scope, name), catalog, selection, "Default");
        ui.add_space(8.0);
    }
}

#[derive(serde::Deserialize)]
struct Status {
    state: String,
}

#[derive(serde::Deserialize)]
struct Prompt {
    #[serde(rename = "attemptID")]
    id: String,
    url: String,
    instructions: String,
    mode: String,
}

struct Login {
    access: String,
    begin: u64,
    prompt: Option<Prompt>,
    code: String,
    finish: Option<u64>,
    cancelling: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Provider {
    #[default]
    ChatGpt,
}

impl Provider {
    const AVAILABLE: [Self; 1] = [Self::ChatGpt];

    fn label(self) -> &'static str {
        match self {
            Self::ChatGpt => "OpenAI (ChatGPT)",
        }
    }

    fn new_access(self) -> (String, String) {
        match self {
            Self::ChatGpt => {
                let id = format!("factory-openai-{}", RequestId::new());
                let path = format!("{}/start", oauth_path(&id));
                (id, path)
            }
        }
    }
}

#[derive(Default)]
struct FactoryPane {
    draft: Option<FactoryModelSettings>,
    save: Option<u64>,
    name: String,
    naming: bool,
    focus_name: bool,
    provider: Provider,
    status: BTreeMap<String, Load<Status>>,
    login: Option<Login>,
}

pub fn show_factory(ui: &mut egui::Ui, app: &mut App, defaults_bottom: Option<f32>) {
    let mut pane = std::mem::take(&mut app.models.pane);
    pane.show(ui, app, defaults_bottom);
    app.models.pane = pane;
}

pub fn show_add_dialog(ctx: &egui::Context, app: &mut App) {
    let mut pane = std::mem::take(&mut app.models.pane);
    if pane.naming {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("name-model-access"))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING))
            .show(ctx, |ui| {
                ui.set_width(
                    design::dialog_content_size(ctx.content_rect().size())
                        .x
                        .min(380.0),
                );
                close = design::dialog_header(ui, "OpenAI-Zugang benennen").clicked();
                ui.separator();
                ui.label("Name");
                let input = ui.add(
                    design::singleline(&mut pane.name)
                        .hint_text(design::input_hint("z. B. OpenAI Arbeit"))
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut pane.focus_name) {
                    input.request_focus();
                }
                let name = pane.name.trim();
                let duplicate = app.models.factory.value.as_ref().is_some_and(|settings| {
                    settings
                        .accesses
                        .iter()
                        .any(|a| a.name.trim().to_lowercase() == name.to_lowercase())
                });
                if duplicate {
                    ui.colored_label(RED, "Dieser Name ist bereits vergeben.");
                }
                let valid = app.online
                    && !name.is_empty()
                    && name.len() <= 120
                    && !name.chars().any(char::is_control)
                    && !duplicate;
                let submit = design::accent_button(
                    ui,
                    egui::Button::new("Weiter zur Anmeldung"),
                    valid,
                    crate::app::BLUE.gamma_multiply(0.7),
                )
                .clicked();
                if valid
                    && (submit
                        || (input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                {
                    let (access, path) = pane.provider.new_access();
                    let begin = ephemeral(app, "POST", path, json!({"name":name}));
                    pane.login = Some(Login {
                        access,
                        begin,
                        prompt: None,
                        code: String::new(),
                        finish: None,
                        cancelling: false,
                    });
                    close = true;
                }
            });
        pane.naming = !(close || response.should_close());
    } else {
        pane.show_login(ctx, app);
    }
    app.models.pane = pane;
}

impl FactoryPane {
    fn show(&mut self, ui: &mut egui::Ui, app: &mut App, defaults_bottom: Option<f32>) {
        if !app.models.managed() {
            if let Some(error) = &app.models.setup.error {
                ui.colored_label(RED, error);
            } else if app.models.setup.value.is_some() {
                ui.label("Diese Factory wurde ohne zentralen Modell-Gateway gestartet.");
            } else {
                ui.label("Verbindung und OpenCode-Einrichtung werden geprüft …");
            }
            return;
        }
        if self.save.is_none() {
            self.save = app
                .settings
                .pending
                .iter()
                .find(|p| p.server == app.server && p.path == "/v1/model-settings")
                .map(|p| p.id);
        }
        if let Some(result) = settled_save(app, self.save) {
            self.save = None;
            match result {
                Ok(value) => match serde_json::from_value::<FactoryModelSettings>(value) {
                    Ok(saved) => {
                        self.draft = None;
                        app.models.factory.value = Some(saved);
                        app.models.factory.request = None;
                        app.models.next_refresh = None;
                    }
                    Err(_) => app.notice("Speichern nicht eindeutig bestätigt."),
                },
                Err(_) => app.notice("Die Default-Modelle konnten nicht gespeichert werden."),
            }
        }
        let Some(saved) = app.models.factory.value.clone() else {
            ui.label("Modell-Anbieter werden geladen …");
            if let Some(error) = &app.models.factory.error {
                ui.colored_label(RED, error);
            }
            return;
        };
        let admin = app
            .access
            .as_ref()
            .is_some_and(|a| a.role == DeviceRole::Admin);
        let editable = admin && app.online && self.save.is_none();
        if let Some(bottom) = defaults_bottom {
            self.show_defaults(ui, app, saved, editable, bottom);
            return;
        }
        if !admin {
            ui.colored_label(
                MUTED,
                "Nur Factory-Admins können Zugänge und Standardmodelle ändern.",
            );
        }
        ui.add_space(8.0);
        design::section_heading(ui, "Anbieter hinzufügen");
        ui.horizontal(|ui| {
            ui.add_enabled_ui(editable && self.login.is_none(), |ui| {
                egui::ComboBox::from_id_salt("add-model-provider")
                    .selected_text(self.provider.label())
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for provider in Provider::AVAILABLE {
                            if design::navigation_row(
                                ui,
                                provider.label(),
                                provider == self.provider,
                                None,
                            )
                            .clicked()
                            {
                                self.provider = provider;
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if ui.button("Hinzufügen").clicked() {
                    self.naming = true;
                    self.focus_name = true;
                }
            });
        });
        ui.add_space(12.0);
        design::section_heading(ui, "Vorhandene Zugänge");
        if saved.accesses.is_empty() {
            ui.colored_label(MUTED, "Noch keine Zugänge verbunden.");
        }
        for access in &saved.accesses {
            let status = self.status.entry(access.id.clone()).or_default();
            status.load(
                app,
                &format!("/v1/model-settings/accesses/{}/status", access.id),
            );
            ui.horizontal_wrapped(|ui| {
                ui.strong(&access.name);
                match status.value.as_ref().map(|status| status.state.as_str()) {
                    Some("ready") => {
                        ui.colored_label(crate::app::GREEN, "Verbunden");
                    }
                    Some(state) => {
                        if state != "not_connected" {
                            ui.colored_label(RED, "Zugang nicht verfügbar");
                        }
                        let action = if state == "not_connected" {
                            "Mit ChatGPT verbinden"
                        } else {
                            "Erneut anmelden"
                        };
                        if ui
                            .add_enabled(
                                editable && self.login.is_none(),
                                egui::Button::new(action),
                            )
                            .clicked()
                        {
                            let begin = ephemeral(app, "POST", oauth_path(&access.id), Value::Null);
                            self.login = Some(Login {
                                access: access.id.clone(),
                                begin,
                                prompt: None,
                                code: String::new(),
                                finish: None,
                                cancelling: false,
                            });
                        }
                    }
                    None => {
                        ui.colored_label(
                            MUTED,
                            status.error.as_deref().unwrap_or("Wird geprüft …"),
                        );
                    }
                }
            });
        }
    }

    fn show_defaults(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut App,
        saved: FactoryModelSettings,
        editable: bool,
        bottom: f32,
    ) {
        let mut draft = self.draft.clone().unwrap_or_else(|| saved.clone());
        let state = |a: &ModelAccess| {
            self.status
                .get(&a.id)
                .and_then(|s| s.value.as_ref())
                .map(|s| s.state.as_str())
        };
        let ready: Vec<_> = saved
            .accesses
            .iter()
            .filter(|a| state(a) == Some("ready"))
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("factory-default-models-fields")
            .max_height(design::settings_body_height(ui, bottom))
            .show(ui, |ui| {
                design::section_heading(ui, "Standardmodell");
                if saved
                    .accesses
                    .iter()
                    .all(|a| matches!(state(a), Some("not_connected" | "unavailable")))
                {
                    ui.label(egui::RichText::new("Verbinde zuerst einen Anbieter.").color(MUTED));
                    ui.add_space(6.0);
                }
                ui.add_enabled_ui(editable, |ui| {
                    choice(
                        ui,
                        "factory-default",
                        &ready,
                        &mut draft.default_model,
                        "Nicht eingerichtet",
                    );
                });
                ui.add_space(16.0);
                design::section_heading(ui, "Agenten Overrides");
                ui.label(egui::RichText::new("Überschreibe die Modelle pro Agent").color(MUTED));
                ui.add_space(6.0);
                ui.add_enabled_ui(editable, |ui| {
                    roles(ui, "factory-models", &ready, &mut draft.roles);
                });
            });
        self.draft = (draft != saved).then(|| draft.clone());
        if model_save_footer(ui, app, bottom, editable && self.draft.is_some(), self.save) {
            self.send(app, draft);
        }
    }

    fn send(&mut self, app: &mut App, settings: FactoryModelSettings) {
        self.save = Some(app.write(
            "Factory-Modelle speichern",
            "PUT",
            "/v1/model-settings".into(),
            json!(SetFactoryModels {
                request_id: RequestId::new(),
                settings
            }),
        ));
    }

    fn show_login(&mut self, ctx: &egui::Context, app: &mut App) {
        let Some(mut login) = self.login.take() else {
            return;
        };
        if let Some(result) = app.responses.remove(&login.begin) {
            match result {
                Ok(value) => match serde_json::from_value(value) {
                    Ok(prompt) => {
                        let prompt: Prompt = prompt;
                        if !login.cancelling {
                            ctx.open_url(egui::OpenUrl::new_tab(&prompt.url));
                            if prompt.mode == "auto" {
                                login.finish = Some(ephemeral(
                                    app,
                                    "POST",
                                    format!("{}/complete", oauth_path(&login.access)),
                                    json!({"attempt_id":prompt.id,"code":null}),
                                ));
                            }
                        }
                        login.prompt = Some(prompt);
                    }
                    Err(_) => {
                        app.notice(
                            "Die Factory hat keine gültige ChatGPT-Anmeldung zurückgegeben.",
                        );
                        return;
                    }
                },
                Err(error) => {
                    if !login.cancelling {
                        app.notice(error.message);
                    }
                    return;
                }
            }
        }
        if let Some(result) = login.finish.and_then(|id| app.responses.remove(&id)) {
            match result {
                Ok(value) if value["cancelled"] == true => {
                    let mut factory = std::mem::take(&mut app.models.factory);
                    factory.fetch(app, "/v1/model-settings");
                    app.models.factory = factory;
                }
                Ok(value) if value["connected"] == true => {
                    self.status.remove(&login.access);
                    let automatic = serde_json::from_value::<ModelSelection>(
                        value["automatic_default"].clone(),
                    )
                    .ok();
                    let access_name = self.name.trim().to_owned();
                    self.name.clear();
                    let mut factory = std::mem::take(&mut app.models.factory);
                    factory.fetch(app, "/v1/model-settings");
                    app.models.factory = factory;
                    if let Some(model) = automatic {
                        app.dialog = Some(crate::dialogs::Dialog::ModelDefaultsConfigured {
                            model: if access_name.is_empty() {
                                model.model_id
                            } else {
                                format!("{access_name} · {}", model.model_id)
                            },
                        });
                    } else {
                        app.notice("ChatGPT-Zugang verbunden.");
                    }
                }
                Ok(_) => app.notice("Die ChatGPT-Anmeldung wurde nicht bestätigt."),
                Err(error) => {
                    self.status.remove(&login.access);
                    app.notice(error.message);
                }
            }
            return;
        }
        if login.cancelling && login.finish.is_none() {
            login.cancel(app);
        }
        let modal = egui::Modal::new(egui::Id::new("chatgpt-login"))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING))
            .show(ctx, |ui| {
                ui.set_width(
                    design::dialog_content_size(ctx.content_rect().size())
                        .x
                        .min(380.0),
                );
                ui.heading("Mit ChatGPT verbinden");
                ui.separator();
                if let Some(prompt) = &login.prompt {
                    if let Some(code) = prompt
                        .instructions
                        .strip_prefix("Enter code:")
                        .map(str::trim)
                        .filter(|code| !code.is_empty())
                    {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(code).monospace().size(22.0))
                                    .selectable(true)
                                    .wrap(),
                            );
                            if design::icon_button(
                                ui,
                                egui_phosphor::regular::COPY_SIMPLE,
                                "Code kopieren",
                                true,
                            )
                            .clicked()
                            {
                                ui.ctx().copy_text(code.to_owned());
                            }
                        });
                    } else if !prompt.instructions.is_empty() {
                        ui.add(
                            egui::Label::new(&prompt.instructions)
                                .selectable(true)
                                .wrap(),
                        );
                    }
                    if prompt.mode == "code" && login.finish.is_none() && !login.cancelling {
                        let input = ui.add(
                            design::singleline(&mut login.code)
                                .password(true)
                                .hint_text(design::input_hint(
                                    "Bestätigungscode · mit Enter senden",
                                )),
                        );
                        if app.online
                            && !login.code.trim().is_empty()
                            && input.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            let code = std::mem::take(&mut login.code);
                            login.finish = Some(ephemeral(
                                app,
                                "POST",
                                format!("{}/complete", oauth_path(&login.access)),
                                json!({"attempt_id":prompt.id,"code":code}),
                            ));
                        }
                    }
                }
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if login.cancelling {
                        "Anmeldung wird abgebrochen …"
                    } else if login.prompt.is_some() {
                        "Warte auf ChatGPT …"
                    } else {
                        "Anmeldung wird vorbereitet …"
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled(!login.cancelling, egui::Button::new("Abbrechen"))
                            .clicked()
                    })
                })
                .inner
                .inner
            });
        if !login.cancelling && (modal.inner || modal.should_close()) {
            login.cancel(app);
        }
        self.login = Some(login);
    }
}

impl Login {
    fn cancel(&mut self, app: &mut App) {
        self.cancelling = true;
        self.code.clear();
        if let Some(prompt) = &self.prompt {
            self.finish = Some(ephemeral(
                app,
                "POST",
                format!("{}/cancel", oauth_path(&self.access)),
                json!({"attempt_id":prompt.id}),
            ));
        }
    }
}

fn oauth_path(id: &str) -> String {
    format!("/v1/model-settings/accesses/{id}/openai/oauth")
}

fn model_save_footer(
    ui: &mut egui::Ui,
    app: &mut App,
    bottom: f32,
    enabled: bool,
    request: Option<u64>,
) -> bool {
    design::settings_footer(ui, bottom, |ui| {
        let clicked = design::settings_save_button(ui, enabled).clicked();
        if let Some(id) = request {
            if app.in_flight.contains(&id) {
                ui.spinner();
            } else {
                if app
                    .responses
                    .remove(&id)
                    .is_some_and(|result| result.is_err())
                {
                    app.notice("Speichern nicht bestätigt. Bitte dieselbe Anfrage abgleichen.");
                }
                if ui
                    .add_enabled(app.online, egui::Button::new("Abgleichen"))
                    .clicked()
                {
                    app.retry(id);
                }
            }
        }
        clicked
    })
}

fn settled_save(
    app: &mut App,
    request: Option<u64>,
) -> Option<Result<Value, crate::backend::Failure>> {
    let id = request?;
    if app
        .responses
        .get(&id)
        .is_some_and(|r| r.as_ref().is_err_and(|e| e.uncertain))
    {
        return None;
    }
    app.responses.remove(&id)
}

#[derive(Default)]
pub struct ProjectPane {
    draft: Option<ProjectModelSettings>,
    save: Option<u64>,
    error: Option<String>,
}

pub fn load_project(app: &mut App, id: ProjectId) {
    let mut load = app.models.projects.remove(&id).unwrap_or_default();
    load.load(app, &format!("/v1/projects/{id}/models"));
    app.models.projects.insert(id, load);
}

pub fn show_project(ui: &mut egui::Ui, app: &mut App, id: ProjectId, bottom: f32) {
    let mut pane = app.models.project_panes.remove(&id).unwrap_or_default();
    pane.show(ui, app, id, bottom);
    app.models.project_panes.insert(id, pane);
}

impl ProjectPane {
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App, project: ProjectId, bottom: f32) {
        if !app.models.managed() {
            ui.label("Zentrale Modellzugänge sind für diese Factory nicht aktiv.");
            return;
        }
        load_project(app, project);
        if self.save.is_none() {
            let path = format!("/v1/projects/{project}/models");
            self.save = app
                .settings
                .pending
                .iter()
                .find(|p| p.server == app.server && p.path == path)
                .map(|p| p.id);
        }
        if let Some(result) = settled_save(app, self.save) {
            self.save = None;
            match result {
                Ok(value) => {
                    let cache = app.models.projects.get_mut(&project).unwrap();
                    cache.value = serde_json::from_value(value).ok();
                    self.draft = None;
                }
                Err(error) => self.error = Some(error.message),
            }
        }
        let ready = app.models.ready_catalog();
        let Some(_) = app.models.factory.value.as_ref() else {
            ui.label("Modellkatalog lädt …");
            return;
        };
        let Some(saved) = app
            .models
            .projects
            .get(&project)
            .and_then(|load| load.value.as_ref())
        else {
            let error = app
                .models
                .projects
                .get(&project)
                .and_then(|load| load.error.as_deref());
            ui.label(error.unwrap_or("Projektmodelle laden …"));
            return;
        };
        let mut draft = self.draft.clone().unwrap_or_else(|| saved.clone());
        let editable = app.online && self.save.is_none();
        egui::ScrollArea::vertical()
            .id_salt(("project-models-fields", project))
            .max_height(design::settings_body_height(ui, bottom))
            .show(ui, |ui| {
                ui.label("Default übernimmt das jeweilige Rollenmodell der Factory.");
                ui.add_enabled_ui(editable, |ui| {
                    roles(ui, "project-models", &ready, &mut draft.roles)
                });
            });
        self.draft = (draft != *saved).then(|| draft.clone());
        if model_save_footer(ui, app, bottom, editable && self.draft.is_some(), self.save) {
            self.error = None;
            self.save = Some(app.write(
                "Projektmodelle speichern",
                "PUT",
                format!("/v1/projects/{project}/models"),
                json!(SetProjectModels {
                    request_id: RequestId::new(),
                    settings: draft
                }),
            ));
        }
        if let Some(error) = &self.error {
            ui.colored_label(RED, error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_login_opens_browser_once_and_starts_waiting_automatically() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let mut pane = FactoryPane::default();
        pane.login = Some(Login {
            access: "factory-test".into(),
            begin: 123,
            prompt: None,
            code: String::new(),
            finish: None,
            cancelling: false,
        });
        app.responses.insert(
            123,
            Ok(json!({
                "attemptID":"demo","url":"https://auth.openai.com/codex/device",
                "instructions":"Enter code: DEMO-CODE","mode":"auto"
            })),
        );
        let frame = ctx.run(egui::RawInput::default(), |ctx| {
            pane.show_login(ctx, &mut app)
        });
        assert_eq!(
            frame
                .platform_output
                .commands
                .iter()
                .filter(|command| matches!(command, egui::OutputCommand::OpenUrl(_)))
                .count(),
            1
        );
        let finish = pane
            .login
            .as_ref()
            .unwrap()
            .finish
            .expect("automatic OAuth completion");
        assert!(app.in_flight.contains(&finish));
        assert!(app.settings.pending.is_empty());
        let frame = ctx.run(egui::RawInput::default(), |ctx| {
            pane.show_login(ctx, &mut app)
        });
        assert!(!frame
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))));
        assert_eq!(pane.login.as_ref().unwrap().finish, Some(finish));
        let button: egui::Rect = ctx
            .data_mut(|data| data.get_temp(egui::Id::new(("test-icon-button", "Code kopieren"))))
            .unwrap();
        let pos = button.center();
        let frame = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| pane.show_login(ctx, &mut app),
        );
        assert!(frame.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(code) if code == "DEMO-CODE")
        ));
    }

    #[test]
    fn memorizer_model_selection_is_visible_in_factory_and_project_settings() {
        let mut app = crate::tests::fixture();
        seed(&mut app, true);
        let project = app.project().unwrap().id;
        app.models.projects.entry(project).or_default().value = Some(ProjectModelSettings {
            revision: factory_core::Revision(1),
            roles: RoleModels::default(),
        });
        let ctx = app.backend.context().clone();
        for (mut pane, section) in [
            (ProjectSettings::factory(), ProjectSection::DefaultModels),
            (ProjectSettings::new(project), ProjectSection::Models),
        ] {
            pane.selected = section;
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 900.0))),
                ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| pane.show(ui, &mut app, 730.0));
            });
            assert!(output.shapes.iter().any(|s| matches!(
                &s.shape, egui::epaint::Shape::Text(text) if text.galley.job.text == "Memorizer"
            )));
        }
    }

    #[test]
    fn default_model_and_project_settings_pin_save_to_the_bottom_right() {
        let mut app = crate::tests::fixture();
        seed(&mut app, true);
        let ctx = app.backend.context().clone();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(950.0, 620.0),
            )),
            ..Default::default()
        };
        let id = app.project().unwrap().id;
        for mut settings in [ProjectSettings::factory(), ProjectSettings::new(id)] {
            settings.selected = if settings.project.is_some() {
                ProjectSection::Profile
            } else {
                ProjectSection::DefaultModels
            };
            for _ in 0..2 {
                let _ = ctx.run(input(), |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| settings.show(ui, &mut app, 410.0));
                });
            }
            let (save, (_, _, pane)): (egui::Rect, (f32, egui::Rect, egui::Rect)) =
                ctx.data_mut(|d| {
                    (
                        d.get_temp(egui::Id::new("test-settings-save")).unwrap(),
                        d.get_temp(egui::Id::new("test-settings-geometry")).unwrap(),
                    )
                });
            assert!(
                (pane.right() - save.right()).abs() < 2.0,
                "{pane:?} {save:?}"
            );
            assert!(
                pane.bottom() - save.bottom() >= 0.0 && pane.bottom() - save.bottom() <= 12.0,
                "{pane:?} {save:?}"
            );
        }
    }

    #[test]
    fn first_login_opens_confirmation_then_default_models() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let mut pane = FactoryPane::default();
        pane.login = Some(Login {
            access: "factory-test".into(),
            begin: 98,
            prompt: None,
            code: String::new(),
            finish: Some(99),
            cancelling: false,
        });
        app.responses.insert(
            99,
            Ok(json!({"connected":true,
            "automatic_default":{"access_id":"factory-test","model_id":"discovered-model"}})),
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            pane.show_login(ctx, &mut app);
        });
        let mut dialog = app.dialog.take().unwrap();
        assert!(
            matches!(&dialog, crate::dialogs::Dialog::ModelDefaultsConfigured { model } if model == "discovered-model")
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            dialog.show(ctx, &mut app);
        });
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                assert!(dialog.show(ctx, &mut app));
            },
        );
        assert!(matches!(
            app.dialog,
            Some(crate::dialogs::Dialog::FactorySettings)
        ));
        assert_eq!(
            app.factory_settings.as_ref().unwrap().selected,
            ProjectSection::DefaultModels
        );
    }

    #[test]
    fn oauth_failure_is_shown_as_toast_not_as_inline_error() {
        let mut app = crate::tests::fixture();
        let ctx = app.backend.context().clone();
        let mut pane = FactoryPane::default();
        let request = 123;
        pane.login = Some(Login {
            access: "factory-test".into(),
            begin: request,
            prompt: None,
            code: String::new(),
            finish: None,
            cancelling: false,
        });
        app.responses.insert(
            request,
            Err(crate::backend::Failure {
                message: "Die ChatGPT-Anmeldung konnte nicht gestartet werden.".into(),
                uncertain: false,
            }),
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            pane.show_login(ctx, &mut app);
        });
        assert!(pane.login.is_none());
        assert_eq!(
            app.notices.last().unwrap().0,
            "Die ChatGPT-Anmeldung konnte nicht gestartet werden."
        );
    }

    #[test]
    fn oauth_request_does_not_enter_desktop_outbox() {
        let mut app = crate::tests::fixture();
        let id = ephemeral(
            &mut app,
            "POST",
            format!("{}/complete", oauth_path("factory-test")),
            json!({"attempt_id":"test", "code":"not-a-real-code"}),
        );
        assert!(app.in_flight.contains(&id));
        assert!(app.settings.pending.is_empty());
        assert!(!serde_json::to_string(&app.settings)
            .unwrap()
            .contains("not-a-real-code"));
    }

    #[test]
    fn same_model_on_two_accounts_has_different_display_labels() {
        let mut app = crate::tests::fixture();
        seed(&mut app, true);
        let catalog = &mut app.models.factory.value.as_mut().unwrap().accesses;
        catalog.push(ModelAccess {
            id: "factory-private".into(),
            name: "OpenAI Privat".into(),
            models: vec!["demo".into()],
        });
        let model = |id: &str| ModelSelection {
            access_id: id.into(),
            model_id: "demo".into(),
        };
        assert_eq!(
            label(catalog, &model("factory-test")),
            "OpenAI Arbeit · demo"
        );
        assert_eq!(
            label(catalog, &model("factory-private")),
            "OpenAI Privat · demo"
        );
    }

    #[test]
    fn add_access_dialog_only_asks_for_a_name_and_keeps_it_on_close() {
        let mut app = crate::tests::fixture();
        seed(&mut app, false);
        app.models.pane.naming = true;
        app.models.pane.name = "OpenAI Privat".into();
        let ctx = app.backend.context().clone();
        let mut text = Vec::new();
        fn collect(shape: &egui::epaint::Shape, text: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(value) => text.push(value.galley.job.text.clone()),
                egui::epaint::Shape::Vec(children) => {
                    children.iter().for_each(|child| collect(child, text))
                }
                _ => {}
            }
        }
        let render = |app: &mut App, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| show_add_dialog(ctx, app),
            )
        };
        let _ = render(&mut app, vec![]);
        for shape in render(&mut app, vec![]).shapes {
            collect(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|s| s == "Name"));
        assert!(text.iter().any(|s| s.contains("Weiter zur Anmeldung")));
        assert!(!text.iter().any(|s| s.contains("Modell-ID")));
        let _ = render(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!app.models.pane.naming);
        assert_eq!(app.models.pane.name, "OpenAI Privat");
        assert!(app.settings.pending.is_empty());
    }
}
