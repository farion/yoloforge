//! Project and Factory settings, with drafts retained across navigation.
use crate::{
    app::App,
    design,
    dialogs::{read_content, Dialog},
};
use factory_core::*;
use factory_protocol::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectSection {
    General,
    Runner,
    Mirrors,
    Models,
    DefaultModels,
    Skills,
    Mcps,
    Limits,
    Supervisor,
    Devices,
    Language,
}
impl ProjectSection {
    pub const ALL: [Self; 8] = [
        Self::General,
        Self::Runner,
        Self::Mirrors,
        Self::Models,
        Self::Skills,
        Self::Mcps,
        Self::Limits,
        Self::Supervisor,
    ];
    pub const FACTORY: [Self; 6] = [
        Self::Models,
        Self::DefaultModels,
        Self::Skills,
        Self::Mcps,
        Self::Devices,
        Self::Language,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::General => crate::tr!("Allgemein"),
            Self::Runner => crate::tr!("Runner-Zuordnung"),
            Self::Mirrors => crate::tr!("Lokale Spiegelkopie"),
            Self::Models => crate::tr!("Modell-Anbieter"),
            Self::DefaultModels => crate::tr!("Default-Modelle"),
            Self::Skills => "Skills",
            Self::Mcps => "MCPs",
            Self::Limits => "Limits",
            Self::Supervisor => "Supervisor",
            Self::Devices => crate::tr!("Geräte und Zugänge"),
            Self::Language => if crate::i18n::is_german() { "Sprache" } else { crate::tr!("Language") },
        }
    }
}

pub struct ProjectSettings {
    pub project: Option<ProjectId>,
    pub selected: ProjectSection,
    pub panes: BTreeMap<ProjectSection, Vec<Dialog>>,
    batch: Option<SaveBatch>,
    confirm_close: bool,
    language_draft: Option<crate::i18n::Language>,
    android_push: AndroidPush,
}

const ANDROID_PUSH_PATH: &str = "/v1/android-push/settings";

#[derive(Default)]
struct AndroidPush {
    attempted: bool,
    request: Option<(u64, bool)>,
    status: Option<Value>,
    enabled: bool,
    label: String,
    app_json: String,
    service_json: String,
    picker: Option<(bool, std::sync::mpsc::Receiver<Result<Option<String>, ()>>)>,
}

impl AndroidPush {
    fn pick(&mut self, ctx: &egui::Context, private: bool) {
        let ctx = ctx.clone();
        let (send, receive) = std::sync::mpsc::channel();
        self.picker = Some((private, receive));
        std::thread::spawn(move || {
            use std::io::Read;
            let result = (|| {
                let Some(path) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() else {
                    return Ok(None);
                };
                let mut content = String::new();
                std::fs::File::open(path).map_err(|_| ())?.take(65_537).read_to_string(&mut content).map_err(|_| ())?;
                if content.len() > 65_536 { return Err(()); }
                Ok(Some(content))
            })();
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }

    fn settle_picker(&mut self, app: &mut App) {
        use std::sync::mpsc::TryRecvError;
        let Some((private, receiver)) = &self.picker else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(()),
        };
        let private = *private;
        self.picker = None;
        match result {
            Ok(Some(text)) if private => self.service_json = text,
            Ok(Some(text)) => self.app_json = text,
            Ok(None) => {},
            Err(()) => app.notice_error(crate::tr!("Firebase-Datei konnte nicht gelesen werden.")),
        }
    }

    fn send(&mut self, app: &mut App, save: bool) {
        let id = app.next_request;
        app.next_request += 1;
        app.in_flight.insert(id);
        let body = if save {
            json!({
                "enabled":self.enabled, "label":self.label,
                "app_json":(!self.app_json.trim().is_empty()).then_some(self.app_json.trim()),
                "service_json":(!self.service_json.trim().is_empty()).then_some(self.service_json.trim()),
            })
        } else { Value::Null };
        app.backend.request(id, if save { "PUT" } else { "GET" }.into(), ANDROID_PUSH_PATH.into(), body);
        self.request = Some((id, save));
    }

    fn settle(&mut self, app: &mut App) {
        let Some((id, save)) = self.request else { return };
        let Some(response) = app.responses.remove(&id) else { return };
        self.request = None;
        match response {
            Ok(status) => {
                self.enabled = status["enabled"] == true;
                self.label = status["label"].as_str().unwrap_or_default().into();
                self.status = Some(status);
                if save {
                    self.app_json.clear();
                    self.service_json.clear();
                    app.notice_success(crate::tr!("Android-Push gespeichert."));
                }
            }
            Err(error) => {
                app.action_errors.push((crate::overview::now_ms(), error.message));
                app.notice_error(if save { crate::tr!("Android-Push konnte nicht gespeichert werden. Prüfe die Firebase-Dateien.") }
                    else { crate::tr!("Android-Push konnte nicht geladen werden.") });
            }
        }
    }

    fn can_save(&self, app: &App) -> bool {
        app.online && app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin)
            && self.request.is_none() && !self.label.trim().is_empty()
            && self.status.as_ref().is_some_and(|s| {
                let app_changed = !self.app_json.trim().is_empty();
                let key_changed = !self.service_json.trim().is_empty();
                if s["configured"] == true {
                    self.enabled != (s["enabled"] == true) || s["label"] != self.label.trim() || app_changed || key_changed
                } else { app_changed && key_changed }
            })
    }

