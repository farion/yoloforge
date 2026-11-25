use crate::{app::App, design, model_settings::ephemeral};
use factory_core::{DeviceRole, LocalMcp, ProjectId, RequestId, Revision, RunnerId};
use factory_protocol::mcp::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const CATALOG: &str = "/v1/mcp/local";
const CHOICES: &str = "/v1/mcp/local/choices";

#[derive(Default)]
pub struct Settings {
    catalog: Editor<LocalMcpCatalog>,
    choices: Editor<Vec<Choice>>,
    projects: BTreeMap<ProjectId, Editor<ProjectLocalMcps>>,
    remote: RemoteEditor,
    local_probe: LocalProbe,
    modal: Option<CatalogDialog>,
    add_remote: bool,
    new_local: Option<LocalMcp>,
    new_remote: Option<RemoteDraft>,
    edit_local: Option<(String, Revision, LocalMcp)>,
    local_drafts: BTreeMap<String, (Revision, LocalMcp)>,
    edit_remote: BTreeMap<String, RemoteDraft>,
    delete_remote: Option<(u64, String)>,
}

#[derive(Clone)]
enum CatalogDialog {
    Add,
    EditLocal,
    EditRemote,
    Delete {
        id: String,
        name: String,
        revision: Revision,
        remote: bool,
    },
}

#[derive(serde::Deserialize, Clone)]
struct Choice {
    id: String,
    name: String,
    #[serde(default)]
    remote: bool,
    #[serde(default)]
    revision: Option<Revision>,
}

struct Editor<T> {
    saved: Option<T>,
    draft: Option<T>,
    pending: Option<(u64, bool)>,
    load_failed: bool,
}

impl<T> Default for Editor<T> {
    fn default() -> Self {
        Self {
            saved: None,
            draft: None,
            pending: None,
            load_failed: false,
        }
    }
}

impl<T: Clone + DeserializeOwned> Editor<T> {
    /// Returns true only for a confirmed save.
    fn refresh(&mut self, ui: &mut egui::Ui, app: &mut App, path: &str) -> bool {
        let mut saved = false;
        if let Some((id, writing)) = self.pending {
            if let Some(result) = app.responses.remove(&id) {
                self.pending = None;
                match result
                    .ok()
                    .and_then(|value| serde_json::from_value::<T>(value).ok())
                {
                    Some(value) => {
                        self.draft = Some(value.clone());
                        self.saved = Some(value);
                        if writing {
                            app.notice(crate::tr!("MCP-Einstellungen gespeichert."));
                            saved = true;
                        }
                    }
                    None => {
                        self.load_failed = !writing;
                        app.notice(if writing {
                            crate::tr!("MCP-Einstellungen konnten nicht gespeichert werden. Der Entwurf bleibt erhalten.")
                        } else {
                            crate::tr!("MCP-Einstellungen konnten nicht geladen werden.")
                        });
                    }
                }
            }
        }
        if self.saved.is_none() && self.pending.is_none() {
            if self.load_failed {
                if ui
                    .add_enabled(app.online, egui::Button::new(crate::tr!("Erneut laden")))
                    .clicked()
                {
                    self.load_failed = false;
                }
            } else if app.online {
                self.pending = Some((ephemeral(app, "GET", path.into(), Value::Null), false));
            }
        }
        if self.saved.is_none() && self.pending.is_some() {
            ui.spinner();
            ui.label(crate::tr!("MCP-Einstellungen werden geladen …"));
        }
        saved
    }

    fn save(&mut self, app: &mut App, path: &str, value: Value) {
        self.pending = Some((ephemeral(app, "PUT", path.into(), value), true));
    }
}

pub fn show_catalog(ui: &mut egui::Ui, app: &mut App, bottom: f32) {
    let mut state = std::mem::take(&mut app.mcp_settings);
    if state.catalog.refresh(ui, app, CATALOG) {
        match state.modal.take() {
            Some(CatalogDialog::Add) => {
                state.new_local = None;
                state.new_remote = state.remote.draft.take();
            }
            Some(CatalogDialog::EditLocal) => state.edit_local = None,
            _ => {}
        }
        state.invalidate_choices(app);
    }
    if state.remote.update(ui, app) {
        if matches!(state.modal, Some(CatalogDialog::Add)) {
            state.new_remote = None;
        }
        state.remote.draft = None;
        state.modal = None;
        state.invalidate_choices(app);
    }
    if let Some((request, id)) = state.delete_remote.as_ref() {
        if let Some(result) = app.responses.remove(request) {
            let id = id.clone();
            state.delete_remote = None;
            match result {
                Ok(value) if value == Value::Bool(true) => {
                    if let Some(entries) = &mut state.remote.entries.saved {
                        entries.retain(|entry| entry.reference.id != id);
                    }
                    state.edit_remote.remove(&id);
                    state.modal = None;
                    state.invalidate_choices(app);
                    app.notice(crate::tr!("MCP gelöscht."));
                }
                Err(error) => app.notice(error.message),
                _ => app.notice(crate::tr!("MCP konnte nicht gelöscht werden.")),
            }
        }
    }
    state.local_probe.update(ui, app);
    let editable = app.online && !state.busy();
    let has_catalog = state.catalog.saved.is_some() || state.remote.entries.saved.is_some();
    design::section_heading(ui, crate::tr!("MCPs hinzufügen"));
    let add = ui.add_enabled(editable && has_catalog, egui::Button::new(crate::tr!("MCP hinzufügen")));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new("test-mcp-add"), add.rect));
    if add.clicked() {
        state.modal = Some(CatalogDialog::Add);
        state.remote.draft = state.new_remote.take();
        state.remote.status = None;
        state.remote.verified_attempt = None;
    }
    ui.add_space(16.0);
    design::section_heading(ui, crate::tr!("Vorhandene MCPs"));
    let local = state.catalog.saved.clone();
    let remote = state.remote.entries.saved.clone();
    if let Some(entries) = &remote {
        let references: Vec<_> = entries.iter().map(|entry| entry.reference.clone()).collect();
        state.remote.connections.update(ui, app, &references);
    }
    egui::ScrollArea::vertical()
        .id_salt("existing-mcps")
        .max_height((bottom - ui.cursor().top()).max(0.0))
        .show(ui, |ui| {
            if local.as_ref().is_some_and(|items| items.entries.is_empty())
                && remote.as_ref().is_some_and(Vec::is_empty)
            {
                ui.label(crate::tr!("Noch keine MCPs vorhanden."));
            }
            if let Some(catalog) = local {
                for entry in &catalog.entries {
                    let (edit, delete) =
                        mcp_row(ui, &entry.id, &entry.name, crate::tr!("Lokal"), None, editable);
                    if edit {
                        if state
                            .edit_local
                            .as_ref()
                            .is_none_or(|(id, _, _)| id != &entry.id)
                        {
                            let (rev, draft) = state
                                .local_drafts
                                .remove(&entry.id)
                                .unwrap_or_else(|| (catalog.revision, entry.clone()));
                            state.edit_local = Some((entry.id.clone(), rev, draft));
                        }
                        state.modal = Some(CatalogDialog::EditLocal);
                    }
                    if delete {
                        state.modal = Some(CatalogDialog::Delete {
                            id: entry.id.clone(),
                            name: entry.name.clone(),
                            revision: catalog.revision,
                            remote: false,
                        });
                    }
                }
            }
            for entry in remote.into_iter().flatten() {
                let reference = &entry.reference;
                let status = state.remote.connections.status(reference);
                let (edit, delete) = mcp_row(
                    ui,
                    &reference.id,
                    &reference.name,
                    "Remote",
                    Some(status),
                    editable,
                );
                if edit {
                    state.remote.status = None;
                    state.remote.verified_attempt = None;
                    state.remote.draft = state
                        .edit_remote
                        .remove(&reference.id)
                        .or_else(|| Some(RemoteDraft::from_entry(&entry)));
                    state.modal = Some(CatalogDialog::EditRemote);
                }
                if delete {
                    state.modal = Some(CatalogDialog::Delete {
                        id: reference.id.clone(),
                        name: reference.name.clone(),
                        revision: reference.revision,
                        remote: true,
                    });
                }
            }
        });
    state.show_modal(ui.ctx(), app);
    app.mcp_settings = state;
}

pub(crate) fn settings_modal<R>(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    body: impl FnOnce(&mut egui::Ui, f32) -> R,
) -> (R, bool) {
    let size = design::dialog_content_size(ctx.content_rect().size());
    let modal = egui::Modal::new(egui::Id::new(id))
        .frame(egui::Frame::window(&ctx.style()).inner_margin(design::DIALOG_PADDING))
        .show(ctx, |ui| {
            ui.set_width(size.x.min(560.0));
            let top = ui.cursor().top();
            let close = design::dialog_header(ui, title).clicked();
            ui.separator();
            (
                body(ui, (top + size.y - ui.cursor().top() - 62.0).max(34.0)),
                close,
            )
        });
    let close = modal.should_close() || modal.inner.1;
    #[cfg(test)]
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(("test-settings-modal", id)),
            modal.response.rect,
        )
    });
    (modal.inner.0, close)
}

