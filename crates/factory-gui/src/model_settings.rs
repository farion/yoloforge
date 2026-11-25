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
                Err(_) => self.error = Some(crate::tr!("Unerwartete Antwort der Factory.").into()),
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
            pane.settle_defaults(app, &mut self.factory, &mut self.next_refresh);
            if let Some((id, result)) = pane.remove_request.as_ref().and_then(|(id, request)| {
                app.responses.remove(request).map(|result| (id.clone(), result))
            }) {
                pane.remove_request = None;
                match result {
                    Ok(value) => match serde_json::from_value::<FactoryModelSettings>(value["settings"].clone()) {
                        Ok(saved) => {
                            if let Some(draft) = pane.draft.as_mut() {
                                draft.revision = saved.revision;
                                draft.accesses.clone_from(&saved.accesses);
                                for model in [&mut draft.default_model, &mut draft.roles.orchestrator,
                                    &mut draft.roles.implementer, &mut draft.roles.supervisor,
                                    &mut draft.roles.memorizer] {
                                    if model.as_ref().is_some_and(|model| model.access_id == id) {
                                        *model = None;
                                    }
                                }
                            }
                            pane.status.remove(&id);
                            self.factory.value = Some(saved);
                            self.factory.request = None;
                            self.projects.clear();
                            app.notice_success(if value["credential_removed"] == true {
                                crate::tr!("Modellzugang gelöscht.")
                            } else {
                                crate::tr!("Modellzugang entfernt. Die Zugangsdaten konnten nicht gelöscht werden.")
                            });
                        }
                        Err(_) => app.notice_error(crate::tr!("Das Löschen wurde nicht bestätigt.")),
                    },
                    Err(error) => app.notice_error(error.message),
                }
            }
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
        name: crate::tr!("OpenAI Arbeit").into(),
        models: vec!["demo".into()],
        transports: Default::default(),
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
) -> Option<(factory_protocol::SubmitTask, Option<ModelSelection>, Option<Vec<String>>)> {
    match path {
        "/v1/tasks/submit" => serde_json::from_value(value.clone())
            .ok()
            .map(|task| (task, None, None)),
        "/v1/tasks/submit-with-model" => {
            serde_json::from_value::<SubmitTaskWithModel>(value.clone())
                .ok()
            .map(|body| (body.submission, body.implementer, body.local_mcps))
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
    format!("{} / {name}", model.model_id)
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
        design::settings_field(ui, name, |ui| {
            choice(ui, (scope, name), catalog, selection, "Default");
        });
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
    Copilot,
    Key(KeyModelProvider),
}

impl Provider {
    fn available() -> impl Iterator<Item = Self> {
        [Self::ChatGpt, Self::Copilot].into_iter().chain(KeyModelProvider::ALL.into_iter().map(Self::Key))
    }

    fn label(self) -> &'static str {
        match self {
            Self::ChatGpt => "OpenAI (ChatGPT)",
            Self::Copilot => "GitHub Copilot",
            Self::Key(provider) => provider.label(),
        }
    }

    fn new_access(self) -> (String, String) {
        match self {
            Self::ChatGpt | Self::Copilot => {
                let prefix = if self == Self::Copilot { COPILOT_ACCESS_PREFIX } else { "factory-openai-" };
                let id = format!("{prefix}{}", RequestId::new());
                let path = format!("{}/start", oauth_path(&id));
                (id, path)
            }
            Self::Key(provider) => {
                let id = provider.new_access_id();
                let path = format!("/v1/model-settings/accesses/{id}/key");
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
    api_key: String,
    endpoint: String,
    key_request: Option<u64>,
    remove_access: Option<(String, String)>,
    remove_request: Option<(String, u64)>,
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

pub(crate) fn factory_defaults_can_save(app: &App) -> bool {
    app.online && app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin)
        && match app.models.pane.save {
            None => app.models.pane.draft.is_some(),
            Some(id) => !app.in_flight.contains(&id) && app.settings.pending.iter()
                .any(|request| request.server == app.server && request.id == id),
        }
}

pub(crate) fn factory_defaults_saving(app: &App) -> bool {
    app.models.pane.save.is_some_and(|id| app.in_flight.contains(&id))
}

pub(crate) fn save_factory_defaults(app: &mut App) {
    if factory_defaults_can_save(app) {
        let mut pane = std::mem::take(&mut app.models.pane);
        if let Some(id) = pane.save {
            app.responses.remove(&id);
            app.retry(id);
        } else if let Some(draft) = pane.draft.clone() {
            pane.send(app, draft);
        }
        app.models.pane = pane;
    }
}

pub fn show_add_dialog(ctx: &egui::Context, app: &mut App) {
    let mut pane = std::mem::take(&mut app.models.pane);
    if let Some(result) = pane.key_request.and_then(|id| app.responses.remove(&id)) {
        pane.key_request = None;
        match result {
            Ok(value) if value["connected"] == true => {
                pane.api_key.clear();
                let name = std::mem::take(&mut pane.name);
                pane.naming = false;
                let mut factory = std::mem::take(&mut app.models.factory);
                factory.fetch(app, "/v1/model-settings");
                app.models.factory = factory;
                if let Ok(model) =
                    serde_json::from_value::<ModelSelection>(value["automatic_default"].clone())
                {
                    app.dialog = Some(crate::dialogs::Dialog::ModelDefaultsConfigured {
                        model: format!("{} · {}", name.trim(), model.model_id),
                    });
                } else {
                    app.notice_success(crate::tr!("Modellzugang verbunden."));
                }
            }
            Ok(_) => app.notice_error(crate::tr!("Der Modellzugang wurde nicht bestätigt.")),
            Err(error) => app.notice_error(error.message),
        }
    }
    if pane.remove_access.is_some() {
        pane.show_remove_dialog(ctx, app);
    } else if pane.naming && matches!(pane.provider, Provider::Key(_)) {
        pane.show_key_dialog(ctx, app);
    } else if pane.naming {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("name-model-access"))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING))
            .show(ctx, |ui| {
                ui.set_width(
                    design::dialog_content_size(ctx.content_rect().size())
                        .x
                        .min(380.0),
                );
                let title = if pane.provider == Provider::Copilot { crate::tr!("GitHub-Copilot-Zugang benennen") }
                    else { crate::tr!("OpenAI-Zugang benennen") };
                close = design::dialog_header(ui, title).clicked();
                ui.separator();
                ui.label("Name");
                let input = ui.add(
                    design::singleline(&mut pane.name)
                        .hint_text(design::input_hint(if pane.provider == Provider::Copilot {
                            crate::tr!("z. B. Arbeit")
                        } else { crate::tr!("z. B. OpenAI Arbeit") }))
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
                    ui.colored_label(RED, crate::tr!("Dieser Name ist bereits vergeben."));
                }
                let valid = app.online
                    && !name.is_empty()
                    && name.len() <= 120
                    && !name.chars().any(char::is_control)
                    && !duplicate;
                ui.separator();
                let bottom = ui.cursor().top();
                let (submit, cancel) = design::settings_footer(ui, bottom, |ui| {
                    let submit = design::accent_button(
                        ui, egui::Button::new(crate::tr!("Weiter zur Anmeldung")), valid,
                        crate::app::BLUE.gamma_multiply(0.7),
                    ).clicked();
                    let cancel = ui.button(crate::tr!("Abbrechen")).clicked();
                    (submit, cancel)
                });
                close |= cancel;
                if !close && valid
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
    fn show_remove_dialog(&mut self, ctx: &egui::Context, app: &mut App) {
        let Some((id, name)) = self.remove_access.as_ref() else { return };
        let response = egui::Modal::new(egui::Id::new("remove-model-access"))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING))
            .show(ctx, |ui| {
                ui.set_width(design::dialog_content_size(ctx.content_rect().size()).x.min(440.0));
                let close = design::dialog_header(ui, crate::tr!("Modellzugang löschen")).clicked();
                ui.separator();
                ui.label(crate::tr!("„{}“ und seine Zugangsdaten aus der Factory löschen?", name));
                ui.separator();
                let bottom = ui.cursor().top();
                design::settings_footer(ui, bottom, |ui| {
                    let remove = design::accent_button(ui, egui::Button::new(crate::tr!("Löschen")),
                        app.online && self.remove_request.is_none(), RED.gamma_multiply(0.7));
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new("test-model-remove-confirm"), remove.rect));
                    (remove.clicked(), close || ui.button(crate::tr!("Abbrechen")).clicked())
                })
            });
        let (confirm, cancel) = response.inner;
        if confirm {
            let request = ephemeral(app, "DELETE", format!("/v1/model-settings/accesses/{id}"), Value::Null);
            self.remove_request = Some((id.clone(), request));
        }
        if confirm || cancel || response.should_close() {
            self.remove_access = None;
        }
    }

    fn provider_picker(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let count = Provider::available().count() as f32;
        let rows_height = count * (design::LIST_ROW_HEIGHT + ui.spacing().item_spacing.y);
        let viewport = ui.ctx().content_rect();
        let anchor = ui.cursor().top();
        let room = (anchor - viewport.top())
            .max(viewport.bottom() - anchor - ui.spacing().interact_size.y);
        egui::ComboBox::from_id_salt("add-model-provider")
            .selected_text(self.provider.label())
            .width(ui.available_width().min(260.0))
            .height(rows_height.min((room - 24.0).max(design::LIST_ROW_HEIGHT)))
            .show_ui(ui, |ui| {
                for provider in Provider::available() {
                    if design::navigation_row(ui, provider.label(), provider == self.provider, None)
                        .clicked()
                    {
                        if provider != self.provider {
                            self.api_key.clear();
                            self.endpoint.clear();
                        }
                        self.provider = provider;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    fn show_key_dialog(&mut self, ctx: &egui::Context, app: &mut App) {
        let Provider::Key(provider) = self.provider else { return };
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("key-model-access"))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING))
            .show(ctx, |ui| {
                let size = design::dialog_content_size(ctx.content_rect().size());
                ui.set_width(size.x.min(440.0));
                let max_bottom = ui.cursor().top() + size.y;
                close = design::dialog_header(ui, &crate::tr!("{} hinzufügen", provider.label())).clicked();
                ui.separator();
                let pending = self.key_request.is_some();
                let max_height = design::settings_body_height(ui, max_bottom).max(24.0);
                let body = egui::ScrollArea::vertical().id_salt("provider-fields")
                    .max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add_enabled_ui(!pending, |ui| {
                    design::settings_field(ui, "Name", |ui| {
                        let input = ui.add(design::singleline(&mut self.name)
                            .hint_text(design::input_hint(crate::tr!("z. B. Arbeit")))
                            .desired_width(f32::INFINITY));
                        if std::mem::take(&mut self.focus_name) {
                            input.request_focus();
                        }
                    });
                    if provider.endpoint_env().is_some() {
                        let example = if provider == KeyModelProvider::Ollama {
                            "http://localhost:11434"
                        } else {
                            "http://litellm:4000"
                        };
                        design::settings_field(ui, crate::tr!("Server-URL"), |ui| {
                            ui.add(design::singleline(&mut self.endpoint)
                                .hint_text(design::input_hint(example))
                                .desired_width(f32::INFINITY));
                            ui.label(egui::RichText::new(crate::tr!("Die URL muss vom Factory-Server erreichbar sein.")).color(MUTED));
                        });
                    }
                    let key_label = if provider.key_optional() {
                        crate::tr!("API-Schlüssel (optional)")
                    } else {
                        crate::tr!("API-Schlüssel")
                    };
                    design::settings_field(ui, key_label, |ui| {
                        ui.add(design::singleline(&mut self.api_key)
                            .password(true)
                            .hint_text(design::input_hint(crate::tr!("API-Schlüssel einfügen")))
                            .desired_width(f32::INFINITY));
                        if provider == KeyModelProvider::Gemini {
                            ui.hyperlink_to(crate::tr!("Schlüssel in Google AI Studio erstellen"), "https://aistudio.google.com/apikey");
                        }
                    });
                    });
                    let name = self.name.trim();
                    let duplicate = app.models.factory.value.as_ref().is_some_and(|settings| {
                        settings.accesses.iter().any(|a| a.name.trim().to_lowercase() == name.to_lowercase())
                    });
                    if duplicate {
                        ui.colored_label(RED, crate::tr!("Dieser Name ist bereits vergeben."));
                    }
                    duplicate
                });
                #[cfg(test)]
                ctx.data_mut(|d| {
                    let mut visible = body.inner_rect;
                    visible.set_height(visible.height().min(body.content_size.y));
                    d.insert_temp(egui::Id::new("test-provider-form-body"), visible);
                });
                let name = self.name.trim();
                let key = self.api_key.trim();
                let endpoint = self.endpoint.trim();
                let valid = app.online && !pending && !body.inner && !name.is_empty()
                    && name.len() <= 120 && !name.chars().any(char::is_control)
                    && (provider.key_optional() || !key.is_empty())
                    && (endpoint.is_empty() || endpoint.starts_with("https://") || endpoint.starts_with("http://"))
                    && key.len() <= 4096 && key.bytes().all(|b| b.is_ascii_graphic());
                ui.separator();
                let bottom = ui.cursor().top();
                let (save, cancel) = design::settings_footer(ui, bottom, |ui| {
                    let save = design::settings_save_button(ui, valid);
                    let cancel = ui.add_enabled(!pending, egui::Button::new(crate::tr!("Abbrechen")));
                    #[cfg(test)]
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-provider-form-cancel"), cancel.rect));
                    if pending {
                        ui.spinner().on_hover_text(crate::tr!("API-Schlüssel wird geprüft …"));
                    }
                    (save.clicked(), cancel.clicked())
                });
                close |= cancel;
                if save && !close {
                    let (_, path) = self.provider.new_access();
                    let url = (provider.endpoint_env().is_some() && !endpoint.is_empty()).then_some(endpoint);
                    self.key_request = Some(ephemeral(app, "POST", path, json!({"name":name,"key":key,"url":url})));
                }
            });
        self.naming = !(close || response.should_close());
    }

    fn show(&mut self, ui: &mut egui::Ui, app: &mut App, defaults_bottom: Option<f32>) {
        if !app.models.managed() {
            if let Some(error) = &app.models.setup.error {
                ui.colored_label(RED, error);
            } else if app.models.setup.value.is_some() {
                ui.label(crate::tr!("Diese Factory wurde ohne zentralen Modell-Gateway gestartet."));
            } else {
                ui.label(crate::tr!("Verbindung und OpenCode-Einrichtung werden geprüft …"));
            }
            return;
        }
        let Some(saved) = app.models.factory.value.clone() else {
            ui.label(crate::tr!("Modell-Anbieter werden geladen …"));
            if let Some(error) = &app.models.factory.error {
                ui.colored_label(RED, error);
            }
            return;
        };
        let admin = app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin);
        let editable = admin && app.online && self.save.is_none();
        if let Some(bottom) = defaults_bottom {
            self.show_defaults(ui, saved, editable, bottom);
            return;
        }
        self.show_providers(ui, app, &saved, admin, editable);
    }

    fn settle_defaults(&mut self, app: &mut App, factory: &mut Load<FactoryModelSettings>, next_refresh: &mut Option<Instant>) {
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
                        factory.value = Some(saved);
                        factory.request = None;
                        *next_refresh = None;
                    }
                    Err(_) => app.project_notice(crate::app::ToastKind::Warning, None, crate::tr!("Speichern nicht eindeutig bestätigt.")),
                },
                Err(_) => app.notice_error(crate::tr!("Die Default-Modelle konnten nicht gespeichert werden.")),
            }
        }
    }

    fn show_providers(&mut self, ui: &mut egui::Ui, app: &mut App, saved: &FactoryModelSettings, admin: bool, editable: bool) {
        if !admin {
            ui.colored_label(
                MUTED,
                crate::tr!("Nur Factory-Admins können Zugänge und Standardmodelle ändern."),
            );
        }
        ui.add_space(8.0);
        design::section_heading(ui, crate::tr!("Anbieter hinzufügen"));
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(editable && self.login.is_none() && self.key_request.is_none(), |ui| {
                self.provider_picker(ui);
                if ui.button(crate::tr!("Hinzufügen")).clicked() {
                    self.naming = true;
                    self.focus_name = true;
                }
            });
        });
        ui.add_space(12.0);
        design::section_heading(ui, crate::tr!("Vorhandene Zugänge"));
        if saved.accesses.is_empty() {
            ui.colored_label(MUTED, crate::tr!("Noch keine Zugänge verbunden."));
        }
        for access in &saved.accesses {
            let status = self.status.entry(access.id.clone()).or_default();
            status.load(
                app,
                &format!("/v1/model-settings/accesses/{}/status", access.id),
            );
            ui.push_id(&access.id, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), design::LIST_ROW_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let remove = design::icon_button(ui, egui_phosphor::regular::TRASH,
                        crate::tr!("Modellzugang löschen"), editable && self.login.is_none()
                            && self.key_request.is_none() && self.remove_request.is_none());
                    if remove.clicked() {
                        self.remove_access = Some((access.id.clone(), access.name.clone()));
                    }
                    match status.value.as_ref().map(|status| status.state.as_str()) {
                        Some("ready") => {
                            ui.colored_label(crate::app::GREEN, crate::tr!("Verbunden"));
                        }
                        Some(_) if KeyModelProvider::from_access_id(&access.id).is_some() => {
                            ui.colored_label(RED, crate::tr!("Zugang nicht verfügbar"));
                        }
                        Some(state) => {
                            let action = if state == "not_connected" && is_copilot_access(&access.id) {
                                crate::tr!("Mit GitHub verbinden")
                            } else if state == "not_connected" {
                                crate::tr!("Mit ChatGPT verbinden")
                            } else {
                                crate::tr!("Erneut anmelden")
                            };
                            if ui.add_enabled(editable && self.login.is_none() && self.remove_request.is_none(),
                                egui::Button::new(action)).clicked()
                            {
                                let begin = ephemeral(app, "POST", oauth_path(&access.id), Value::Null);
                                self.login = Some(Login {
                                    access: access.id.clone(), begin, prompt: None, code: String::new(),
                                    finish: None, cancelling: false,
                                });
                            }
                            if state != "not_connected" {
                                ui.colored_label(RED, crate::tr!("Zugang nicht verfügbar"));
                            }
                        }
                        None => {
                            ui.colored_label(
                                MUTED,
                                status.error.as_deref().unwrap_or(crate::tr!("Wird geprüft …")),
                            );
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&access.name).strong()).truncate())
                            .on_hover_text(&access.name);
                    });
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-provider-row", &access.id)), ui.min_rect()));
                });
            });
        }
    }

    fn show_defaults(
        &mut self,
        ui: &mut egui::Ui,
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
            .max_height((bottom - ui.cursor().top()).max(0.0))
            .show(ui, |ui| {
                design::section_heading(ui, crate::tr!("Standardmodell"));
                if saved
                    .accesses
                    .iter()
                    .all(|a| matches!(state(a), Some("not_connected" | "unavailable")))
                {
                    ui.label(egui::RichText::new(crate::tr!("Verbinde zuerst einen Anbieter.")).color(MUTED));
                    ui.add_space(6.0);
                }
                ui.add_enabled_ui(editable, |ui| {
                    choice(
                        ui,
                        "factory-default",
                        &ready,
                        &mut draft.default_model,
                        crate::tr!("Nicht eingerichtet"),
                    );
                });
                ui.add_space(16.0);
                design::section_heading(ui, crate::tr!("Agenten Overrides"));
                ui.label(egui::RichText::new(crate::tr!("Überschreibe die Modelle pro Agent")).color(MUTED));
                ui.add_space(6.0);
                ui.add_enabled_ui(editable, |ui| {
                    roles(ui, "factory-models", &ready, &mut draft.roles);
                });
            });
        self.draft = (draft != saved).then_some(draft);
    }

    fn send(&mut self, app: &mut App, settings: FactoryModelSettings) {
        self.save = Some(app.write(
            crate::tr!("Factory-Modelle speichern"),
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
        let copilot = is_copilot_access(&login.access);
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
                            if copilot { crate::tr!("Die Factory hat keine gültige GitHub-Anmeldung zurückgegeben.") }
                            else { crate::tr!("Die Factory hat keine gültige ChatGPT-Anmeldung zurückgegeben.") },
                        );
                        return;
                    }
                },
                Err(error) => {
                    if !login.cancelling {
                        app.notice_error(error.message);
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
                        app.notice_success(if copilot { crate::tr!("GitHub Copilot verbunden.") }
                            else { crate::tr!("ChatGPT-Zugang verbunden.") });
                    }
                }
                Ok(_) => app.notice_error(if copilot { crate::tr!("Die GitHub-Anmeldung wurde nicht bestätigt.") }
                    else { crate::tr!("Die ChatGPT-Anmeldung wurde nicht bestätigt.") }),
                Err(error) => {
                    self.status.remove(&login.access);
                    app.notice_error(error.message);
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
                ui.heading(if copilot { crate::tr!("Mit GitHub verbinden") } else { crate::tr!("Mit ChatGPT verbinden") });
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
                                crate::tr!("Code kopieren"),
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
                                    crate::tr!("Bestätigungscode · mit Enter senden"),
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
                        crate::tr!("Anmeldung wird abgebrochen …")
                    } else if login.prompt.is_some() && copilot {
                        crate::tr!("Warte auf GitHub …")
                    } else if login.prompt.is_some() {
                        crate::tr!("Warte auf ChatGPT …")
                    } else {
                        crate::tr!("Anmeldung wird vorbereitet …")
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled(!login.cancelling, egui::Button::new(crate::tr!("Abbrechen")))
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
    let provider = if is_copilot_access(id) { COPILOT_INTEGRATION } else { "openai" };
    format!("/v1/model-settings/accesses/{id}/{provider}/oauth")
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
        app.responses.remove(&id);
        app.project_notice(crate::app::ToastKind::Warning, None, crate::tr!("Speichern nicht bestätigt. Bitte dieselbe Anfrage abgleichen."));
        return None;
    }
    app.responses.remove(&id)
}