    fn show(&mut self, ui: &mut egui::Ui, app: &mut App) {
        if !app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin) { return; }
        ui.add_space(12.0);
        ui.separator();
        design::section_heading(ui, crate::tr!("Android-Push"));
        let Some(status) = &self.status else {
            if self.request.is_some() { ui.spinner(); }
            else if ui.add_enabled(app.online, egui::Button::new(crate::tr!("Konfiguration laden"))).clicked() {
                self.send(app, false);
            }
            return;
        };
        let configured = status["configured"] == true;
        if let Some(project) = status["project_id"].as_str() {
            ui.label(format!("Firebase: {project}"));
        }
        ui.add_space(design::SETTINGS_FIELD_GAP);
        ui.add_enabled_ui(app.online, |ui| {
            ui.checkbox(&mut self.enabled, crate::tr!("Android-Push aktivieren"));
            ui.add_space(design::SETTINGS_FIELD_GAP);
            design::settings_field(ui, crate::tr!("Factory-Name in der Benachrichtigung"), |ui| {
                ui.add(design::singleline(&mut self.label).desired_width(f32::INFINITY)
                    .hint_text(design::input_hint(crate::tr!("z. B. Büro-Factory"))));
            });
            let public_file = design::settings_field(ui, "google-services.json", |ui| {
                ui.add(design::multiline(&mut self.app_json).desired_width(f32::INFINITY).desired_rows(3)
                    .hint_text(design::input_hint(if configured { crate::tr!("Leer lassen, um die Datei zu behalten") }
                        else { crate::tr!("JSON-Inhalt einfügen") })));
                ui.add_enabled(self.picker.is_none(), egui::Button::new(crate::tr!("Firebase-App-Datei auswählen …"))).clicked()
            });
            let private_file = design::settings_field(ui, crate::tr!("Privater Firebase-Dienstkontoschlüssel (JSON)"), |ui| {
                let mut output = design::multiline(&mut self.service_json).id_salt("android-push-private-key")
                    .password(true).desired_width(f32::INFINITY).desired_rows(2)
                    .hint_text(design::input_hint(if configured { crate::tr!("Leer lassen, um den Schlüssel zu behalten") }
                        else { crate::tr!("JSON-Inhalt einfügen") })).show(ui);
                output.state.clear_undoer();
                output.state.store(ui.ctx(), output.response.id);
                #[cfg(test)]
                ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("test-android-push-key"), (output.response.rect, ui.clip_rect())));
                ui.add_enabled(self.picker.is_none(), egui::Button::new(crate::tr!("Dienstkonto-Datei auswählen …"))).clicked()
            });
            if public_file || private_file { self.pick(ui.ctx(), private_file); }
        });
    }
}

#[cfg(test)]
mod android_push_tests {
    use super::*;

    #[test]
    fn android_push_key_remains_masked_and_settings_save_is_reachable() {
        for size in [egui::vec2(780.0, 610.0), egui::vec2(1560.0, 1000.0)] {
            let mut app = crate::tests::fixture();
            app.online = true;
            app.access = Some(AccessView { role: DeviceRole::Admin, device_id: None, runner_id: None });
            let read = 900;
            app.responses.insert(read, Ok(json!([])));
            let mut settings = ProjectSettings::factory();
            settings.selected = ProjectSection::Devices;
            settings.panes.insert(ProjectSection::Devices, vec![Dialog::Read {
                request: read, title: "Devices".into(), path: "/v1/devices".into(),
            }]);
            settings.android_push.status = Some(json!({"enabled":true,"configured":true,"label":"Factory"}));
            settings.android_push.enabled = true;
            settings.android_push.label = "Factory".into();
            settings.android_push.service_json = "sensitive-fixture-do-not-display".into();
            assert!(settings.android_push.can_save(&app));
            app.factory_settings = Some(settings);
            let ctx = app.backend.context().clone();
            let mut dialog = Dialog::FactorySettings;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            let mut output = None;
            for _ in 0..2 {
                output = Some(ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                    assert!(!dialog.show(ctx, &mut app));
                }));
            }
            let (editor, clip): (egui::Rect, egui::Rect) =
                ctx.data(|data| data.get_temp(egui::Id::new("test-android-push-key"))).unwrap();
            let save: egui::Rect = ctx.data(|data| data.get_temp(egui::Id::new("test-settings-save"))).unwrap();
            assert!(screen.contains_rect(save), "{size:?}: {save:?}");
            assert!(clip.bottom() <= save.top(), "the form, not the footer, scrolls");
            if size.x > 1000.0 {
                assert!(clip.contains_rect(editor), "the key field is visible in the larger window");
            }
            let mut visible = vec![];
            for shape in output.unwrap().shapes { crate::tests::shape_text(&shape.shape, &mut visible); }
            assert!(!visible.iter().any(|text| text.contains("sensitive-fixture")));
            assert!(app.settings.pending.is_empty());
        }
    }
}

struct Write {
    form: Option<RequestId>,
    label: String,
    method: String,
    path: String,
    body: Value,
    retry: Option<u64>,
}