fn dialog_actions(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    ui.separator();
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), design::SETTINGS_FOOTER_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if label == crate::tr!("Speichern") {
                design::settings_save_button(ui, enabled).clicked()
            } else {
                ui.add_enabled(enabled, egui::Button::new(label)).clicked()
            }
        },
    )
    .inner
}

fn new_mcp_id() -> String {
    format!("mcp-{}", RequestId::new())
}

fn dropdown<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    selected: &mut T,
    options: &[(T, &str)],
) {
    let label = options
        .iter()
        .find(|(value, _)| value == selected)
        .map_or("", |(_, label)| *label);
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(ui.available_width().clamp(132.0, 420.0))
        .show_ui(ui, |ui| {
            for &(value, label) in options {
                if design::navigation_row(ui, label, *selected == value, None).clicked() {
                    *selected = value;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
}

impl Settings {
    fn invalidate_choices(&mut self, app: &mut App) {
        if let Some((request, _)) = self.choices.pending.take() {
            app.responses.remove(&request);
        }
        self.choices = Default::default();
    }

    fn busy(&self) -> bool {
        self.catalog.pending.is_some_and(|(_, writing)| writing)
            || self.remote.busy()
            || self.delete_remote.is_some()
    }

    fn show_modal(&mut self, ctx: &egui::Context, app: &mut App) {
        let Some(mode) = self.modal.clone() else {
            return;
        };
        let title = match mode {
            CatalogDialog::Add => crate::tr!("MCP hinzufügen"),
            CatalogDialog::EditLocal | CatalogDialog::EditRemote => crate::tr!("MCP bearbeiten"),
            CatalogDialog::Delete { .. } => crate::tr!("MCP löschen"),
        };
        let busy = self.busy();
        let editable = app.online && !busy;
        let (_, close) = settings_modal(ctx, "mcp-catalog-dialog", title, |ui, max_height| {
            match &mode {
                CatalogDialog::Delete {
                    id,
                    name,
                    revision,
                    remote,
                } => {
                    egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
                        ui.label(crate::tr!("„{name}“ aus dem Factory-Katalog löschen?", name = name));
                        ui.label(crate::tr!("Ist der MCP in einem Projekt freigegeben, muss die Freigabe zuerst entfernt werden."));
                    });
                    if dialog_actions(ui, crate::tr!("Löschen"), editable) {
                        if *remote {
                            let body = RemoveRemoteMcp {
                                request_id: RequestId::new(),
                                expected_revision: *revision,
                            };
                            self.delete_remote = Some((
                                ephemeral(app, "DELETE", format!("{REMOTE}/{id}"), json!(body)),
                                id.clone(),
                            ));
                        } else if let Some(mut catalog) = self.catalog.saved.clone() {
                            catalog.revision = *revision;
                            catalog.entries.retain(|entry| &entry.id != id);
                            self.catalog.save(
                                app,
                                CATALOG,
                                json!(SetLocalMcpCatalog {
                                    request_id: RequestId::new(),
                                    catalog
                                }),
                            );
                        }
                    }
                }
                _ => {
                    let is_new = matches!(mode, CatalogDialog::Add);
                    let revision = if is_new {
                        self.catalog
                            .saved
                            .as_ref()
                            .map_or(Revision(0), |catalog| catalog.revision)
                    } else {
                        self.edit_local
                            .as_ref()
                            .map_or(Revision(0), |(_, revision, _)| *revision)
                    };
                    let remote = egui::ScrollArea::vertical()
                        .id_salt("mcp-form")
                        .auto_shrink([true, true])
                        .max_height(max_height)
                        .show(ui, |ui| {
                            let mut remote = matches!(mode, CatalogDialog::EditRemote)
                                || (is_new && self.add_remote);
                            ui.add_enabled_ui(is_new && editable, |ui| {
                                design::settings_field(ui, crate::tr!("Typ"), |ui| {
                                    dropdown(
                                        ui,
                                        "mcp-type",
                                        &mut remote,
                                        &[(false, crate::tr!("Lokal")), (true, "Remote")],
                                    );
                                });
                            });
                            if is_new {
                                self.add_remote = remote;
                            }
                            if remote {
                                self.remote.draft.get_or_insert_with(RemoteDraft::new);
                                self.remote.fields(ui, app);
                            } else {
                                let entry = if is_new {
                                    Some(self.new_local.get_or_insert_with(|| LocalMcp {
                                        id: new_mcp_id(),
                                        name: String::new(),
                                        command: vec![String::new()],
                                        runners: vec![],
                                        all_runners: true,
                                        excluded_runners: vec![],
                                    }))
                                } else {
                                    self.edit_local.as_mut().map(|(_, _, draft)| draft)
                                };
                                if let Some(entry) = entry {
                                    ui.add_enabled_ui(editable, |ui| {
                                        entry_fields(ui, app, entry);
                                        if !is_new {
                                            let saved = self
                                                .catalog
                                                .saved
                                                .as_ref()
                                                .is_some_and(|s| s.entries.contains(entry));
                                            self.local_probe.show(ui, app, entry, revision, saved);
                                        }
                                    });
                                }
                            }
                            remote
                        })
                        .inner;
                    if remote {
                        if dialog_actions(
                            ui,
                            crate::tr!("Speichern"),
                            editable
                                && self.remote.entries.saved.is_some()
                                && self.remote.status == Some(true)
                                && self.remote.draft.as_ref().is_some_and(RemoteDraft::valid),
                        ) {
                            self.remote.submit(app);
                        }
                    } else {
                        let entry = if is_new {
                            self.new_local.as_ref()
                        } else {
                            self.edit_local.as_ref().map(|(_, _, draft)| draft)
                        };
                        let Some(entry) = entry else { return };
                        let valid = !entry.name.trim().is_empty()
                            && entry
                                .command
                                .first()
                                .is_some_and(|path| std::path::Path::new(path).is_absolute());
                        if dialog_actions(
                            ui,
                            crate::tr!("Speichern"),
                            editable && valid && self.catalog.saved.is_some(),
                        ) {
                            if let Some(mut catalog) = self.catalog.saved.clone() {
                                catalog.revision = revision;
                                if !is_new {
                                    catalog.entries.retain(|item| item.id != entry.id);
                                }
                                catalog.entries.push(entry.clone());
                                self.catalog.save(
                                    app,
                                    CATALOG,
                                    json!(SetLocalMcpCatalog {
                                        request_id: RequestId::new(),
                                        catalog
                                    }),
                                );
                            }
                        }
                    }
                }
            }
        });
        if close && !self.busy() {
            match mode {
                CatalogDialog::Add => self.new_remote = self.remote.draft.take(),
                CatalogDialog::EditLocal => {
                    if let Some((id, revision, entry)) = self.edit_local.take() {
                        self.local_drafts.insert(id, (revision, entry));
                    }
                }
                CatalogDialog::EditRemote => {
                    if let Some(draft) = self.remote.draft.take() {
                        self.edit_remote.insert(draft.id.clone(), draft);
                    }
                }
                CatalogDialog::Delete { .. } => {}
            }
            self.modal = None;
        }
    }
}

fn mcp_row(
    ui: &mut egui::Ui,
    id: &str,
    name: &str,
    kind: &str,
    connection: Option<ConnectionStatus>,
    enabled: bool,
) -> (bool, bool) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let delete =
                    design::icon_button(ui, egui_phosphor::regular::TRASH, crate::tr!("MCP löschen"), enabled)
                        .clicked();
                let edit = design::icon_button(
                    ui,
                    egui_phosphor::regular::PENCIL_SIMPLE,
                    crate::tr!("MCP bearbeiten"),
                    enabled,
                )
                .clicked();
                if let Some(connection) = connection {
                    connection.label(ui);
                }
                ui.label(kind);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(name).truncate())
                        .on_hover_text(name);
                });
                (edit, delete)
            })
            .inner
        })
        .inner
    })
    .inner
}

#[derive(Default)]
struct LocalProbe {
    runner: Option<RunnerId>,
    target: String,
    request: Option<u64>,
    job: Option<RequestId>,
    next: Option<Instant>,
    result: Option<LocalMcpProbeStatus>,
}