#[derive(Default)]
pub struct ProjectPane {
    draft: Option<ProjectModelSettings>,
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
            ui.label(crate::tr!("Zentrale Modellzugänge sind für diese Factory nicht aktiv."));
            return;
        }
        let had_load_error = app.models.projects.get(&project).is_some_and(|load| load.error.is_some());
        load_project(app, project);
        let load_error = app.models.projects.get(&project).is_some_and(|load| load.error.is_some());
        if load_error && !had_load_error {
            app.project_notice(crate::app::ToastKind::Error, Some(project), crate::tr!("Projektmodelle konnten nicht geladen werden."));
        }
        let ready = app.models.ready_catalog();
        let Some(factory) = app.models.factory.value.as_ref() else {
            ui.label(crate::tr!("Modellkatalog lädt …"));
            return;
        };
        let Some(saved) = app
            .models
            .projects
            .get(&project)
            .and_then(|load| load.value.as_ref())
        else {
            if load_error {
                if ui.add_enabled(app.online, egui::Button::new(crate::tr!("Erneut laden"))).clicked() {
                    app.models.projects.get_mut(&project).unwrap().error = None;
                }
            } else if app.online {
                ui.label(crate::tr!("Projektmodelle laden …"));
            }
            return;
        };
        let mut draft = self.draft.clone().unwrap_or_else(|| saved.clone());
        let editable = app.online;
        egui::ScrollArea::vertical()
            .id_salt(("project-models-fields", project))
            .max_height((bottom - ui.cursor().top()).max(0.0))
            .show(ui, |ui| {
                design::section_heading(ui, crate::tr!("Standardmodell"));
                let default = factory.default_model.as_ref()
                    .map_or_else(|| crate::tr!("Nicht eingerichtet").into(), |model| label(&factory.accesses, model));
                ui.label(crate::tr!("Aus der Factory: {default}", default = default));
                ui.add_space(16.0);
                design::section_heading(ui, crate::tr!("Agenten-Overrides"));
                ui.label(crate::tr!("Ohne eigene Auswahl gilt das jeweilige Rollenmodell der Factory."));
                ui.add_space(6.0);
                ui.add_enabled_ui(editable, |ui| {
                    roles(ui, "project-models", &ready, &mut draft.roles)
                });
            });
        self.draft = (draft != *saved).then(|| draft.clone());
    }
}

