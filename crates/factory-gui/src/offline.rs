//! Offline preparation is local and server-scoped. Reconnection uploads drafts,
//! never starts work, and reuses the durable outbox for ambiguous responses.
use crate::{
    app::{App, AMBER, MUTED},
    dialogs::{Dialog, TaskDialog},
    model::Page,
    persistence::{CachedProjects, OfflineTask, TaskDraft},
};
use factory_core::*;
use factory_protocol::*;
use serde_json::json;

pub fn footer_notice() -> &'static str {
    crate::tr!("Offline-Modus: Aufgaben können nur lokal vorbereitet werden.")
}

impl App {
    pub fn available_projects(&self) -> &[Project] {
        self.snapshot
            .as_ref()
            .map(|s| s.projects.as_slice())
            .unwrap_or_else(|| {
                self.settings
                    .cached_projects
                    .iter()
                    .find(|cache| cache.server == self.server)
                    .map_or(&[], |cache| cache.projects.as_slice())
            })
    }

    pub(crate) fn cache_projects(&mut self, projects: &[Project]) {
        let cached = self
            .settings
            .cached_projects
            .iter_mut()
            .find(|c| c.server == self.server);
        if let Some(cached) = cached {
            if cached
                .projects
                .iter()
                .map(|p| (p.id, p.revision))
                .collect::<Vec<_>>()
                != projects
                    .iter()
                    .map(|p| (p.id, p.revision))
                    .collect::<Vec<_>>()
            {
                cached.projects = projects.to_vec();
                self.settings_dirty = true;
            }
        } else {
            self.settings.cached_projects.push(CachedProjects {
                server: self.server.clone(),
                projects: projects.to_vec(),
            });
            self.settings_dirty = true;
        }
    }

    pub(crate) fn enter_offline_mode(&mut self) {
        if self.settings.page != Page::Tasks || self.active_tab != 0 {
            self.set_page(Page::Tasks);
        }
        let allowed = matches!(
            &self.dialog,
            None | Some(
                Dialog::ConnectionInfo
                    | Dialog::Connection { .. }
                    | Dialog::LocalFactory { .. }
                    | Dialog::RemoveOfflineTask(_)
            )
        ) || matches!(&self.dialog, Some(Dialog::Task(task)) if !task.read_only && task.draft.ticket.is_none());
        if !allowed {
            if let Some(dialog) = self.dialog.take() {
                dialog.persist_drafts(self);
            }
            if let Some(settings) = self.factory_settings.take() {
                settings.persist(self);
                self.factory_settings = Some(settings);
            }
        }
    }

    pub(crate) fn save_offline_task(
        &mut self,
        id: Option<RequestId>,
        draft: TaskDraft,
    ) -> Result<RequestId, String> {
        let previous = self.settings.clone();
        let id = id.unwrap_or_else(RequestId::new);
        let name = self
            .available_projects()
            .iter()
            .find(|p| p.id == draft.project)
            .map_or_else(
                || crate::tr!("Nicht mehr verfügbares Projekt").into(),
                |p| p.name.clone(),
            );
        if let Some(item) = self
            .settings
            .offline_tasks
            .iter_mut()
            .find(|item| item.id == id)
        {
            if item.sync_request.is_some() {
                return Err(crate::tr!("Die Zustellung dieser Aufgabe muss zuerst abgeglichen werden.").into());
            }
            item.draft = draft.clone();
            item.edit = None;
            item.sync_error = None;
        } else {
            self.settings.offline_tasks.push(OfflineTask {
                id,
                draft: draft.clone(),
                project_name: name,
                edit: None,
                sync_request: None,
                sync_error: None,
            });
        }
        self.settings.task_drafts.retain(|d| {
            !(d.server == draft.server
                && d.project == draft.project
                && d.ticket.is_none()
                && d.prompt == draft.prompt
                && d.images == draft.images
                && d.files == draft.files)
        });
        self.settings_dirty = true;
        if !cfg!(test) && !self.backend.flush(self.settings.clone()) {
            self.settings = previous;
            return Err(
                crate::tr!("Aufgabe konnte nicht lokal gespeichert werden. Bitte erneut versuchen.").into(),
            );
        }
        self.notice(crate::tr!("Aufgabe lokal vorbereitet. Sie wird nach Wiederverbindung als Server-Entwurf übertragen."));
        Ok(id)
    }

    pub(crate) fn edit_offline_task(&mut self, id: RequestId) {
        if let Some(item) = self
            .settings
            .offline_tasks
            .iter_mut()
            .find(|item| item.id == id && item.sync_request.is_none())
        {
            let draft = item.edit.clone().unwrap_or_else(|| item.draft.clone());
            item.edit = Some(draft.clone());
            self.settings_dirty = true;
            self.dialog = Some(Dialog::Task(TaskDialog::offline(draft, id)));
        }
    }