impl LocalProbe {
    fn update(&mut self, ui: &egui::Ui, app: &mut App) {
        if let Some(result) = self.request.and_then(|id| app.responses.remove(&id)) {
            self.request = None;
            match result
                .ok()
                .and_then(|value| serde_json::from_value::<LocalMcpProbeView>(value).ok())
            {
                Some(view) if view.status == LocalMcpProbeStatus::Pending => {
                    self.job = Some(view.id);
                    self.next = Some(Instant::now() + Duration::from_secs(1));
                }
                Some(view) => {
                    self.job = None;
                    self.result = Some(view.status);
                    match view.status {
                        LocalMcpProbeStatus::ProgramMissing => app.notice(
                            crate::tr!("Das MCP-Programm fehlt auf dem Runner oder ist nicht ausführbar."),
                        ),
                        LocalMcpProbeStatus::Unavailable => {
                            app.notice(crate::tr!("Der lokale MCP hat nicht erfolgreich geantwortet."))
                        }
                        _ => {}
                    }
                }
                None => {
                    self.job = None;
                    self.result = None;
                    app.notice(crate::tr!("Der lokale MCP-Test konnte nicht abgeschlossen werden."));
                }
            }
        }
        if let (Some(job), Some(next)) = (self.job, self.next) {
            if app.online && self.request.is_none() && next <= Instant::now() {
                self.request = Some(ephemeral(
                    app,
                    "GET",
                    format!("/v1/mcp/local-tests/{job}"),
                    Value::Null,
                ));
            }
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut App,
        entry: &LocalMcp,
        revision: Revision,
        saved: bool,
    ) {
        let active = self.request.is_some() || self.job.is_some();
        if active {
            if self.target == entry.id {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(crate::tr!("Verbindung wird auf dem Runner getestet …"));
                });
            }
            return;
        }
        let runners: Vec<_> = app
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| &snapshot.runners)
            .filter(|runner| {
                !app.runner_removed(runner.id)
                    && entry.enabled_on(runner.id)
                    && runner
                        .capabilities
                        .iter()
                        .any(|c| c == LOCAL_MCP_PROBE_CAPABILITY)
            })
            .map(|runner| (runner.id, runner.name.clone()))
            .collect();
        if self
            .runner
            .is_none_or(|id| !runners.iter().any(|(runner, _)| *runner == id))
        {
            self.runner = runners.first().map(|(id, _)| *id);
        }
        ui.horizontal_wrapped(|ui| {
            if !runners.is_empty() {
                let old = self.runner;
                egui::ComboBox::from_id_salt("local-mcp-probe-runner")
                    .selected_text(
                        runners
                            .iter()
                            .find(|(id, _)| Some(*id) == self.runner)
                            .map_or(crate::tr!("Runner wählen"), |(_, n)| n),
                    )
                    .show_ui(ui, |ui| {
                        for (id, name) in &runners {
                            ui.selectable_value(&mut self.runner, Some(*id), name);
                        }
                    });
                if old != self.runner {
                    self.result = None;
                }
            }
            if ui
                .add_enabled(
                    saved && self.runner.is_some(),
                    egui::Button::new(crate::tr!("Gespeicherte Verbindung testen")),
                )
                .on_disabled_hover_text(crate::tr!("MCP speichern und einen unterstützten Runner zuordnen."))
                .clicked()
            {
                self.request = Some(ephemeral(
                    app,
                    "POST",
                    format!("{CATALOG}/{}/test", entry.id),
                    json!({"runner":self.runner,"expected_revision":revision}),
                ));
                self.target.clone_from(&entry.id);
                self.result = None;
            }
            if self.target == entry.id && self.result == Some(LocalMcpProbeStatus::Connected) {
                ui.label(crate::tr!("Verbunden"));
            }
        });
    }
}

fn entry_fields(ui: &mut egui::Ui, app: &App, entry: &mut LocalMcp) {
    // Clean confirmed removals from this editable draft, without writing settings
    // until the user explicitly saves. Merely offline runners remain assigned.
    entry.runners.retain(|id| !app.runner_removed(*id));
    entry.excluded_runners.retain(|id| !app.runner_removed(*id));
    design::settings_field(ui, "Name", |ui| {
        ui.add(design::singleline(&mut entry.name).char_limit(80));
    });
    design::settings_field(ui, crate::tr!("Absoluter Programmpfad auf dem Runner"), |ui| {
        if let Some(executable) = entry.command.first_mut() {
            ui.add(design::singleline(executable).char_limit(512));
        }
    });
    let mut remove = None;
    for (index, value) in entry.command.iter_mut().enumerate().skip(1) {
        ui.horizontal(|ui| {
            ui.add(
                design::singleline(value)
                    .hint_text(design::input_hint(&format!("Argument {index}")))
                    .char_limit(512),
            );
            if design::icon_button(ui, egui_phosphor::regular::X, crate::tr!("Argument entfernen"), true)
                .clicked()
            {
                remove = Some(index);
            }
        });
        ui.add_space(design::SETTINGS_FIELD_GAP);
    }
    if let Some(index) = remove {
        entry.command.remove(index);
    }
    if ui
        .add_enabled(
            entry.command.len() < 32,
            egui::Button::new(crate::tr!("Argument hinzufügen")),
        )
        .clicked()
    {
        entry.command.push(String::new());
    }
    ui.add_space(design::SETTINGS_FIELD_GAP);
    ui.label(crate::tr!("Zugeordnete Runner"));
    if let Some(snapshot) = &app.snapshot {
        for runner in &snapshot.runners {
            if app.runner_removed(runner.id) {
                continue;
            }
            if runner
                .capabilities
                .iter()
                .any(|c| c == LOCAL_MCP_CAPABILITY)
                || entry.runners.contains(&runner.id)
            {
                runner_membership(ui, entry, runner.id, &runner.name);
            }
        }
        for id in entry.runners.clone() {
            if !snapshot.runners.iter().any(|runner| runner.id == id) {
                runner_membership(ui, entry, id, &format!("Runner {id}"));
            }
        }
    }
}

fn runner_membership(ui: &mut egui::Ui, entry: &mut LocalMcp, id: RunnerId, name: &str) {
    let mut enabled = entry.enabled_on(id);
    if ui.checkbox(&mut enabled, name).changed() {
        set_runner_enabled(entry, id, enabled);
    }
}

fn set_runner_enabled(entry: &mut LocalMcp, id: RunnerId, enabled: bool) {
    let (list, include) = if entry.all_runners {
        (&mut entry.excluded_runners, !enabled)
    } else {
        (&mut entry.runners, enabled)
    };
    list.retain(|runner| *runner != id);
    if include {
        list.push(id);
    }
}

pub(crate) fn project_draft(app: &App, project: ProjectId) -> Option<Value> {
    let editor = app.mcp_settings.projects.get(&project)?;
    let settings = editor.draft.as_ref()?;
    (editor.saved.as_ref() != Some(settings)).then(|| json!(SetProjectLocalMcps {
        request_id: RequestId::new(), settings: settings.clone()
    }))
}

pub(crate) fn discard_project_draft(app: &mut App, project: ProjectId) {
    if let Some(editor) = app.mcp_settings.projects.get_mut(&project) {
        editor.draft.clone_from(&editor.saved);
    }
}

pub(crate) fn confirm_project_save(app: &mut App, project: ProjectId, value: Value) -> bool {
    let Ok(saved) = serde_json::from_value(value) else { return false; };
    app.mcp_settings.projects.entry(project).or_default().saved = Some(saved);
    discard_project_draft(app, project);
    true
}

pub(crate) fn show_project(ui: &mut egui::Ui, app: &mut App, id: ProjectId, bottom: f32) {
    let mut state = std::mem::take(&mut app.mcp_settings);
    let path = format!("/v1/projects/{id}/mcp/local");
    let editor = state.projects.entry(id).or_default();
    editor.refresh(ui, app, &path);
    state.choices.refresh(ui, app, CHOICES);
    let admin = app.access.as_ref().is_none_or(|access| access.role == DeviceRole::Admin);
    let editable = admin && app.online && editor.pending.is_none();
    if let (Some(saved), Some(draft), Some(choices)) = (&editor.saved, &mut editor.draft, &state.choices.saved) {
        let references: Vec<_> = choices.iter().filter_map(Choice::remote_reference).collect();
        if admin {
            state.remote.connections.update(ui, app, &references);
        }
        egui::ScrollArea::vertical()
            .id_salt(("project-local-mcps", id))
            .max_height((bottom - ui.cursor().top()).max(0.0))
            .show(ui, |ui| {
                design::section_heading(ui, crate::tr!("Factory-MCPs freigeben"));
                if !admin {
                    ui.label(crate::tr!("Nur Factory-Admins können Freigaben ändern."));
                }
                if choices.is_empty() {
                    ui.label(crate::tr!("In den Factory-Einstellungen sind noch keine MCPs eingetragen."));
                }
                for choice in choices {
                    let connection = choice.remote_reference().map(|reference| state.remote.connections.status(&reference));
                    project_grant_row(ui, draft, choice, saved.required.contains(&choice.id), editable, connection);
                }
                ui.add_enabled_ui(editable, |ui| {
                    let missing: Vec<_> = draft.required.iter().chain(&draft.optional)
                        .filter(|id| !choices.iter().any(|choice| &choice.id == *id))
                        .cloned().collect();
                    for id in missing {
                        let mut granted = true;
                        if ui.checkbox(&mut granted, crate::tr!("{id} · nicht im Factory-Katalog", id = id)).changed() {
                            draft.required.retain(|entry| entry != &id);
                            draft.optional.retain(|entry| entry != &id);
                        }
                    }
                });
            });
    }
    app.mcp_settings = state;
}