pub(crate) fn project_draft(app: &App, project: ProjectId) -> Option<Value> {
    let settings = app.models.project_panes.get(&project)?.draft.as_ref()?;
    Some(json!(SetProjectModels { request_id: RequestId::new(), settings: settings.clone() }))
}

pub(crate) fn discard_project_draft(app: &mut App, project: ProjectId) {
    if let Some(pane) = app.models.project_panes.get_mut(&project) {
        pane.draft = None;
    }
}

pub(crate) fn confirm_project_save(app: &mut App, project: ProjectId, value: Value) -> bool {
    let Ok(settings) = serde_json::from_value(value) else { return false; };
    app.models.projects.entry(project).or_default().value = Some(settings);
    discard_project_draft(app, project);
    true
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
            .data_mut(|data| {
                data.get_temp(egui::Id::new(("test-icon-button", crate::tr!("Code kopieren"))))
            })
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
        for (mut settings, section) in [
            (ProjectSettings::factory(), ProjectSection::Models),
            (ProjectSettings::factory(), ProjectSection::DefaultModels),
            (ProjectSettings::new(id), ProjectSection::General),
        ] {
            settings.selected = section;
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
            let bottom = if settings.project.is_some() {
                ctx.data_mut(|d| d.get_temp::<f32>(egui::Id::new("test-project-footer-bottom")).unwrap())
            } else {
                let cancel = ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new("test-factory-settings-cancel")).unwrap());
                assert!(cancel.right() < save.left() && cancel.center().y == save.center().y);
                assert!(pane.bottom() < save.top(), "{pane:?} {save:?}");
                ctx.data_mut(|d| d.get_temp::<f32>(egui::Id::new("test-factory-settings-footer-bottom")).unwrap())
            };
            assert!((-1.0..=12.0).contains(&(bottom - save.bottom())), "{bottom} {save:?}");
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
    fn api_key_provider_draft_is_masked_and_never_persisted() {
        let mut app = crate::tests::fixture();
        crate::i18n::set_language(crate::i18n::Language::German);
        seed(&mut app, false);
        let pane = &mut app.models.pane;
        pane.provider = Provider::Key(KeyModelProvider::Gemini);
        pane.naming = true;
        pane.name = "Gemini Test".into();
        pane.api_key = "dummy-secret-for-render-test".into();
        let ctx = app.backend.context().clone();
        let render = |app: &mut App| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| show_add_dialog(ctx, app),
            )
        };
        let _ = render(&mut app);
        fn text_of(shape: &egui::epaint::Shape, text: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(value) => text.push(value.galley.job.text.clone()),
                egui::epaint::Shape::Vec(shapes) => {
                    shapes.iter().for_each(|shape| text_of(shape, text));
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        for shape in render(&mut app).shapes {
            text_of(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|text| text == crate::tr!("API-Schlüssel")), "{text:?}");
        assert!(text.iter().any(|text| text == crate::tr!("Speichern")), "{text:?}");
        assert!(!text.iter().any(|text| text.contains("dummy-secret")));
        assert!(!serde_json::to_string(&app.settings).unwrap().contains("dummy-secret"));
        assert!(!text.iter().any(|text| text == crate::tr!("Server-URL")));
        let rect = |id| ctx.data_mut(|d| d.get_temp::<egui::Rect>(egui::Id::new(id))).unwrap();
        let body = rect("test-provider-form-body");
        let save = rect("test-settings-save");
        let cancel = rect("test-provider-form-cancel");
        assert!(body.bottom() < cancel.top() && cancel.right() <= save.left(),
            "body {body:?}, cancel {cancel:?}, save {save:?}");
        for provider in [KeyModelProvider::LiteLlm, KeyModelProvider::Ollama] {
            app.models.pane.provider = Provider::Key(provider);
            let mut text = Vec::new();
            for shape in render(&mut app).shapes {
                text_of(&shape.shape, &mut text);
            }
            assert!(text.iter().any(|text| text == crate::tr!("Server-URL")), "{provider:?}");
            assert_eq!(
                text.iter().any(|text| text == crate::tr!("API-Schlüssel (optional)")),
                provider == KeyModelProvider::Ollama,
            );
        }
    }

    #[test]
    fn copilot_is_available_as_a_direct_oauth_provider() {
        assert!(Provider::available().any(|provider| provider == Provider::Copilot));
        let (access, path) = Provider::Copilot.new_access();
        assert!(is_copilot_access(&access));
        assert_eq!(path, format!("/v1/model-settings/accesses/{access}/github-copilot/oauth/start"));
    }

    #[test]
    fn provider_menu_shows_all_choices_at_full_width_without_scrolling() {
        let ctx = egui::Context::default();
        let mut pane = FactoryPane::default();
        let mut button = egui::Rect::NOTHING;
        let mut frame = |events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events, ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                button = pane.provider_picker(ui).rect;
            });
        });
        let _ = frame(vec![]);
        drop(frame);
        assert!(button.width() >= 250.0);
        let pos = button.center();
        let mut frame = |events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events, ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| { pane.provider_picker(ui); });
        });
        let click = |pressed| egui::Event::PointerButton {
            pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
        };
        let _ = frame(vec![egui::Event::PointerMoved(pos), click(true), click(false)]);
        let output = frame(vec![]);
        for provider in Provider::available() {
            assert!(output.shapes.iter().any(|s| match &s.shape {
                egui::epaint::Shape::Text(text) => text.galley.job.text == provider.label()
                    && text.galley.elided == false
                    && s.clip_rect.contains_rect(text.galley.rect.translate(text.pos.to_vec2())),
                _ => false,
            }), "{} wasn't fully visible", provider.label());
        }
    }

    #[test]
    fn provider_trash_requires_confirmation() {
        let mut app = crate::tests::fixture();
        seed(&mut app, true);
        app.access = Some(factory_protocol::AccessView {
            role: DeviceRole::Admin, device_id: None, runner_id: None,
        });
        let ctx = app.backend.context().clone();
        let frame = |app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0))),
            events, ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show_factory(ui, app, None));
            show_add_dialog(ctx, app);
        });
        let click = |pos| {
            let button = |pressed| egui::Event::PointerButton {
                pos, pressed, button: egui::PointerButton::Primary, modifiers: egui::Modifiers::NONE,
            };
            vec![egui::Event::PointerMoved(pos), button(true), button(false)]
        };
        let rect = |id| ctx.data_mut(|d| d.get_temp::<egui::Rect>(id)).unwrap();
        let _ = frame(&mut app, vec![]);
        let trash = rect(egui::Id::new(("test-icon-button", crate::tr!("Modellzugang löschen"))));
        assert!(trash.top() < 220.0, "Access rows must sit at the top of the list: {trash:?}");
        let _ = frame(&mut app, click(trash.center()));
        assert!(app.models.pane.remove_access.is_some());
        assert!(app.models.pane.remove_request.is_none());
        let _ = frame(&mut app, vec![]);
        let confirm = rect(egui::Id::new("test-model-remove-confirm"));
        let _ = frame(&mut app, click(confirm.center()));
        assert_eq!(app.models.pane.remove_request.as_ref().unwrap().0, "factory-test");
        assert!(app.models.pane.remove_access.is_none());
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
            transports: Default::default(),
        });
        let model = |id: &str| ModelSelection {
            access_id: id.into(),
            model_id: "demo".into(),
        };
        assert_eq!(
            label(catalog, &model("factory-test")),
            format!("demo / {}", crate::tr!("OpenAI Arbeit"))
        );
        assert_eq!(
            label(catalog, &model("factory-private")),
            "demo / OpenAI Privat"
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
        assert!(text
            .iter()
            .any(|s| s.contains(crate::tr!("Weiter zur Anmeldung"))));
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