struct SaveBatch {
    remaining: VecDeque<Write>,
    current: (Write, u64),
    retry_needed: bool,
    saved: usize,
}
impl ProjectSettings {
    pub fn new(project: ProjectId) -> Self {
        Self {
            project: Some(project),
            selected: ProjectSection::General,
            panes: BTreeMap::new(),
            batch: None,
            confirm_close: false,
            language_draft: None,
            android_push: Default::default(),
        }
    }
    pub fn factory() -> Self {
        Self {
            project: None,
            selected: ProjectSection::Models,
            panes: BTreeMap::new(),
            batch: None,
            confirm_close: false,
            language_draft: None,
            android_push: Default::default(),
        }
    }
    pub fn persist(&self, app: &mut App) {
        for pane in self.panes.values().flatten() {
            if matches!(pane, Dialog::Form(form) if form.settings_baseline.is_some() && !form.settings_dirty()) {
                continue;
            }
            pane.persist_drafts(app);
        }
    }
    pub fn title(&self, app: &App) -> String {
        let Some(id) = self.project else { return crate::tr!("Factory-Einstellungen").into(); };
        app.snapshot.as_ref()
            .and_then(|s| s.projects.iter().find(|p| p.id == id))
            .map_or_else(|| crate::tr!("Projekteinstellungen").into(), |p| crate::tr!("Projekteinstellungen - {}", p.name))
    }
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32) -> bool {
        let Some(project) = self.project else { return self.show_factory_dialog(ui, app, height); };
        if self.advance(app, project) {
            return true;
        }
        let footer_gap = design::SETTINGS_FOOTER_HEIGHT + 24.0;
        let bottom = ui.cursor().top() + height;
        let view_height = (height - footer_gap).max(24.0);
        let view_close = ui.add_enabled_ui(self.batch.is_none() && !self.confirm_close, |ui| {
            ui.set_max_height(view_height);
            self.show_mode(ui, app, view_height, false)
        }).inner;
        // Paint the divider without consuming another layout row.
        ui.add_space(4.0);
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(),
            egui::Stroke::new(1.0_f32, design::SEPARATOR_COLOR));
        let dirty = self.has_changes(app, project);
        let pending = self.has_pending(app, project);
        let running = self.is_saving();
        let (save, cancel) = design::settings_footer(ui, bottom, |ui| {
            let save = design::settings_save_button(ui, app.online && !running && (dirty || pending));
            let cancel = ui.add_enabled(!running, egui::Button::new(crate::tr!("Abbrechen")));
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("test-project-settings-cancel"), cancel.rect);
                d.insert_temp(egui::Id::new("test-project-footer-bottom"), bottom);
            });
            (save.clicked(), cancel.clicked())
        });
        if save {
            self.start_save(app, project);
        }
        (cancel || view_close) && self.request_close(app)
    }

    fn show_factory_dialog(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32) -> bool {
        self.android_push.settle(app);
        self.android_push.settle_picker(app);
        let bottom = ui.cursor().top() + height;
        let view_height = (height - design::SETTINGS_FOOTER_HEIGHT - 24.0).max(24.0);
        let saving = app.language_save.is_some() || crate::model_settings::factory_defaults_saving(app)
            || self.android_push.request.is_some_and(|(_, save)| save);
        let view_close = ui.add_enabled_ui(!saving, |ui| {
            ui.set_max_height(view_height);
            self.show_mode(ui, app, view_height, false)
        }).inner;
        ui.add_space(4.0);
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(),
            egui::Stroke::new(1.0_f32, design::SEPARATOR_COLOR));
        let language = self.language_draft.filter(|language| *language != app.settings.language);
        let (save, cancel) = design::settings_footer(ui, bottom, |ui| {
            let save = design::settings_save_button(ui,
                !saving && (language.is_some() || crate::model_settings::factory_defaults_can_save(app)
                    || self.android_push.can_save(app)));
            let cancel = ui.add_enabled(!saving, egui::Button::new(crate::tr!("Abbrechen")));
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("test-factory-settings-cancel"), cancel.rect);
                data.insert_temp(egui::Id::new("test-factory-settings-footer-bottom"), bottom);
            });
            (save.clicked(), cancel.clicked())
        });
        if save {
            if self.android_push.can_save(app) { self.android_push.send(app, true); }
            crate::model_settings::save_factory_defaults(app);
            if let Some(language) = language {
                let mut settings = app.settings.clone();
                settings.language = language;
                app.language_save = Some((language, app.backend.persist_confirmed(settings)));
            }
        }
        cancel || view_close
    }

    pub(crate) fn is_saving(&self) -> bool {
        self.batch.as_ref().is_some_and(|batch| !batch.retry_needed)
    }

    pub(crate) fn owns_request(&self, id: u64) -> bool {
        self.batch.as_ref().is_some_and(|batch| batch.current.1 == id)
    }

    pub(crate) fn confirmation_open(&self) -> bool {
        self.confirm_close
    }

    fn forms(&self) -> impl Iterator<Item = &crate::dialogs::FormDialog> {
        self.panes.values().flatten().filter_map(|pane| match pane {
            Dialog::Form(form) if form.settings_baseline.is_some() => Some(form),
            _ => None,
        })
    }

    fn has_changes(&self, app: &App, project: ProjectId) -> bool {
        self.forms().any(|form| form.settings_dirty())
            || crate::model_settings::project_draft(app, project).is_some()
            || crate::skill_settings::project_draft(app, project).is_some()
            || crate::mcp_settings::project_draft(app, project).is_some()
    }

    fn has_pending(&self, app: &App, project: ProjectId) -> bool {
        app.settings.pending.iter().any(|r| r.server == app.server && settings_write(project, &r.method, &r.path))
    }

    pub(crate) fn request_close(&mut self, app: &App) -> bool {
        let Some(project) = self.project else { return true; };
        if self.is_saving() {
            return false;
        }
        if self.has_changes(app, project) || self.has_pending(app, project) || self.batch.is_some() {
            self.confirm_close = true;
            false
        } else {
            true
        }
    }

    pub(crate) fn overlay(&mut self, ctx: &egui::Context, app: &mut App) -> bool {
        if self.is_saving() {
            egui::Modal::new(egui::Id::new(("project-settings-saving", self.project))).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(24.0));
                    ui.label(crate::tr!("Projekteinstellungen werden gespeichert …"));
                });
            });
        } else if self.confirm_close {
            let Some(project) = self.project else { return false; };
            let response = egui::Modal::new(egui::Id::new(("discard-project-settings", project))).show(ctx, |ui| {
                ui.set_width(420.0_f32.min(design::dialog_content_size(ctx.content_rect().size()).x));
                let stay = design::dialog_header(ui, crate::tr!("Änderungen verwerfen?")).clicked();
                ui.separator();
                ui.label(crate::tr!("Ungespeicherte Änderungen an den Projekteinstellungen gehen verloren."));
                if self.has_pending(app, project) {
                    ui.label(crate::tr!("Bereits gesendete Anfragen werden dadurch nicht zurückgenommen."));
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let discard = ui.button(crate::tr!("Verwerfen"));
                    let stay_button = ui.button(crate::tr!("Weiter bearbeiten"));
                    #[cfg(test)]
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-discard-project-settings"), discard.rect));
                    (discard.clicked(), stay || stay_button.clicked())
                }).inner
            });
            if response.inner.0 {
                self.discard(app, project);
                return true;
            }
            if response.inner.1 || response.should_close() {
                self.confirm_close = false;
            }
        }
        false
    }

    fn discard(&mut self, app: &mut App, project: ProjectId) {
        crate::model_settings::discard_project_draft(app, project);
        crate::skill_settings::discard_project_draft(app, project);
        crate::mcp_settings::discard_project_draft(app, project);
        let keys: Vec<_> = self.forms().map(|f| (f.method.clone(), f.path.clone(), f.body.clone())).collect();
        for panes in self.panes.values_mut() {
            panes.retain(|pane| !matches!(pane, Dialog::Form(form) if form.settings_baseline.is_some()));
        }
        app.settings.form_drafts.retain(|draft| {
            draft.server != app.server || !keys.iter().any(|(method, path, body)| {
                draft.method == *method && draft.path == *path && crate::dialogs::same_form_body(method, path, body, &draft.body)
            })
        });
        app.settings_dirty = true;
        self.batch = None;
    }

    fn collect_writes(&self, app: &App, project: ProjectId) -> Result<VecDeque<Write>, String> {
        let base = format!("/v1/projects/{project}");
        let mut writes = VecDeque::new();
        for form in self.forms().filter(|form| form.settings_dirty()) {
            let body = form.settings_payload(app).map_err(|error| format!("{}: {error}", form.title))?;
            writes.push_back(Write {
                form: Some(form.id), label: form.title.clone(), method: form.method.clone(),
                path: form.path.clone(), body, retry: None,
            });
        }
        let session_path = format!("{base}/session-limit");
        for write in &writes {
            if write.path == session_path && !valid_limit(&write.body["max_sessions"]) {
                return Err(crate::tr!("Das Sessionlimit muss zwischen 1 und 16 liegen.").into());
            }
        }
        for (suffix, label, body) in [
            ("models", crate::tr!("Modell-Anbieter"), crate::model_settings::project_draft(app, project)),
            ("skills", "Skills", crate::skill_settings::project_draft(app, project)),
            ("mcp/local", "MCPs", crate::mcp_settings::project_draft(app, project)),
        ] {
            if let Some(body) = body {
                writes.push_back(Write {
                    form: None, label: label.into(), method: "PUT".into(), path: format!("{base}/{suffix}"),
                    body, retry: None,
                });
            }
        }
        // Persist shared-revision project changes first and use their confirmed revision for the next one.
        writes.make_contiguous().sort_by_key(|w| {
            if w.path == base { 0 } else if w.path == session_path { 1 } else { 2 }
        });
        let mut outstanding = VecDeque::new();
        for request in app.settings.pending.iter().filter(|r| r.server == app.server && settings_write(project, &r.method, &r.path)) {
            let mut write = if let Some(index) = writes.iter().position(|w| w.path == request.path && w.method == request.method
                && crate::dialogs::same_form_body(&w.method, &w.path, &w.body, &request.body)) {
                let write = writes.remove(index).unwrap();
                let mut previous = request.body.clone();
                previous["request_id"] = write.body["request_id"].clone();
                if previous != write.body {
                    return Err(crate::tr!("Für „{}“ ist eine frühere Anfrage offen. Bitte zuerst in der Diagnose über „Zustellungen prüfen“ abgleichen.", write.label));
                }
                write
            } else {
                Write {
                    form: self.forms().find(|f| f.path == request.path && f.method == request.method && f.matches_body(&request.body))
                        .map(|f| f.id),
                    label: request.label.clone(), method: request.method.clone(), path: request.path.clone(),
                    body: request.body.clone(), retry: None,
                }
            };
            write.retry = Some(request.id);
            outstanding.push_back(write);
        }
        outstanding.append(&mut writes);
        Ok(outstanding)
    }

    fn start_save(&mut self, app: &mut App, project: ProjectId) {
        if let Some(batch) = &mut self.batch {
            if batch.retry_needed {
                app.responses.remove(&batch.current.1);
                app.retry(batch.current.1);
                batch.retry_needed = false;
            }
            return;
        }
        match self.collect_writes(app, project) {
            Ok(mut remaining) => {
                if let Some(write) = remaining.pop_front() {
                    let id = send_write(app, &write);
                    self.batch = Some(SaveBatch { remaining, current: (write, id), retry_needed: false, saved: 0 });
                }
            }
            Err(error) => app.project_notice(crate::app::ToastKind::Error, self.project, error),
        }
    }

    fn advance(&mut self, app: &mut App, project: ProjectId) -> bool {
        let Some(mut batch) = self.batch.take() else { return false; };
        let Some(result) = app.responses.remove(&batch.current.1) else {
            self.batch = Some(batch);
            return false;
        };
        let (write, _) = &batch.current;
        if let Err(error) = &result {
            app.action_errors.push((
                crate::overview::now_ms(),
                crate::tr!("Projekteinstellungen ({} · {}): {}", app.server, write.path, error.message),
            ));
            if app.action_errors.len() > 50 {
                app.action_errors.remove(0);
            }
        }
        match result {
            Ok(value) if self.confirm_write(app, project, write, &value, &mut batch.remaining) => {
                batch.saved += 1;
                if let Some(next) = batch.remaining.pop_front() {
                    let id = send_write(app, &next);
                    batch.current = (next, id);
                    self.batch = Some(batch);
                    false
                } else {
                    app.project_notice(crate::app::ToastKind::Success, self.project, crate::tr!("Projekteinstellungen gespeichert."));
                    true
                }
            }
            Err(error) if error.uncertain => {
                app.project_notice(crate::app::ToastKind::Warning, self.project, crate::tr!("Speichern nicht bestätigt. Mit „Speichern“ dieselbe Anfrage erneut prüfen."));
                batch.retry_needed = true;
                self.batch = Some(batch);
                false
            }
            _ => {
                app.project_notice(crate::app::ToastKind::Error, self.project, if batch.saved == 0 {
                    crate::tr!("„{}“ konnte nicht gespeichert werden. Änderungen bleiben erhalten.", write.label)
                } else {
                    crate::tr!("Nicht alle Änderungen gespeichert: „{}“. Die übrigen Änderungen bleiben erhalten.", write.label)
                });
                false
            }
        }
    }

    fn confirm_write(&mut self, app: &mut App, project: ProjectId, write: &Write, value: &Value, remaining: &mut VecDeque<Write>) -> bool {
        let base = format!("/v1/projects/{project}");
        if write.path == format!("{base}/models") {
            return crate::model_settings::confirm_project_save(app, project, value.clone());
        }
        if write.path == format!("{base}/skills") {
            return crate::skill_settings::confirm_project_save(app, project, value.clone());
        }
        if write.path == format!("{base}/mcp/local") {
            return crate::mcp_settings::confirm_project_save(app, project, value.clone());
        }
        let project_update = write.path == base
            || write.path == format!("{base}/session-limit")
            || write.path == format!("{base}/mirror");
        let revision = value.get("revision").and_then(Value::as_u64);
        let Some(revision) = revision else { return false; };
        if project_update {
            let Ok(updated) = serde_json::from_value::<Project>(value.clone()) else { return false; };
            if updated.id != project { return false; }
            if let Some(snapshot) = &mut app.snapshot {
                if let Some(current) = std::sync::Arc::make_mut(snapshot).projects.iter_mut()
                    .find(|p| p.id == project && p.revision.0 < revision) {
                    *current = updated;
                }
            }
        }
        for next in remaining.iter_mut().filter(|w| w.retry.is_none()) {
            if project_update
                && (next.path == base
                    || next.path == format!("{base}/session-limit")
                    || next.path == format!("{base}/mirror"))
                && next.body["expected_revision"] == write.body["expected_revision"] {
                next.body["expected_revision"] = json!(revision);
            }
        }
        for pane in self.panes.values_mut().flatten() {
            if let Dialog::Form(form) = pane {
                if write.form == Some(form.id) {
                    form.body["expected_revision"] = json!(revision);
                    form.settings_baseline = Some(form.body.clone());
                    form.request = None;
                    app.settings.form_drafts.retain(|draft| draft.server != app.server
                        || draft.path != form.path || draft.method != form.method || !form.matches_body(&draft.body));
                } else if let Some(next) = remaining.iter().find(|w| w.form == Some(form.id)) {
                    if next.path == form.path {
                        form.body["expected_revision"] = next.body["expected_revision"].clone();
                    }
                }
            }
        }
        app.settings_dirty = true;
        true
    }
    fn show_mode(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32, embedded: bool) -> bool {
        let mut close = false;
        ui.horizontal_top(|ui| {
            let nav_width=(ui.available_width()*0.27).clamp(215.0,245.0);
            let _nav=ui.allocate_ui_with_layout(egui::vec2(nav_width,height),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                egui::ScrollArea::vertical().id_salt((self.project,"project-settings-nav")).max_height(height).show(ui,|ui| {
                    ui.set_width(nav_width);
                    for &section in if self.project.is_some() {ProjectSection::ALL.as_slice()} else {ProjectSection::FACTORY.as_slice()} {
                        let row=design::navigation_row(ui,section.title(),self.selected==section,None);
                        #[cfg(test)]
                        ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new(("test-project-section",section.title())),row.rect));
                        if row.clicked() {
                            if let Some(panes)=self.panes.get(&self.selected) {for pane in panes {
                                if !matches!(pane, Dialog::Form(form) if form.settings_baseline.is_some()) {
                                    pane.persist_drafts(app);
                                }
                            }}
                            self.selected=section;
                        }
                    }
                });
            });
            ui.separator();
             let _pane=ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),height),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                 let pane_width=ui.available_width();
                 let pane_bottom=ui.cursor().top()+height;
                let mut content=|ui:&mut egui::Ui| {
                ui.set_width(pane_width);
                  if self.project.is_some_and(|id| !app.snapshot.as_ref().is_some_and(|s| s.projects.iter().any(|p| p.id == id))) {
                      ui.label(crate::tr!("Dieses Projekt ist im aktuellen Serverstand nicht mehr verfügbar. Entwürfe bleiben erhalten."));
                      return;
                 }
                  match (self.selected, self.project) {
                      (ProjectSection::Language, None) => {
                          self.show_language(ui, app, pane_bottom);
                          return;
                      }
                     (ProjectSection::Skills, Some(id)) => {
                          crate::skill_settings::show(ui, app, id, pane_bottom);
                         return;
                     }
                     (ProjectSection::Skills, None) => {
                         crate::factory_skills::show(ui, app, pane_bottom);
                         return;
                     }
                     (ProjectSection::Mcps, Some(id)) => {
                          crate::mcp_settings::show_project(ui, app, id, pane_bottom);
                         return;
                     }
                     (ProjectSection::Mcps, None) => {
                         crate::mcp_settings::show_catalog(ui, app, pane_bottom);
                         return;
                     }
                     _ => {}
                 }
                 if matches!(self.selected, ProjectSection::Models | ProjectSection::DefaultModels) {
                    if let Some(id) = self.project {
                        crate::model_settings::show_project(ui, app, id, pane_bottom);
                    } else if self.selected == ProjectSection::DefaultModels {
                        crate::model_settings::show_factory(ui, app, Some(pane_bottom));
                    } else {
                        let max_height = (pane_bottom-ui.cursor().top()).max(0.0);
                        egui::ScrollArea::vertical().id_salt("model-providers").max_height(max_height)
                            .show(ui, |ui| crate::model_settings::show_factory(ui, app, None));
                    }
                    return;
                }
                  if self.project.is_some() && matches!(self.selected,
                      ProjectSection::General | ProjectSection::Runner | ProjectSection::Mirrors
                          | ProjectSection::Limits | ProjectSection::Supervisor
                  ) {
                     design::section_heading(ui, self.selected.title());
                  }
                  if self.selected == ProjectSection::Devices {
                       design::section_heading(ui, self.selected.title());
                       if app.online && app.access.as_ref().is_some_and(|a| a.role == DeviceRole::Admin)
                           && self.android_push.status.is_none() && self.android_push.request.is_none()
                           && !self.android_push.attempted
                       {
                           self.android_push.attempted = true;
                           self.android_push.send(app, false);
                       }
                  }
                 let panes=self.panes.entry(self.selected).or_default();
                if panes.is_empty() {
                    if let Some(dialog)=content(app,self.project,self.selected) {panes.push(dialog);}
                    else {ui.label(crate::tr!("Dieses Projekt ist im aktuellen Serverstand nicht mehr verfügbar."));return;}
                 }
                 if self.selected == ProjectSection::Supervisor {
                     if let Some(Dialog::Read { request, .. }) = panes.last() {
                         if let Some(result) = app.responses.get(request) {
                             match result {
                                 Ok(value) if value.is_null() => {
                                     ui.label(crate::tr!("Der Supervisor ist für dieses Projekt noch nicht eingerichtet."));
                                     return;
                                 }
                                 Ok(value) => {
                                     if let Ok(policy) = serde_json::from_value::<SupervisorPolicy>(value.clone()) {
                                         let mut editor = Some(Dialog::form(
                                             "Supervisor", "", "PUT", format!("/v1/projects/{}/supervisor", policy.project_id),
                                             json!(SetSupervisor {
                                                 request_id: RequestId::new(),
                                                 expected_revision: policy.revision,
                                                 stream_id: policy.stream_id,
                                                 enabled: policy.enabled,
                                                 evaluation_budget: policy.remaining_evaluations,
                                                 interval_seconds: policy.interval_seconds,
                                             }),
                                         ));
                                         restore_form(app, &mut editor);
                                         *panes.last_mut().unwrap() = editor.unwrap();
                                     } else {
                                         ui.label(crate::tr!("Keine Supervisor-Einstellungen verfügbar."));
                                         return;
                                     }
                                 }
                                 Err(_) => {
                                     if ui.add_enabled(app.online, egui::Button::new(crate::tr!("Erneut laden"))).clicked() {
                                         if let Some(pane) = content(app, self.project, self.selected) {
                                             *panes.last_mut().unwrap() = pane;
                                         }
                                     }
                                     return;
                                 }
                             }
                         }
                     }
                 }
                 let staging = matches!(panes.last(), Some(Dialog::Form(form)) if form.settings_dirty());
                 let back=panes.len()>1 && !staging && ui.button(crate::tr!("Zurück zur Übersicht")).clicked();
                 if back {if let Some(pane)=panes.pop() {
                     if !matches!(&pane, Dialog::Form(form) if form.settings_baseline.is_some()) {
                         pane.persist_drafts(app);
                     }
                 }}
                let remaining=(height-100.0).max(80.0);
                let pane=panes.last_mut().expect("project pane initialized");
                let pane_close=match pane {
                    Dialog::Form(form)=>form.show_in_settings(ui,app,pane_bottom,(remaining-145.0).max(40.0)),
                     Dialog::Read {request,path,..}=>{
                         let devices = self.selected == ProjectSection::Devices;
                         let scroll = if devices { egui::ScrollArea::vertical().max_height((pane_bottom-ui.cursor().top()).max(0.0)) }
                             else { egui::ScrollArea::both().max_height((remaining-50.0).max(40.0)) };
                         scroll.id_salt((self.project,self.selected,"read")).show(ui,|ui| {
                            match app.responses.get(request).cloned() {
                                Some(Ok(value))=>read_content(ui,app,&value,path),
                                Some(Err(error))=>{
                                    if self.selected == ProjectSection::Devices {
                                        let shown = egui::Id::new(("devices-load-error", *request));
                                        if !ui.ctx().data_mut(|d| d.get_temp::<bool>(shown).unwrap_or(false)) {
                                            ui.ctx().data_mut(|d| d.insert_temp(shown, true));
                                            app.notice_error(crate::tr!("Gerätezugänge konnten nicht geladen werden."));
                                            app.action_errors.push((crate::overview::now_ms(), error.message));
                                         }
                                     } else { ui.colored_label(crate::app::RED,error.message); }
                                 }
                                 None=>{ui.spinner();ui.label(crate::tr!("Wird geladen …"));}
                             }
                             if devices { self.android_push.show(ui, app); }
                        });
                        if self.selected != ProjectSection::Devices {
                            if let Some(Ok(value))=app.responses.get(request) {if ui.button(crate::tr!("Exportieren …")).clicked() {app.backend.save_json("yoloforge-projekteinstellungen.json",value.clone());}}
                        }
                        false
                    }
                    _=>{ui.label(crate::tr!("Diese Ansicht bitte über die Hauptnavigation öffnen."));false}
                };
                 if pane_close {
                      if panes.len()>1 {if let Some(pane)=panes.pop() {pane.persist_drafts(app);}}
                      else if embedded {
                        if let Some(pane)=panes.pop() {pane.persist_drafts(app);}
                        self.selected=ProjectSection::Models;
                    } else {close=true;}
                }
                  if let Some(mut next)=app.dialog.take() {
                      if self.project.is_some() {
                          if let Dialog::Form(form) = &mut next { form.stage_in_project_settings(); }
                      }
                      self.panes.entry(self.selected).or_default().push(next);
                  }
                };
                if embedded {
                    egui::ScrollArea::both().id_salt("factory-settings-page-content").max_height(height).auto_shrink([false,false]).show(ui,content);
                } else {content(ui);}
            });
            #[cfg(test)]
            ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("test-settings-geometry"),(height,_nav.response.rect,_pane.response.rect)));
        });
        close
    }

    fn show_language(&mut self, ui: &mut egui::Ui, app: &mut App, bottom: f32) {
        use crate::i18n::Language;
        let draft = self.language_draft.get_or_insert(app.settings.language);
        let saving = app.language_save.is_some();
        let content_height = (bottom - ui.cursor().top()).max(0.0);
        egui::ScrollArea::vertical().id_salt("language-settings-content").max_height(content_height)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                design::section_heading(ui, crate::tr!("Language"));
                ui.add_enabled_ui(!saving, |ui| {
                        let _combo = egui::ComboBox::from_id_salt("ui-language")
                            .selected_text(draft.label())
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for language in Language::ALL {
                                    let _option = ui.selectable_value(draft, language, language.label());
                                    #[cfg(test)]
                                    ui.ctx().data_mut(|d| d.insert_temp(
                                        egui::Id::new(("test-language-option", language.label())), _option.rect));
                                }
                            });
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test-language-combo"), _combo.response.rect));
                });
                ui.add_space(design::SETTINGS_FIELD_GAP);
                ui.label(crate::tr!("This setting applies only to this device."));
            });
    }
}