impl Choice {
    fn remote_reference(&self) -> Option<factory_core::RemoteMcpRef> {
        self.remote.then(|| factory_core::RemoteMcpRef {
            id: self.id.clone(),
            name: self.name.clone(),
            revision: self.revision.unwrap_or(Revision(0)),
        })
    }
}

fn project_grant_row(
    ui: &mut egui::Ui,
    draft: &mut ProjectLocalMcps,
    choice: &Choice,
    saved_required: bool,
    editable: bool,
    connection: Option<ConnectionStatus>,
) {
    ui.push_id(&choice.id, |ui| {
        let was_granted = draft.required.contains(&choice.id) || draft.optional.contains(&choice.id);
        let mut granted = was_granted;
        let has_room = was_granted || draft.required.len() + draft.optional.len() < 8;
        let grant = ui.horizontal(|ui| {
            ui.set_min_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(connection) = connection {
                    connection.label(ui);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let label = format!("{} · {}", choice.name, if choice.remote { "Remote" } else { crate::tr!("Lokal") });
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    let grant = ui.add_enabled(editable && has_room, egui::Checkbox::new(&mut granted, &label));
                    if !has_room {
                        grant.on_disabled_hover_text(crate::tr!("Maximal acht MCPs pro Projekt."))
                    } else {
                        grant.on_hover_text(&label)
                    }
                }).inner
            }).inner
        }).inner;
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-mcp-grant", choice.id.as_str())), grant.rect));
        if grant.changed() {
            // Rechecking before a save restores the original grant; new grants are optional.
            draft.required.retain(|id| id != &choice.id);
            draft.optional.retain(|id| id != &choice.id);
            if granted {
                let list = if saved_required { &mut draft.required } else { &mut draft.optional };
                list.push(choice.id.clone());
                list.sort_unstable();
            }
        }
    });
}

pub fn task_choices(
    ui: &mut egui::Ui,
    app: &mut App,
    project: ProjectId,
    selected: &mut Option<Vec<String>>,
) -> bool {
    let mut state = std::mem::take(&mut app.mcp_settings);
    let editor = state.projects.entry(project).or_default();
    editor.refresh(ui, app, &format!("/v1/projects/{project}/mcp/local"));
    state.choices.refresh(ui, app, CHOICES);
    let mut changed = false;
    if let (Some(grants), Some(choices)) = (&editor.saved, &state.choices.saved) {
        let mut selection = selected.clone().unwrap_or_else(|| grants.optional.clone());
        let before = selection.clone();
        if grants.optional.is_empty() && selection.is_empty() && grants.required.is_empty() {
            ui.label(crate::tr!("Für dieses Projekt sind keine MCPs freigegeben."));
        }
        for choice in choices {
            if grants.required.contains(&choice.id) {
                let mut enabled = true;
                let response = ui.add_enabled(false, egui::Checkbox::new(&mut enabled, crate::tr!("{} · erforderlich", choice.name)))
                    .on_disabled_hover_text(crate::tr!("Vom Projekt vorgeschrieben"));
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-mcp-choice", &choice.id)), (response.rect, true, false)));
                #[cfg(not(test))]
                let _ = response;
            } else if grants.optional.contains(&choice.id) {
                let mut enabled = selection.contains(&choice.id);
                let response = ui.checkbox(&mut enabled, &choice.name);
                if response.changed() {
                    selection.retain(|id| id != &choice.id);
                    if enabled { selection.push(choice.id.clone()); selection.sort_unstable(); }
                }
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test-task-mcp-choice", &choice.id)), (response.rect, enabled, true)));
            }
        }
        for stale in selection.clone() {
            if !grants.optional.contains(&stale) {
                membership(
                    ui,
                    &mut selection,
                    stale.clone(),
                    &crate::tr!("{stale} · nicht mehr freigegeben", stale = stale),
                );
            }
        }
        if selection != before {
            *selected = Some(selection);
            changed = true;
        }
    } else {
        ui.spinner();
        ui.label(crate::tr!("MCPs werden geladen …"));
    }
    app.mcp_settings = state;
    changed
}

fn membership<T: Ord>(ui: &mut egui::Ui, values: &mut Vec<T>, value: T, label: &str) {
    let mut selected = values.contains(&value);
    if ui.checkbox(&mut selected, label).changed() {
        if selected {
            values.push(value);
            values.sort_unstable();
        } else {
            values.retain(|entry| entry != &value);
        }
    }
}

mod oauth_loopback;

const REMOTE: &str = "/v1/mcp/remote";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionStatus {
    Unchecked,
    Connected,
    Disconnected,
}

impl ConnectionStatus {
    fn label(self, ui: &mut egui::Ui) {
        let (text, color) = match self {
            Self::Connected => (crate::tr!("Verbunden"), crate::app::GREEN),
            Self::Disconnected => (crate::tr!("Nicht verbunden"), crate::app::RED),
            Self::Unchecked => (crate::tr!("Nicht geprüft"), crate::app::MUTED),
        };
        ui.colored_label(color, text);
    }
}

#[derive(Default)]
struct RemoteConnections {
    results: BTreeMap<String, (Revision, ConnectionStatus, Instant)>,
    pending: BTreeMap<u64, factory_core::RemoteMcpRef>,
}

impl RemoteConnections {
    const REFRESH: Duration = Duration::from_secs(60);

    fn status(&self, entry: &factory_core::RemoteMcpRef) -> ConnectionStatus {
        self.results
            .get(&entry.id)
            .filter(|(revision, _, _)| *revision == entry.revision)
            .map_or(ConnectionStatus::Unchecked, |(_, status, _)| *status)
    }

    fn record(
        &mut self,
        entry: &factory_core::RemoteMcpRef,
        status: ConnectionStatus,
        now: Instant,
    ) {
        self.results
            .insert(entry.id.clone(), (entry.revision, status, now));
    }

    fn update(&mut self, ui: &egui::Ui, app: &mut App, entries: &[factory_core::RemoteMcpRef]) {
        let now = Instant::now();
        self.results.retain(|id, (revision, _, _)| {
            entries
                .iter()
                .any(|entry| entry.id == *id && entry.revision == *revision)
        });
        let finished = self
            .pending
            .keys()
            .copied()
            .filter(|request| app.responses.contains_key(request))
            .collect::<Vec<_>>();
        for request in finished {
            let entry = self.pending.remove(&request).expect("pending request");
            let result = app.responses.remove(&request).expect("completed request");
            if entries.iter().any(|saved| {
                saved.id == entry.id && saved.revision == entry.revision
            }) {
                let status = match result {
                    Ok(value) if value["status"] == "connected" => ConnectionStatus::Connected,
                    Ok(_) => ConnectionStatus::Disconnected,
                    // Don't turn a cached result red just because the Factory is unreachable.
                    Err(_) => self.status(&entry),
                };
                self.record(&entry, status, now);
            }
        }
        if !app.online {
            return;
        }
        for reference in entries {
            if self.pending.len() >= 3 {
                break;
            }
            if self.pending.values().any(|entry| entry.id == reference.id) {
                continue;
            }
            if let Some((_, _, checked)) = self.results.get(&reference.id) {
                let remaining = Self::REFRESH.saturating_sub(now.duration_since(*checked));
                if !remaining.is_zero() {
                    ui.ctx().request_repaint_after(remaining);
                    continue;
                }
            }
            let request = ephemeral(
                app,
                "POST",
                format!("{REMOTE}/{}/test", reference.id),
                Value::Null,
            );
            self.pending.insert(request, reference.clone());
        }
    }
}

#[derive(Default)]
struct RemoteEditor {
    entries: Editor<Vec<RemoteMcp>>,
    connections: RemoteConnections,
    draft: Option<RemoteDraft>,
    save: Option<u64>,
    test: Option<u64>,
    status: Option<bool>,
    verified_attempt: Option<String>,
    oauth_start: Option<u64>,
    oauth_listener: Option<std::net::TcpListener>,
    oauth_wait: Option<OAuthWait>,
    discovery: Option<(u64, String, String)>,
}

struct OAuthWait {
    attempt: String,
    until: Instant,
    poll_at: Instant,
    request: Option<u64>,
    loopback: Option<tokio::sync::oneshot::Receiver<oauth_loopback::Callback>>,
}

struct RemoteDraft {
    id: String,
    revision: factory_core::Revision,
    name: String,
    url: String,
    auth: RemoteMcpAuth,
    has_key: bool,
    key: String,
    auth_view: AuthView,
    manual_chosen: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthView {
    Unchecked,
    Open,
    OAuth,
    Manual,
    Saved,
}

impl RemoteDraft {
    fn body(&self) -> SaveRemoteMcp {
        SaveRemoteMcp {
            expected_revision: self.revision,
            name: self.name.clone(),
            url: self.url.clone(),
            auth: self.auth.clone(),
            key: (!self.key.is_empty() && matches!(self.auth, RemoteMcpAuth::Header { .. }))
                .then(|| self.key.clone()),
            remove_key: false,
        }
    }