    pub(crate) fn sync_offline_tasks(&mut self, reconnected: bool) {
        if !self.online {
            return;
        }
        let ids: Vec<_> = self
            .settings
            .offline_tasks
            .iter()
            .filter(|t| t.draft.server == self.server && t.edit.is_none())
            .map(|t| t.id)
            .collect();
        for id in ids {
            let Some(item) = self
                .settings
                .offline_tasks
                .iter()
                .find(|t| t.id == id)
                .cloned()
            else {
                continue;
            };
            let existing = item.sync_request.or_else(|| {
                self.settings
                    .pending
                    .iter()
                    .find(|p| {
                        p.server == self.server
                            && p.path == "/v1/tasks/submit-with-model"
                            && p.body["submission"]["task"]["request_id"] == json!(id)
                    })
                    .map(|p| p.id)
            });
            if let Some(request) = existing {
                self.settings
                    .offline_tasks
                    .iter_mut()
                    .find(|t| t.id == id)
                    .unwrap()
                    .sync_request = Some(request);
                if !self.in_flight.contains(&request)
                    && (!self.responses.contains_key(&request)
                        || reconnected && self.responses.get(&request).is_some_and(Result::is_err))
                {
                    self.responses.remove(&request);
                    self.retry(request);
                }
                continue;
            }
            if item.sync_error.is_some() {
                continue;
            }
            if !self
                .available_projects()
                .iter()
                .any(|p| p.id == item.draft.project)
            {
                self.settings
                    .offline_tasks
                    .iter_mut()
                    .find(|t| t.id == id)
                    .unwrap()
                    .sync_error = Some(
                    crate::tr!("Das ursprüngliche Projekt existiert auf dieser Factory nicht mehr.").into(),
                );
                self.settings_dirty = true;
                self.notice(crate::tr!("Eine Offline-Aufgabe konnte keinem Projekt zugeordnet werden. Sie bleibt lokal erhalten."));
                continue;
            }
            let request = self.write(
                crate::tr!("Offline-Aufgabe übertragen"),
                "POST",
                "/v1/tasks/submit-with-model".into(),
                json!(factory_protocol::models::SubmitTaskWithModel {
                    submission: SubmitTask {
                        edit_token: None,
                        draft: true,
                        files: item.draft.files,
                        task: SaveTask {
                            request_id: id,
                            id: None,
                            expected_revision: Revision(0),
                            project_id: item.draft.project,
                            requested_session: item.draft.requested_session,
                            prompt: item.draft.prompt,
                            images: item.draft.images
                        },
                    },
                    implementer: item.draft.implementer,
                    local_mcps: item.draft.local_mcps,
                }),
            );
            self.settings
                .offline_tasks
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .sync_request = Some(request);
            self.settings_dirty = true;
        }
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    if visible_local_tasks(app).next().is_none() {
        let message = if app.available_projects().is_empty() {
            crate::tr!("Noch keine Projekte gespeichert. Verbinde dich mit einer Factory, um Projekte auszuwählen.")
        } else {
            crate::tr!("Noch keine lokal vorbereiteten Aufgaben für dieses Projekt.")
        };
        crate::design::list_empty_state(ui, Page::Tasks.icon(), message);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("offline-tasks")
        .show(ui, |ui| local_tasks(ui, app));
}

fn visible_local_tasks(app: &App) -> impl Iterator<Item = &OfflineTask> {
    app.settings.offline_tasks.iter().filter(|t| {
        t.draft.server == app.server
            && (app.online
                || Some(t.draft.project) == app.settings.project
                || t.sync_error.is_some())
    })
}

pub fn local_tasks(ui: &mut egui::Ui, app: &mut App) {
    let tasks: Vec<_> = visible_local_tasks(app).cloned().collect();
    if tasks.is_empty() {
        return;
    }
    ui.strong(crate::tr!("Lokal vorbereitete Aufgaben"));
    for task in tasks {
        crate::design::task_row_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let draft = task.edit.as_ref().unwrap_or(&task.draft);
            let title = if draft.prompt.trim().is_empty() {
                crate::tr!("Aufgabe mit Anhängen").into()
            } else {
                crate::design::elide_first_line(&draft.prompt, 160)
            };
            ui.add(egui::Label::new(crate::design::elided_text_job(
                title,
                egui::FontId::proportional(18.0),
                ui.visuals().text_color(),
                ui.available_width(),
                1,
            )))
            .on_hover_text(&draft.prompt);
            ui.colored_label(MUTED, &task.project_name);
            ui.horizontal(|ui| {
                let status = if task.sync_error.is_some() {
                    crate::tr!("Übertragung prüfen")
                } else if task.edit.is_some() {
                    crate::tr!("Bearbeitung offen")
                } else if task.sync_request.is_some() {
                    crate::tr!("Wird übertragen")
                } else {
                    crate::tr!("Lokal vorbereitet")
                };
                ui.colored_label(AMBER, status);
                if !draft.images.is_empty() || !draft.files.is_empty() {
                    ui.label(crate::tr!("{} Bilder · {} Dateien", draft.images.len(),
                        draft.files.len()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let edit = ui
                        .add_enabled(task.sync_request.is_none(), egui::Button::new(crate::tr!("Bearbeiten")));
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(("test-offline-edit", task.id)), edit.rect)
                    });
                    if edit.clicked() {
                        app.edit_offline_task(task.id);
                    }
                    if task.sync_request.is_none() && ui.button(crate::tr!("Entfernen …")).clicked() {
                        app.dialog = Some(Dialog::RemoveOfflineTask(task.id));
                    }
                    if app.online {
                        if let Some(request) =
                            task.sync_request.filter(|id| !app.in_flight.contains(id))
                        {
                            if ui.button(crate::tr!("Zustellung prüfen")).clicked() {
                                app.responses.remove(&request);
                                app.retry(request);
                            }
                        } else if task.sync_error.is_some()
                            && task.edit.is_none()
                            && ui.button(crate::tr!("Erneut übertragen")).clicked()
                        {
                            app.settings
                                .offline_tasks
                                .iter_mut()
                                .find(|t| t.id == task.id)
                                .unwrap()
                                .sync_error = None;
                            app.sync_offline_tasks(false);
                        }
                    }
                });
            });
        });
    }
    ui.add_space(12.0);
}