fn settings_write(project: ProjectId, method: &str, path: &str) -> bool {
    let base = format!("/v1/projects/{project}");
    match (method, path.strip_prefix(&base)) {
        ("PATCH", Some("")) => true,
        ("POST", Some("/session-limit")) => true,
        ("PUT", Some("/supervisor" | "/token-budget" | "/models" | "/skills" | "/mcp/local")) => true,
        _ => false,
    }
}

fn valid_limit(value: &Value) -> bool {
    value.as_u64().is_some_and(|n| (1..=16).contains(&n))
}

fn send_write(app: &mut App, write: &Write) -> u64 {
    if let Some(id) = write.retry {
        if !app.in_flight.contains(&id) && app.responses.get(&id).is_none_or(|result| result.is_err()) {
            app.responses.remove(&id);
            app.retry(id);
        }
        id
    } else {
        app.write(&write.label, &write.method, write.path.clone(), write.body.clone())
    }
}

fn content(app: &mut App, id: Option<ProjectId>, section: ProjectSection) -> Option<Dialog> {
    if id.is_none() {
        let mut result=match section {
            ProjectSection::Devices=>{app.read(crate::tr!("Geräte und Zugänge"),"/v1/devices".into());app.dialog.take()},
            _=>None,
        };
        restore_form(app, &mut result);
        return result;
    }
    let id = id?;
    let p = app
        .snapshot
        .as_ref()?
        .projects
        .iter()
        .find(|p| p.id == id)?
        .clone();
    if matches!(section, ProjectSection::General | ProjectSection::Runner) {
        split_legacy_project_draft(app, &p);
    }
    let saved = app.dialog.take();
    match section {
        ProjectSection::General => app.project_editor(Some(&p)),
        ProjectSection::Runner => {
            app.dialog = Some(Dialog::form(
                crate::tr!("Runner-Zuordnung"),
                "",
                "PATCH",
                format!("/v1/projects/{}", p.id),
                json!({
                    "request_id": RequestId::new(),
                    "expected_revision": p.revision,
                    "runner_mapping": p.runner_mapping.unwrap_or(ProjectRunnerMapping::Automatic),
                }),
            ))
        }
        ProjectSection::Mirrors => {
            let local = app.local_runners.iter().find(|runner| runner.visible(&app.server));
            let runner = local.map(|runner| runner.id).unwrap_or_else(RunnerId::new);
            let mirror = p.mirrors.get(&runner).map_or("", |mirror| mirror.path.as_str());
            app.dialog = Some(Dialog::form(
                crate::tr!("Lokale Spiegelkopie"),
                &crate::tr!("Jeder Desktop kann einen eigenen Checkout des bestätigten Factory-Projektstands erhalten."),
                "PUT",
                format!("/v1/projects/{}/mirror", p.id),
                json!(SetProjectMirror {
                    request_id: RequestId::new(),
                    expected_revision: p.revision,
                    runner_id: runner,
                    path: mirror.to_owned(),
                }),
            ))
        }
        ProjectSection::Limits => {
            app.dialog = Some(Dialog::form(
                "Limits",
                "",
                "POST",
                format!("/v1/projects/{}/session-limit", p.id),
                json!(SetSessionLimit {
                    request_id: RequestId::new(),
                    expected_revision: p.revision,
                    max_sessions: p.max_sessions,
                }),
            ));
        }
        ProjectSection::Supervisor => app.read(
            crate::tr!("Supervisor-Konfiguration"),
            format!("/v1/projects/{}/supervisor", p.id),
        ),
        _ => {}
    }
    let mut result = app.dialog.take();
    restore_form(app, &mut result);
    app.dialog = saved;
    result
}