    fn new() -> Self {
        Self {
            id: new_mcp_id(),
            revision: factory_core::Revision(0),
            name: String::new(),
            url: String::new(),
            auth: RemoteMcpAuth::None,
            has_key: false,
            key: String::new(),
            auth_view: AuthView::Unchecked,
            manual_chosen: false,
        }
    }

    fn from_entry(entry: &RemoteMcp) -> Self {
        Self {
            id: entry.reference.id.clone(),
            revision: entry.reference.revision,
            name: entry.reference.name.clone(),
            url: entry.url.clone(),
            auth: entry.auth.clone(),
            has_key: entry.has_key,
            auth_view: AuthView::Saved,
            ..Self::new()
        }
    }

    fn valid(&self) -> bool {
        !self.name.trim().is_empty()
            && self.url.starts_with("https://")
            && self.auth_view != AuthView::Unchecked
            && (self.auth_view != AuthView::Manual || self.manual_chosen)
            && match &self.auth {
                RemoteMcpAuth::None => true,
                RemoteMcpAuth::Header { name, .. } => !name.trim().is_empty(),
                RemoteMcpAuth::OAuth {
                    client_id,
                    authorization_url,
                    token_url,
                    redirect_uri,
                    ..
                } => {
                    !client_id.trim().is_empty()
                        && [authorization_url, token_url]
                            .iter()
                            .all(|url| url.starts_with("https://"))
                        && (redirect_uri == MCP_OAUTH_LOOPBACK_REDIRECT
                            || redirect_uri.starts_with("https://"))
                }
            }
    }
}

impl RemoteEditor {
    fn update(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        let mut changed = false;
        let mut discovered_oauth = false;
        if let Some((request, id, url)) = self.discovery.clone() {
            if let Some(result) = app.responses.remove(&request) {
                self.discovery = None;
                if let Some(draft) = self
                    .draft
                    .as_mut()
                    .filter(|draft| draft.id == id && draft.url == url)
                {
                    match result.ok().and_then(|value| {
                        serde_json::from_value::<RemoteMcpAuthDiscovery>(value).ok()
                    }) {
                        Some(RemoteMcpAuthDiscovery::Open) => {
                            draft.auth = RemoteMcpAuth::None;
                            draft.auth_view = AuthView::Open;
                            self.status = Some(true);
                        }
                        Some(RemoteMcpAuthDiscovery::OAuth { mut auth }) => {
                            if let (
                                RemoteMcpAuth::OAuth {
                                    client_id,
                                    redirect_uri,
                                    authorization_url,
                                    token_url,
                                    ..
                                },
                                RemoteMcpAuth::OAuth {
                                    client_id: prior_id,
                                    redirect_uri: prior_redirect,
                                    authorization_url: prior_auth,
                                    token_url: prior_token,
                                    ..
                                },
                            ) = (&mut auth, &draft.auth)
                            {
                                if authorization_url == prior_auth && token_url == prior_token {
                                    if client_id.is_empty()
                                        || (draft.revision.0 > 0
                                            && redirect_uri == prior_redirect
                                            && !prior_id.is_empty())
                                    {
                                        client_id.clone_from(prior_id);
                                    }
                                    if redirect_uri.is_empty() {
                                        redirect_uri.clone_from(prior_redirect);
                                    }
                                }
                            }
                            draft.auth = auth;
                            draft.auth_view = AuthView::OAuth;
                            discovered_oauth = true;
                        }
                        Some(RemoteMcpAuthDiscovery::Unknown) => {
                            draft.auth_view = AuthView::Manual;
                            draft.manual_chosen = false;
                            self.status = Some(false);
                        }
                        _ => self.status = Some(false),
                    }
                }
            }
        }
        if discovered_oauth && app.online {
            self.start_oauth(app);
        }
        if let Some(result) = self.oauth_start.and_then(|id| app.responses.remove(&id)) {
            self.oauth_start = None;
            let listener = self.oauth_listener.take();
            if result.is_err() {
                eprintln!(
                    "MCP OAuth: Factory did not accept or answer the browser preparation request"
                );
            }
            if let Some((url, attempt)) = result
                .ok()
                .and_then(|v| {
                    Some((
                        v["url"].as_str()?.to_owned(),
                        v["attempt"].as_str()?.to_owned(),
                    ))
                })
                .filter(|(url, attempt)| url.starts_with("https://") && attempt.len() == 43)
            {
                let loopback =
                    listener.map(|listener| oauth_loopback::receive(listener, attempt.clone()));
                let loopback = match loopback.transpose() {
                    Ok(loopback) => loopback,
                    Err(_) => {
                        eprintln!("MCP OAuth: local callback listener could not start");
                        self.status = Some(false);
                        return changed;
                    }
                };
                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                let now = Instant::now();
                self.oauth_wait = Some(OAuthWait {
                    attempt,
                    until: now + Duration::from_secs(600),
                    poll_at: now,
                    request: None,
                    loopback,
                });
            } else {
                self.status = Some(false);
            }
        }
        if let Some(wait) = &mut self.oauth_wait {
            let now = Instant::now();
            if let Some(result) = wait.request.and_then(|id| app.responses.remove(&id)) {
                wait.request = None;
                if let Ok(value) = result {
                    match value["status"].as_str() {
                        Some("verified") => {
                            self.verified_attempt = Some(wait.attempt.clone());
                            self.status = Some(true);
                            self.oauth_wait = None;
                        }
                        Some("connected") => {
                            self.oauth_wait = None;
                            let new = serde_json::from_value::<RemoteMcp>(value["entry"].clone())
                                .ok()
                                .filter(|e| {
                                    e.has_key
                                        && self
                                            .draft
                                            .as_ref()
                                            .is_some_and(|d| d.id == e.reference.id)
                                });
                            if let Some(new) = new {
                                if let Some(entry) =
                                    self.entries.saved.as_mut().and_then(|entries| {
                                        entries
                                            .iter_mut()
                                            .find(|e| e.reference.id == new.reference.id)
                                    })
                                {
                                    *entry = new.clone();
                                }
                                self.draft = Some(RemoteDraft::from_entry(&new));
                                app.notice(crate::tr!("OAuth-Anmeldung gespeichert. Verbindung kann jetzt getestet werden."));
                            } else {
                                app.notice(crate::tr!("OAuth-Anmeldung abgeschlossen, aber der MCP-Eintrag wurde geändert."));
                            }
                        }
                        Some("failed") => {
                            self.oauth_wait = None;
                            self.status = Some(false);
                        }
                        _ => {}
                    }
                }
            }
            if let Some(wait) = &mut self.oauth_wait {
                if now >= wait.until {
                    self.oauth_wait = None;
                    self.status = Some(false);
                } else {
                    if app.online && wait.request.is_none() {
                        if let Some(callback) = wait
                            .loopback
                            .as_mut()
                            .and_then(|receiver| receiver.try_recv().ok())
                        {
                            wait.loopback = None;
                            if let Some(draft) = &self.draft {
                                wait.request = Some(ephemeral(
                                    app,
                                    "POST",
                                    format!("{REMOTE}/{}/oauth/complete", draft.id),
                                    json!(callback),
                                ));
                                wait.poll_at = now;
                            }
                        }
                    }
                    if app.online && wait.request.is_none() && wait.poll_at <= now {
                        if let Some(draft) = &self.draft {
                            wait.request = Some(ephemeral(
                                app,
                                "POST",
                                format!("{REMOTE}/{}/oauth/status", draft.id),
                                json!({"attempt":wait.attempt}),
                            ));
                            wait.poll_at = now + Duration::from_secs(2);
                        }
                    }
                    ui.ctx().request_repaint_after(Duration::from_secs(2));
                }
            }
        }
        if let Some(value) = self.save.and_then(|id| app.responses.remove(&id)) {
            self.save = None;
            match value
                .ok()
                .and_then(|v| serde_json::from_value::<RemoteMcp>(v).ok())
            {
                Some(entry) => {
                    self.connections.record(
                        &entry.reference,
                        ConnectionStatus::Connected,
                        Instant::now(),
                    );
                    let items = self.entries.saved.get_or_insert_with(Vec::new);
                    items.retain(|other| other.reference.id != entry.reference.id);
                    items.push(entry.clone());
                    items.sort_by(|a, b| a.reference.id.cmp(&b.reference.id));
                    self.draft = Some(RemoteDraft::from_entry(&entry));
                    self.status = None;
                    app.notice(crate::tr!("Remote-MCP gespeichert."));
                    self.verified_attempt = None;
                    changed = true;
                }
                None => {
                    if self.verified_attempt.take().is_some() {
                        self.status = Some(false);
                    }
                    app.notice(crate::tr!("Remote-MCP konnte nicht gespeichert werden. Bei unklarer Zustellung zuerst neu laden."));
                }
            }
        }
        if let Some(value) = self.test.and_then(|id| app.responses.remove(&id)) {
            self.test = None;
            self.status = Some(value.is_ok_and(|v| v["status"] == "connected"));
        }
        self.entries.refresh(ui, app, REMOTE);
        changed
    }