fn split_legacy_project_draft(app: &mut App, project: &Project) {
    let path = format!("/v1/projects/{}", project.id);
    let drafts = &mut app.settings.form_drafts;
    let for_project = |draft: &crate::persistence::FormDraft| {
        draft.server == app.server && draft.path == path && draft.method == "PATCH"
    };
    let has_runner_draft = drafts.iter().any(|d| for_project(d) && d.body.get("name").is_none());
    let Some(legacy) = drafts.iter_mut().find(|d| {
        for_project(d) && d.body.get("name").is_some() && d.body.get("runner_mapping").is_some()
    }) else {
        return;
    };
    let Some(mapping) = legacy.body.as_object_mut().and_then(|body| body.remove("runner_mapping")) else {
        return;
    };
    if !has_runner_draft
        && !mapping.is_null()
        && mapping != json!(project.runner_mapping.unwrap_or(ProjectRunnerMapping::Automatic))
    {
        let runner = crate::persistence::FormDraft {
            server: app.server.clone(),
            title: ProjectSection::Runner.title().into(),
            help: String::new(),
            method: "PATCH".into(),
            path,
            body: json!({
                "request_id": RequestId::new(),
                "expected_revision": legacy.body["expected_revision"],
                "runner_mapping": mapping,
            }),
        };
        drafts.push(runner);
    }
    app.settings_dirty = true;
}

pub(crate) fn restore_form(app: &App, result: &mut Option<Dialog>) {
    if let Some(Dialog::Form(form)) = result {
        form.stage_in_project_settings();
        if let Some(draft) = app.settings.form_drafts.iter().find(|d| {
            d.server == app.server
                && d.path == form.path
                && d.method == form.method
                && form.matches_body(&d.body)
        }) {
            if form.path.ends_with("/supervisor") && form.settings_baseline.is_some() {
                if let Some(seconds) = draft.body["interval_seconds"].as_u64().filter(|seconds| (30..=86400).contains(seconds)) {
                    form.body["interval_seconds"] = json!(seconds);
                }
            } else {
                form.body = draft.body.clone();
            }
        }
        if form.is_project_update() && !form.is_runner_mapping() {
            // Older edit dialogs stored both sets of fields in a single draft.
            form.body.as_object_mut().unwrap().remove("runner_mapping");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(settings: &mut ProjectSettings, app: &mut App, section: ProjectSection, field: &str, value: Value) {
        let mut pane = content(app, settings.project, section).unwrap();
        let Dialog::Form(form) = &mut pane else { panic!("expected form") };
        form.body[field] = value;
        settings.panes.insert(section, vec![pane]);
    }

    fn reply(app: &mut App, result: Result<Value, crate::backend::Failure>) {
        let id = app.settings.pending[0].id;
        if !result.as_ref().is_err_and(|err| err.uncertain) {
            app.settings.pending.clear();
        }
        app.in_flight.remove(&id);
        app.responses.insert(id, result);
    }

    #[test]
    fn supervisor_only_exposes_the_interval_and_preserves_existing_policy_fields() {
        let mut app = crate::tests::fixture();
        let project = app.project().unwrap().id;
        let mut settings = ProjectSettings::new(project);
        settings.selected = ProjectSection::Supervisor;
        let policy = SupervisorPolicy {
            project_id: project,
            stream_id: StreamId::new(),
            revision: Revision(3),
            enabled: true,
            remaining_evaluations: 7,
            interval_seconds: 300,
            next_unix_ms: 0,
        };
        app.responses.insert(50, Ok(json!(policy)));
        settings.panes.insert(ProjectSection::Supervisor, vec![Dialog::read(
            "Supervisor".into(), 50, format!("/v1/projects/{project}/supervisor"),
        )]);
        let ctx = app.backend.context().clone();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| settings.show(ui, &mut app, 400.0));
        });
        let mut text = Vec::new();
        for shape in &output.shapes {
            crate::tests::shape_text(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|text| text == "Prüfintervall in Minuten"));
        assert!(text.iter().any(|text| text.trim().parse::<f64>() == Ok(5.0)), "{text:?}");
        for hidden in ["Budget", "Anfragedaten", "Revision", "Arbeitsbereich", "Vollständige Daten"] {
            assert!(!text.iter().any(|text| text.contains(hidden)), "{text:?}");
        }
        assert!(settings.collect_writes(&app, project).unwrap().is_empty());
        let Dialog::Form(form) = settings.panes.get_mut(&ProjectSection::Supervisor).unwrap().last_mut().unwrap() else {
            unreachable!();
        };
        form.body["interval_seconds"] = json!(600);
        let writes = settings.collect_writes(&app, project).unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].body["interval_seconds"], 600);
        assert_eq!(writes[0].body["evaluation_budget"], 7);
        assert_eq!(writes[0].body["stream_id"], json!(policy.stream_id));
        assert_eq!(writes[0].body["expected_revision"], 3);
    }

    #[test]
    fn global_save_waits_for_all_views_and_uses_the_confirmed_project_revision() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let mut project = app.project().unwrap().clone();
        let runner = app.snapshot.as_ref().unwrap().runners[0].id;
        let mut dialog = Dialog::project_settings(project.id);
        let Dialog::ProjectSettings(settings) = &mut dialog else { unreachable!() };
        edit(settings, &mut app, ProjectSection::General, "name", json!("Neuer Name"));
        edit(settings, &mut app, ProjectSection::Runner, "runner_mapping", json!(ProjectRunnerMapping::Runner(runner)));
        edit(settings, &mut app, ProjectSection::Mirrors, "path", json!("/tmp/project-mirror"));
        settings.selected = ProjectSection::Runner;

        let ctx = app.backend.context().clone();
        let draw = |app: &mut App, dialog: &mut Dialog, events| {
            let mut close = false;
            let output = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(950.0, 650.0))),
                events, ..Default::default()
            }, |ctx| { close = dialog.show(ctx, app); });
            (close, output)
        };
        for _ in 0..3 {
            assert!(!draw(&mut app, &mut dialog, vec![]).0);
        }
        let (_, output) = draw(&mut app, &mut dialog, vec![]);
        let labels = output.shapes.iter().filter(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == ProjectSection::Runner.title()
        )).count();
        assert_eq!(labels, 2, "Runner mapping should appear once in the sidebar and once above the dropdown");
        let save: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-settings-save"))).unwrap();
        assert!(!draw(&mut app, &mut dialog, crate::tests::click_events(save.center())).0);
        assert_eq!(app.settings.pending.len(), 1);
        assert_eq!(app.settings.pending[0].body["name"], "Neuer Name");
        let escape = egui::Event::Key {
            key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE,
        };
        let (close, _) = draw(&mut app, &mut dialog, vec![escape]);
        assert!(!close);
        let Dialog::ProjectSettings(settings) = &dialog else { unreachable!() };
        assert!(settings.is_saving());
        assert!(!settings.confirmation_open());
        project.revision.0 += 1;
        project.name = "Neuer Name".into();
        reply(&mut app, Ok(json!(project)));
        assert!(!draw(&mut app, &mut dialog, vec![]).0);
        assert_eq!(app.settings.pending.len(), 1);
        assert_eq!(app.settings.pending[0].body["expected_revision"], json!(project.revision));
        assert_eq!(app.settings.pending[0].body["runner_mapping"], json!(ProjectRunnerMapping::Runner(runner)));
        project.revision.0 += 1;
        project.runner_mapping = Some(ProjectRunnerMapping::Runner(runner));
        reply(&mut app, Ok(json!(project)));
        assert!(!draw(&mut app, &mut dialog, vec![]).0);
        assert_eq!(app.settings.pending.len(), 1);
        assert_eq!(app.settings.pending[0].path, format!("/v1/projects/{}/mirror", project.id));
        assert_eq!(app.settings.pending[0].body["expected_revision"], json!(project.revision));
        let mirror_runner: RunnerId = serde_json::from_value(
            app.settings.pending[0].body["runner_id"].clone(),
        ).unwrap();
        project.revision.0 += 1;
        project.mirrors.insert(mirror_runner, ProjectMirror { path: "/tmp/project-mirror".into() });
        reply(&mut app, Ok(json!(project)));
        assert!(draw(&mut app, &mut dialog, vec![]).0);
        assert!(!app.settings.form_drafts.iter().any(|draft| draft.path == format!("/v1/projects/{}", project.id)));
    }

    #[test]
    fn failed_batch_keeps_remaining_edits_and_discard_requires_confirmation() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let mut project = app.project().unwrap().clone();
        let mut settings = ProjectSettings::new(project.id);
        edit(&mut settings, &mut app, ProjectSection::General, "name", json!("Geändert"));
        edit(&mut settings, &mut app, ProjectSection::Limits, "max_sessions", json!(3));
        settings.start_save(&mut app, project.id);
        project.revision.0 += 1;
        project.name = "Geändert".into();
        reply(&mut app, Ok(json!(project)));
        assert!(!settings.advance(&mut app, project.id));
        reply(&mut app, Err(crate::backend::Failure { message: "test".into(), uncertain: false }));
        assert!(!settings.advance(&mut app, project.id));
        assert_eq!(settings.forms().filter(|f| f.settings_dirty()).count(), 1);
        assert!(!settings.request_close(&app));
        assert!(settings.confirmation_open());
        assert!(settings.has_changes(&app, project.id));
        settings.discard(&mut app, project.id);
        assert!(!settings.has_changes(&app, project.id));
        assert_eq!(app.project().unwrap().name, "Geändert");
    }

    #[test]
    fn cancel_opens_a_confirmation_and_only_discard_closes_the_dialog() {
        let mut app = crate::tests::fixture();
        let id = app.project().unwrap().id;
        let mut dialog = Dialog::project_settings(id);
        let Dialog::ProjectSettings(settings) = &mut dialog else { unreachable!() };
        edit(settings, &mut app, ProjectSection::General, "name", json!("Entwurf"));
        let ctx = app.backend.context().clone();
        let mut render = |events| {
            let mut close = false;
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(950.0, 650.0))),
                events, ..Default::default()
            }, |ctx| { close = dialog.show(ctx, &mut app); });
            close
        };
        let click = |key| {
            let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(key))).unwrap();
            crate::tests::click_events(rect.center())
        };
        for _ in 0..3 {
            assert!(!render(vec![]));
        }
        assert!(!render(click("test-project-settings-cancel")));
        for _ in 0..2 {
            assert!(!render(vec![]));
        }
        assert!(render(click("test-discard-project-settings")));
        assert!(app.settings.pending.is_empty());
        assert!(app.settings.form_drafts.is_empty());
    }

    #[test]
    fn project_logo_is_saved_with_general_settings_and_only_confirmed_artwork_reaches_the_snapshot() {
        let mut png = std::io::Cursor::new(vec![]);
        image::DynamicImage::new_rgba8(PROJECT_LOGO_SIDE, PROJECT_LOGO_SIDE)
            .write_to(&mut png, image::ImageFormat::Png).unwrap();
        let logo = ProjectLogo::from_png(png.get_ref()).unwrap();
        for success in [false, true] {
            let mut app = crate::tests::fixture();
            let mut project = app.project().unwrap().clone();
            let mut settings = ProjectSettings::new(project.id);
            edit(&mut settings, &mut app, ProjectSection::General, "logo",
                json!(ProjectLogoUpdate { image: Some(logo.clone()) }));
            assert!(settings.has_changes(&app, project.id));
            assert!(app.settings.pending.is_empty());
            assert!(app.project().unwrap().logo.is_none());
            settings.start_save(&mut app, project.id);
            assert_eq!(app.settings.pending.len(), 1);
            assert_eq!(app.settings.pending[0].body["logo"]["image"], json!(logo));
            if success {
                project.logo = Some(logo.clone());
                project.revision.0 += 1;
                reply(&mut app, Ok(json!(project)));
                assert!(settings.advance(&mut app, project.id));
                assert_eq!(app.project().unwrap().logo, Some(logo.clone()));
                assert!(!settings.has_changes(&app, project.id));
            } else {
                reply(&mut app, Err(crate::backend::Failure { message: "test".into(), uncertain: false }));
                assert!(!settings.advance(&mut app, project.id));
                assert!(settings.has_changes(&app, project.id));
                assert!(app.project().unwrap().logo.is_none());
            }
        }
    }
}