    fn busy(&self) -> bool {
        self.save.is_some()
            || self.test.is_some()
            || self.oauth_start.is_some()
            || self.oauth_wait.is_some()
            || self.discovery.is_some()
    }

    fn start_oauth(&mut self, app: &mut App) {
        let Some(draft) = self.draft.as_ref().filter(|draft| draft.valid()) else {
            eprintln!("MCP OAuth: discovery did not provide complete client settings");
            self.status = Some(false);
            return;
        };
        if uses_loopback(&draft.auth) {
            match oauth_loopback::bind() {
                Ok(listener) => self.oauth_listener = Some(listener),
                Err(error) => {
                    eprintln!(
                        "MCP OAuth: local callback port could not be bound ({:?})",
                        error.kind()
                    );
                    self.status = Some(false);
                    return;
                }
            }
        }
        self.oauth_start = Some(ephemeral(
            app,
            "POST",
            format!("{REMOTE}/{}/oauth/prepare", draft.id),
            json!(draft.body()),
        ));
        self.status = None;
    }

    fn fields(&mut self, ui: &mut egui::Ui, app: &mut App) {
        let editable = app.online && !self.busy();
        ui.scope(|ui| {
            if let Some(draft) = &mut self.draft {
                design::settings_field(ui, "Name", |ui| {
                    ui.add_enabled(editable, design::singleline(&mut draft.name).char_limit(80));
                });
                design::settings_field(ui, crate::tr!("HTTPS-Endpunkt"), |ui| {
                    if ui
                        .add_enabled(
                            editable,
                            design::singleline(&mut draft.url)
                                .hint_text(design::input_hint("https://example.org/mcp"))
                                .char_limit(1024),
                        )
                        .changed()
                    {
                        draft.auth_view = AuthView::Unchecked;
                        draft.auth = RemoteMcpAuth::None;
                        draft.key.clear();
                        self.status = None;
                        self.verified_attempt = None;
                    }
                });
                if ui
                    .add_enabled(
                        editable
                            && draft.url.starts_with("https://")
                            && !draft.name.trim().is_empty(),
                        egui::Button::new(crate::tr!("Authentifizierung prüfen")),
                    )
                    .clicked()
                {
                    if (draft.auth_view == AuthView::Manual && draft.manual_chosen)
                        || (draft.auth_view == AuthView::Saved
                            && matches!(draft.auth, RemoteMcpAuth::Header { .. }))
                    {
                        self.test = Some(ephemeral(
                            app,
                            "POST",
                            format!("{REMOTE}/{}/verify", draft.id),
                            json!(draft.body()),
                        ));
                    } else {
                        let url = draft.url.clone();
                        let request = ephemeral(
                            app,
                            "POST",
                            format!("{REMOTE}/discover"),
                            json!(ProbeRemoteMcpAuth { url: url.clone() }),
                        );
                        self.discovery = Some((request, draft.id.clone(), url));
                    }
                    self.status = None;
                    self.verified_attempt = None;
                }
                if let Some(success) = self.status {
                    let (icon, color, label) = if success {
                        (
                            egui_phosphor::regular::CHECK,
                            crate::app::GREEN,
                            crate::tr!("Verbindung erfolgreich"),
                        )
                    } else {
                        (
                            egui_phosphor::regular::X,
                            crate::app::RED,
                            crate::tr!("Verbindung fehlgeschlagen"),
                        )
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(color, icon);
                        ui.colored_label(color, label);
                    });
                } else if self.oauth_wait.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(crate::tr!("Anmeldung im Browser abschließen …"));
                        if ui.button(crate::tr!("Warten beenden")).clicked() {
                            self.oauth_wait = None;
                            self.status = Some(false);
                        }
                    });
                } else if self.discovery.is_some()
                    || self.oauth_start.is_some()
                    || self.test.is_some()
                {
                    ui.spinner();
                }
                ui.add_space(design::SETTINGS_FIELD_GAP);
                if !editable
                    || draft.auth_view == AuthView::Unchecked
                    || matches!(draft.auth, RemoteMcpAuth::OAuth { .. })
                        && draft.auth_view != AuthView::Manual
                {
                    return;
                }
                let old = match draft.auth {
                    RemoteMcpAuth::None => 0,
                    RemoteMcpAuth::Header { bearer: true, .. } => 1,
                    RemoteMcpAuth::Header { bearer: false, .. } => 2,
                    RemoteMcpAuth::OAuth { .. } => 3,
                };
                let mut selected = if draft.auth_view == AuthView::Manual && !draft.manual_chosen {
                    4
                } else {
                    old
                };
                let options = [(0, crate::tr!("Keine")), (1, crate::tr!("Bearer-Token")), (2, crate::tr!("API-Key-Header"))];
                if draft.auth_view == AuthView::Manual {
                    design::settings_field(ui, crate::tr!("Authentifizierung"), |ui| {
                        let mut items = options.to_vec();
                        if selected == 4 {
                            items.insert(0, (4, crate::tr!("Bitte auswählen")));
                        }
                        dropdown(ui, "mcp-auth", &mut selected, &items);
                    });
                }
                if selected == 4 {
                    return;
                }
                if draft.auth_view == AuthView::Manual {
                    draft.manual_chosen = true;
                }
                if old != selected {
                    draft.auth = match selected {
                        1 => RemoteMcpAuth::Header {
                            name: "authorization".into(),
                            bearer: true,
                        },
                        2 => RemoteMcpAuth::Header {
                            name: "X-API-Key".into(),
                            bearer: false,
                        },
                        _ => RemoteMcpAuth::None,
                    };
                    draft.key.clear();
                    self.status = None;
                }
                if let RemoteMcpAuth::Header { name, bearer } = &mut draft.auth {
                    if !*bearer {
                        design::settings_field(ui, crate::tr!("Header-Name"), |ui| {
                            if ui.add(design::singleline(name).char_limit(80)).changed() {
                                self.status = None;
                            }
                        });
                    }
                    let label = if draft.has_key {
                        crate::tr!("Schlüssel hinterlegt · neuen Wert zum Ersetzen eingeben")
                    } else {
                        crate::tr!("Schlüssel")
                    };
                    design::settings_field(ui, label, |ui| {
                        if ui
                            .add(
                                design::singleline(&mut draft.key)
                                    .password(true)
                                    .char_limit(4096),
                            )
                            .changed()
                        {
                            self.status = None;
                        }
                    });
                }
            }
        });
    }

    fn submit(&mut self, app: &mut App) {
        let Some(draft) = self
            .draft
            .as_ref()
            .filter(|draft| draft.valid() && self.status == Some(true))
        else {
            return;
        };
        let (method, path, body) = if matches!(draft.auth, RemoteMcpAuth::OAuth { .. }) {
            let Some(attempt) = &self.verified_attempt else {
                return;
            };
            (
                "POST",
                format!("{REMOTE}/{}/oauth/commit", draft.id),
                json!({"attempt":attempt,"name":draft.name}),
            )
        } else {
            ("PUT", format!("{REMOTE}/{}", draft.id), json!(draft.body()))
        };
        self.save = Some(ephemeral(app, method, path, body));
    }
}

fn uses_loopback(auth: &RemoteMcpAuth) -> bool {
    matches!(auth, RemoteMcpAuth::OAuth { redirect_uri, .. } if redirect_uri == MCP_OAUTH_LOOPBACK_REDIRECT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_mcp_keeps_existing_required_grants_and_adds_new_ones_as_optional() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let project = app.project().unwrap().id;
        let id = "calendar";
        let saved = ProjectLocalMcps { revision: Revision(1), optional: vec![], required: vec![id.into()] };
        let editor = app.mcp_settings.projects.entry(project).or_default();
        editor.saved = Some(saved.clone());
        editor.draft = Some(saved);
        let new_id = "notes";
        app.mcp_settings.choices.saved = Some(vec![
            Choice { id: id.into(), name: "Kalender".into(), remote: true, revision: Some(Revision(2)) },
            Choice { id: new_id.into(), name: "Notizen".into(), remote: false, revision: None },
        ]);
        let remote = app.mcp_settings.choices.saved.as_ref().unwrap()[0].remote_reference().unwrap();
        app.mcp_settings.remote.connections.record(&remote, ConnectionStatus::Connected, Instant::now());
        let ctx = app.backend.context().clone();
        let mut settings = crate::project_settings::ProjectSettings::new(project);
        settings.selected = crate::project_settings::ProjectSection::Mcps;
        let mut draw = |app: &mut App, events| {
            ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| settings.show(ui, app, 580.0));
            })
        };
        let click = |id| {
            let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(id)).unwrap();
            crate::tests::click_events(rect.center())
        };
        let grant = egui::Id::new(("test-mcp-grant", id));
        let output = draw(&mut app, vec![]);
        assert!(output.shapes.iter().any(|s| matches!(
            &s.shape, egui::epaint::Shape::Text(text) if text.galley.job.text == "Verbunden"
        )));
        assert_eq!(app.mcp_settings.projects[&project].draft.as_ref().unwrap().required, [id]);
        assert!(app.mcp_settings.projects[&project].pending.is_none());
        draw(&mut app, click(grant));
        assert!(app.mcp_settings.projects[&project].draft.as_ref().unwrap().required.is_empty());
        draw(&mut app, click(grant));
        assert_eq!(app.mcp_settings.projects[&project].draft.as_ref().unwrap().required, [id]);
        draw(&mut app, click(egui::Id::new(("test-mcp-grant", new_id))));
        assert_eq!(app.mcp_settings.projects[&project].draft.as_ref().unwrap().optional, [new_id]);
        draw(&mut app, click(egui::Id::new("test-settings-save")));
        assert_eq!(app.settings.pending[0].body["settings"]["optional"], json!([new_id]));
    }

    #[test]
    fn remote_list_status_uses_probe_result_for_the_current_revision() {
        let mut app = crate::tests::fixture();
        app.online = false;
        let ctx = app.backend.context().clone();
        let mut entry = RemoteMcp {
            reference: factory_core::RemoteMcpRef {
                id: "example".into(),
                name: "Example".into(),
                revision: Revision(1),
            },
            url: "https://example.org/mcp".into(),
            auth: RemoteMcpAuth::None,
            has_key: false,
        };
        let mut connections = RemoteConnections::default();
        let mut probe = |connections: &mut RemoteConnections, entry: &RemoteMcp, sent, status| {
            connections.pending.insert(42, sent);
            app.responses.insert(42, Ok(json!({"status":status})));
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    connections.update(ui, &mut app, std::slice::from_ref(&entry.reference));
                });
            });
        };
        let old = entry.reference.clone();
        probe(&mut connections, &entry, old.clone(), "connected");
        assert_eq!(
            connections.status(&entry.reference),
            ConnectionStatus::Connected
        );
        entry.reference.revision = Revision(2);
        probe(&mut connections, &entry, old, "connected");
        assert_eq!(
            connections.status(&entry.reference),
            ConnectionStatus::Unchecked
        );
        probe(
            &mut connections,
            &entry,
            entry.reference.clone(),
            "needs_auth",
        );
        assert_eq!(
            connections.status(&entry.reference),
            ConnectionStatus::Disconnected
        );
    }

    #[test]
    fn mcp_dialog_changes_height_with_auth_fields_and_generates_stable_draft_ids() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let ctx = app.backend.context().clone();
        let mut settings = Settings {
            modal: Some(CatalogDialog::Add),
            add_remote: true,
            ..Default::default()
        };
        settings.remote.draft = Some(RemoteDraft {
            auth_view: AuthView::Saved,
            ..RemoteDraft::new()
        });
        let height = |settings: &mut Settings, app: &mut App| {
            for _ in 0..2 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(960.0, 850.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| settings.show_modal(ctx, app),
                );
            }
            ctx.data_mut(|data| {
                data.get_temp::<egui::Rect>(egui::Id::new((
                    "test-settings-modal",
                    "mcp-catalog-dialog",
                )))
                .unwrap()
            })
            .height()
        };
        let compact = height(&mut settings, &mut app);
        let draft = settings.remote.draft.as_mut().unwrap();
        let id = draft.id.clone();
        assert!(factory_core::valid_repo_skill_id(&id));
        assert!(draft.name.is_empty());
        draft.auth = RemoteMcpAuth::Header {
            name: "X-API-Key".into(),
            bearer: false,
        };
        let expanded = height(&mut settings, &mut app);
        assert!(expanded > compact + 40.0, "{compact} → {expanded}");
        settings.remote.draft.as_mut().unwrap().auth = RemoteMcpAuth::None;
        let shrunk = height(&mut settings, &mut app);
        assert!(
            (shrunk - compact).abs() < 2.0,
            "{compact} → {expanded} → {shrunk}"
        );
        assert_eq!(settings.remote.draft.as_ref().unwrap().id, id);
        settings.add_remote = false;
        height(&mut settings, &mut app);
        let local = settings.new_local.unwrap();
        assert!(factory_core::valid_repo_skill_id(&local.id));
        assert_ne!(local.id, id);
        assert!(local.name.is_empty() && local.command == [""]);
    }

    #[test]
    fn discovery_only_enables_manual_selection_when_auth_is_unknown() {
        let mut app = crate::tests::fixture();
        let mut editor = RemoteEditor::default();
        let draft = RemoteDraft {
            name: "Example".into(),
            url: "https://example.org/mcp".into(),
            ..RemoteDraft::new()
        };
        let ctx = app.backend.context().clone();
        let id = draft.id.clone();
        let url = draft.url.clone();
        editor.draft = Some(draft);
        let mut apply = |editor: &mut RemoteEditor, response, probe_url: &str| {
            editor.discovery = Some((7, id.clone(), probe_url.to_owned()));
            app.responses
                .insert(7, Ok(serde_json::to_value(response).unwrap()));
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| editor.update(ui, &mut app));
            });
        };
        apply(&mut editor, RemoteMcpAuthDiscovery::Unavailable, &url);
        assert!(editor.draft.as_ref().unwrap().auth_view == AuthView::Unchecked);
        apply(&mut editor, RemoteMcpAuthDiscovery::Unknown, &url);
        let draft = editor.draft.as_ref().unwrap();
        assert!(draft.auth_view == AuthView::Manual);
        assert!(!draft.valid());
        apply(
            &mut editor,
            RemoteMcpAuthDiscovery::Open,
            "https://example.net/mcp",
        );
        assert!(editor.draft.as_ref().unwrap().auth_view == AuthView::Manual);
        apply(&mut editor, RemoteMcpAuthDiscovery::Open, &url);
        let draft = editor.draft.as_ref().unwrap();
        assert!(draft.auth_view == AuthView::Open && draft.valid());
        apply(
            &mut editor,
            RemoteMcpAuthDiscovery::OAuth {
                auth: RemoteMcpAuth::OAuth {
                    client_id: "https://factory.example/mcp-oauth/client.json".into(),
                    authorization_url: "https://login.example/authorize".into(),
                    token_url: "https://login.example/token".into(),
                    redirect_uri: "https://factory.example/mcp-oauth/callback".into(),
                    scope: String::new(),
                },
            },
            &url,
        );
        let draft = editor.draft.as_ref().unwrap();
        assert!(draft.auth_view == AuthView::OAuth && draft.valid());
    }

    #[test]
    fn discovered_oauth_can_only_be_saved_after_verification() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let draft = RemoteDraft {
            name: "Example".into(),
            url: "https://example.org/mcp".into(),
            ..RemoteDraft::new()
        };
        let entry = RemoteMcp {
            reference: factory_core::RemoteMcpRef {
                id: draft.id.clone(),
                name: "Example".into(),
                revision: Revision(1),
            },
            url: "https://example.org/mcp".into(),
            auth: RemoteMcpAuth::OAuth {
                client_id: "registered-id".into(),
                authorization_url: "https://login.example/authorize".into(),
                token_url: "https://login.example/token".into(),
                redirect_uri: "https://factory.example/mcp-oauth/callback".into(),
                scope: String::new(),
            },
            has_key: false,
        };
        let mut editor = RemoteEditor {
            discovery: Some((42, draft.id.clone(), draft.url.clone())),
            draft: Some(draft),
            ..Default::default()
        };
        let ctx = app.backend.context().clone();
        app.responses.insert(
            42,
            Ok(json!(RemoteMcpAuthDiscovery::OAuth {
                auth: entry.auth.clone()
            })),
        );
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!editor.update(ui, &mut app));
            });
        });
        let start = editor.oauth_start.unwrap();
        editor.submit(&mut app);
        assert!(editor.save.is_none());
        app.responses.insert(
            start,
            Ok(json!({
                "url": "https://login.example/authorize",
                "attempt": "a".repeat(43),
            })),
        );
        let frame = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| editor.update(ui, &mut app));
        });
        assert!(frame
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))));
        let request = editor.oauth_wait.as_ref().unwrap().request.unwrap();
        app.responses
            .insert(request, Ok(json!({"status":"verified"})));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!editor.update(ui, &mut app));
            });
        });
        assert_eq!(editor.status, Some(true));
        assert!(editor.save.is_none());
        assert_eq!(editor.draft.as_ref().unwrap().revision, Revision(0));
        editor.submit(&mut app);
        assert!(editor.save.is_some());
    }

    #[test]
    fn oauth_opens_browser_once_without_putting_state_into_desktop_outbox() {
        let mut app = crate::tests::fixture();
        app.online = false;
        app.responses.insert(
            42,
            Ok(json!({"url":"https://login.example.invalid/authorize?state=fixture","attempt":"a".repeat(43)})),
        );
        let mut editor = RemoteEditor {
            oauth_start: Some(42),
            ..Default::default()
        };
        editor.entries.saved = Some(vec![]);
        let ctx = app.backend.context().clone();
        let mut frame = || {
            ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| editor.update(ui, &mut app));
            })
        };
        assert!(frame().platform_output.commands.iter().any(|command|
            matches!(command, egui::OutputCommand::OpenUrl(url) if url.url.starts_with("https://login.example.invalid/"))));
        assert!(!frame()
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))));
        assert!(editor.oauth_wait.is_some());
        assert!(app.settings.pending.is_empty());
    }

    #[test]
    fn task_mcp_menu_is_in_the_footer_and_defaults_to_all_project_grants() {
        let mut app = crate::tests::fixture();
        let project = app.settings.project.unwrap();
        app.mcp_settings.projects.entry(project).or_default().saved = Some(ProjectLocalMcps {
            revision: Revision(0), required: vec!["required".into()], optional: vec!["one".into(), "two".into()],
        });
        app.mcp_settings.choices.saved = Some(["required", "one", "two", "not-granted"].into_iter()
            .map(|id| Choice { id: id.into(), name: format!("Choice {id}"), remote: id == "two", revision: None }).collect());
        let mut dialog = crate::dialogs::Dialog::task(app.server.clone(), project, None);
        if let crate::dialogs::Dialog::Task(task) = &mut dialog { task.advanced = true; }
        let crate::dialogs::Dialog::Task(task) = &dialog else { unreachable!() };
        let id = task.id;
        let ctx = app.backend.context().clone();
        let render = |dialog: &mut crate::dialogs::Dialog, app: &mut App, events| ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 720.0))),
            events, ..Default::default()
        }, |ctx| { assert!(!dialog.show(ctx, app)); });
        for _ in 0..3 { let _ = render(&mut dialog, &mut app, vec![]); }
        let button: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new(("test-task-mcps", id)))).unwrap();
        let (body, _): (egui::Rect, egui::Vec2) = ctx.data_mut(|d|
            d.get_temp(egui::Id::new(("factory-task", id)).with("test-body-scroll"))).unwrap();
        assert!(button.top() >= body.bottom());
        let choice = |name| ctx.data_mut(|d| d.get_temp::<(egui::Rect, bool, bool)>(
            egui::Id::new(("test-task-mcp-choice", name))));
        assert!(choice("one").is_none(), "Choices must remain hidden until the footer button is opened");
        let click = |dialog: &mut crate::dialogs::Dialog, app: &mut App, pos| {
            let _ = render(dialog, app, vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ]);
            render(dialog, app, vec![])
        };
        let _ = click(&mut dialog, &mut app, button.center());
        assert_eq!(choice("one").map(|(_, on, editable)| (on, editable)), Some((true, true)));
        assert_eq!(choice("two").map(|(_, on, editable)| (on, editable)), Some((true, true)));
        assert_eq!(choice("required").map(|(_, on, editable)| (on, editable)), Some((true, false)));
        assert!(choice("not-granted").is_none());
        let _ = click(&mut dialog, &mut app, choice("one").unwrap().0.center());
        let crate::dialogs::Dialog::Task(task) = &dialog else { unreachable!() };
        assert_eq!(task.draft.local_mcps, Some(vec!["two".into()]));
        // Multiple checkbox changes are possible without reopening the menu.
        let _ = click(&mut dialog, &mut app, choice("two").unwrap().0.center());
        let crate::dialogs::Dialog::Task(task) = &dialog else { unreachable!() };
        assert_eq!(task.draft.local_mcps, Some(vec![]));
        assert_eq!(app.settings.task_drafts.last().unwrap().local_mcps, Some(vec![]),
            "Explicitly disabling every optional MCP must survive closing the empty draft");
        assert!(app.settings.pending.is_empty(), "Only edit the draft, do not save or execute MCPs");
    }

    #[test]
    fn automatic_runner_membership_includes_future_runners_and_remembers_opt_outs() {
        let first = RunnerId::new();
        let future = RunnerId::new();
        let mut entry = LocalMcp {
            id: "test".into(),
            name: "Test".into(),
            command: vec!["/opt/mcp".into()],
            runners: vec![],
            all_runners: true,
            excluded_runners: vec![],
        };
        assert!(entry.enabled_on(first));
        assert!(entry.enabled_on(future));
        set_runner_enabled(&mut entry, first, false);
        let mut restored: LocalMcp =
            serde_json::from_value(serde_json::to_value(&entry).unwrap()).unwrap();
        assert!(!restored.enabled_on(first));
        assert!(restored.enabled_on(future));
        assert!(restored.enabled_on(RunnerId::new()));
        set_runner_enabled(&mut restored, first, true);
        assert!(restored.enabled_on(first));
        assert!(restored.excluded_runners.is_empty());
        assert!(restored.runners.is_empty());
    }

    #[test]
    fn runner_assignment_hides_confirmed_removals_but_keeps_offline_runners() {
        let mut app = crate::tests::fixture();
        let snapshot = std::sync::Arc::make_mut(app.snapshot.as_mut().unwrap());
        snapshot.runners[0].name = "Vorhandener Runner".into();
        snapshot.runners[0]
            .capabilities
            .push(LOCAL_MCP_CAPABILITY.into());
        let active = snapshot.runners[0].id;
        let mut offline = snapshot.runners[0].clone();
        offline.id = RunnerId::new();
        offline.name = "Offline-Runner".into();
        offline.connection = factory_core::ConnectionState::Unreachable;
        let offline_id = offline.id;
        let mut removed = snapshot.runners[0].clone();
        removed.id = RunnerId::new();
        removed.name = "Entfernter Runner".into();
        let removed_id = removed.id;
        snapshot.runners.extend([offline, removed]);
        let mut record = crate::local_runners::fixture(removed_id, &app.server);
        record.removed = true;
        app.local_runners.push(record);
        let mut other_factory = crate::local_runners::fixture(offline_id, "https://other.example");
        other_factory.removed = true;
        app.local_runners.push(other_factory);
        let mut entry = LocalMcp {
            all_runners: false,
            excluded_runners: vec![],
            id: "test".into(),
            name: "Test".into(),
            command: vec!["/opt/mcp".into()],
            runners: vec![active, offline_id, removed_id],
        };
        let ctx = app.backend.context().clone();
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| entry_fields(ui, &app, &mut entry));
            }));
        }
        assert_eq!(entry.runners, vec![active, offline_id]);
        fn contains(shape: &egui::epaint::Shape, label: &str) -> bool {
            match shape {
                egui::epaint::Shape::Text(text) => text.galley.job.text.contains(label),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, label)),
                _ => false,
            }
        }
        let output = output.unwrap();
        for label in ["Vorhandener Runner", "Offline-Runner"] {
            assert!(output.shapes.iter().any(|s| contains(&s.shape, label)));
        }
        assert!(!output
            .shapes
            .iter()
            .any(|s| contains(&s.shape, "Entfernter Runner")));
        assert!(app.settings.pending.is_empty());
        assert!(!app.settings_dirty);
    }

    #[test]
    fn add_and_delete_open_dialogs_and_close_preserves_the_add_draft() {
        let mut app = crate::tests::fixture();
        app.online = true;
        let blender = LocalMcp {
            all_runners: true,
            excluded_runners: vec![],
            id: "blender".into(),
            name: "Blender".into(),
            command: vec!["/opt/blender-mcp".into()],
            runners: vec![],
        };
        app.mcp_settings.catalog.saved = Some(LocalMcpCatalog {
            revision: Revision(1),
            entries: vec![blender],
        });
        app.mcp_settings.remote.entries.saved = Some(vec![]);
        let ctx = app.backend.context().clone();
        let render = |app: &mut App, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0, 720.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| show_catalog(ui, app, 690.0));
                },
            )
        };
        let click = |rect: egui::Rect| {
            let pos = rect.center();
            vec![
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
            ]
        };
        let escape = || {
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        let _ = render(&mut app, vec![]);
        let _ = render(&mut app, vec![]);
        let rect: egui::Rect = ctx.data_mut(|d| d.get_temp(egui::Id::new("test-mcp-add")).unwrap());
        let _ = render(&mut app, click(rect));
        assert!(matches!(app.mcp_settings.modal, Some(CatalogDialog::Add)));
        app.mcp_settings.new_local.as_mut().unwrap().name = "Entwurf".into();
        let _ = render(&mut app, vec![]);
        let _ = render(&mut app, escape());
        assert!(app.mcp_settings.modal.is_none());
        assert_eq!(app.mcp_settings.new_local.as_ref().unwrap().name, "Entwurf");
        let _ = render(&mut app, vec![]);
        let rect: egui::Rect = ctx.data_mut(|d| {
            d.get_temp(egui::Id::new(("test-icon-button", "MCP löschen")))
                .unwrap()
        });
        let _ = render(&mut app, click(rect));
        assert!(
            matches!(&app.mcp_settings.modal,Some(CatalogDialog::Delete {id,..}) if id == "blender")
        );
        assert_eq!(
            app.mcp_settings
                .catalog
                .saved
                .as_ref()
                .unwrap()
                .entries
                .len(),
            1
        );
        assert!(app.settings.pending.is_empty());
    }
}
