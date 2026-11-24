//! Project-scoped expert tools, with drafts retained across navigation.
use crate::{
    app::App,
    app::MUTED,
    design,
    dialogs::{read_content, Dialog},
    model::*,
};
use factory_core::*;
use factory_protocol::*;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectSection {
    Setup,
    Profile,
    Models,
    DefaultModels,
    Scheduler,
    SessionLimit,
    Supervisor,
    SupervisorHistory,
    Knowledge,
    Context,
    AddKnowledge,
    Ticket,
    Workspace,
    Commands,
    Usage,
    Budget,
    Devices,
    Invite,
    Revoke,
    Desktop,
    Pending,
}
impl ProjectSection {
    pub const ALL: [Self; 15] = [
        Self::Setup,
        Self::Profile,
        Self::Models,
        Self::Scheduler,
        Self::SessionLimit,
        Self::Supervisor,
        Self::SupervisorHistory,
        Self::Knowledge,
        Self::Context,
        Self::AddKnowledge,
        Self::Ticket,
        Self::Workspace,
        Self::Commands,
        Self::Usage,
        Self::Budget,
    ];
    pub const FACTORY: [Self; 7] = [
        Self::Models,
        Self::DefaultModels,
        Self::Devices,
        Self::Invite,
        Self::Revoke,
        Self::Desktop,
        Self::Pending,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Setup => "Einrichtung",
            Self::Profile => "Modell und Ausführung",
            Self::Models => "Modell-Anbieter",
            Self::DefaultModels => "Default-Modelle",
            Self::Scheduler => "Verteilung",
            Self::SessionLimit => "Sessionlimit",
            Self::Supervisor => "Supervisor",
            Self::SupervisorHistory => "Supervisorhistorie",
            Self::Knowledge => "Projektwissen",
            Self::Context => "Wissenskontext",
            Self::AddKnowledge => "Wissen hinzufügen",
            Self::Ticket => "Technisches Ticket",
            Self::Workspace => "Arbeitsbereich anlegen",
            Self::Commands => "Alle Aufträge",
            Self::Usage => "Nutzung und Kosten",
            Self::Budget => "Tokenbudget",
            Self::Devices => "Geräte und Zugänge",
            Self::Invite => "Gerät einladen",
            Self::Revoke => "Gerät widerrufen",
            Self::Desktop => "Desktop-Meldungen",
            Self::Pending => "Offene Zustellungen",
        }
    }
}

pub struct ProjectSettings {
    pub project: Option<ProjectId>,
    pub selected: ProjectSection,
    pub panes: BTreeMap<ProjectSection, Vec<Dialog>>,
}
impl ProjectSettings {
    pub fn new(project: ProjectId) -> Self {
        Self {
            project: Some(project),
            selected: ProjectSection::Setup,
            panes: BTreeMap::new(),
        }
    }
    pub fn factory() -> Self {
        Self {
            project: None,
            selected: ProjectSection::Desktop,
            panes: BTreeMap::new(),
        }
    }
    pub fn persist(&self, app: &mut App) {
        for pane in self.panes.values().flatten() {
            pane.persist_drafts(app);
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App, height: f32) -> bool {
        self.show_mode(ui, app, height, false)
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
                            if let Some(panes)=self.panes.get(&self.selected) {for pane in panes {pane.persist_drafts(app);}}
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
                 if let Some(id)=self.project {
                     if let Some(project)=app.snapshot.as_ref().and_then(|s|s.projects.iter().find(|p|p.id==id)) {ui.colored_label(MUTED,&project.name);}
                     else {ui.label("Dieses Projekt ist im aktuellen Serverstand nicht mehr verfügbar. Entwürfe bleiben erhalten.");return;}
                     if !embedded {ui.separator();}
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
                let panes=self.panes.entry(self.selected).or_default();
                if panes.is_empty() {
                    if let Some(dialog)=content(app,self.project,self.selected) {panes.push(dialog);}
                    else {ui.label("Dieses Projekt ist im aktuellen Serverstand nicht mehr verfügbar.");return;}
                }
                let back=panes.len()>1 && ui.button("Zurück zur Übersicht").clicked();
                if back {if let Some(pane)=panes.pop() {pane.persist_drafts(app);}}
                let remaining=(height-100.0).max(80.0);
                let pane=panes.last_mut().expect("project pane initialized");
                let pane_close=match pane {
                    Dialog::Form(form)=>form.show_in_settings(ui,app,pane_bottom,(remaining-145.0).max(40.0)),
                    Dialog::Settings=>crate::dialogs::desktop_settings(ui,app,remaining,!embedded),
                    Dialog::Pending=>crate::dialogs::pending_requests(ui,app,remaining,!embedded),
                    Dialog::Read {request,path,..}=>{
                        egui::ScrollArea::both().id_salt((self.project,self.selected,"read")).max_height((remaining-50.0).max(40.0)).show(ui,|ui| {
                            match app.responses.get(request).cloned() {
                                Some(Ok(value))=>read_content(ui,app,&value,path),
                                Some(Err(error))=>{ui.colored_label(crate::app::RED,error.message);}
                                None=>{ui.spinner();ui.label("Wird geladen …");}
                            }
                        });
                        if let Some(Ok(value))=app.responses.get(request) {if ui.button("Exportieren …").clicked() {app.backend.save_json("yoloforge-projekteinstellungen.json",value.clone());}}
                        false
                    }
                    _=>{ui.label("Diese Ansicht bitte über die Hauptnavigation öffnen.");false}
                };
                if pane_close {
                    if panes.len()>1 {if let Some(pane)=panes.pop() {pane.persist_drafts(app);}}
                    else if embedded {
                        if let Some(pane)=panes.pop() {pane.persist_drafts(app);}
                        self.selected=ProjectSection::Desktop;
                    } else {close=true;}
                }
                if let Some(next)=app.dialog.take() {panes.push(next);}
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
}

fn content(app: &mut App, id: Option<ProjectId>, section: ProjectSection) -> Option<Dialog> {
    if id.is_none() {
        let mut result=match section {
            ProjectSection::Desktop=>Some(Dialog::Settings),ProjectSection::Pending=>Some(Dialog::Pending),
            ProjectSection::Invite=>Some(Dialog::form("Gerät einladen","Rolle und Gültigkeit prüfen. Der Einladungscode wird erst nach Bestätigung angezeigt.","POST","/v1/devices/enrollment".into(),json!({"role":"operator","runner_id":null,"ttl_seconds":600}))),
            ProjectSection::Devices|ProjectSection::Revoke=>{app.read("Geräte und Zugänge","/v1/devices".into());app.dialog.take()},
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
    let saved = app.dialog.take();
    match section {
        ProjectSection::Setup => app.project_setup(Some(p.clone())),
        ProjectSection::Profile => {
            app.dialog = Some(Dialog::form(
                "Ausführungsprofil",
                "Gilt für neue Aufträge; laufende Aufträge behalten ihr Profil.",
                "PATCH",
                format!("/v1/projects/{}/profile", p.id),
                json!(UpdateProfile {
                    request_id: RequestId::new(),
                    expected_revision: p.revision,
                    profile: p.profile.clone()
                }),
            ))
        }
        ProjectSection::Scheduler => app.scheduler_for_project(&p),
        ProjectSection::SessionLimit => {
            app.dialog = Some(Dialog::form(
                "Sessionlimit ändern",
                "Maximal gleichzeitig arbeitende Sessions dieses Projekts (1–16). Eine niedrigere Grenze stoppt keine bereits laufende Arbeit.",
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
            "Supervisor-Konfiguration",
            format!("/v1/projects/{}/supervisor", p.id),
        ),
        ProjectSection::SupervisorHistory => app.read(
            "Supervisorhistorie",
            format!("/v1/projects/{}/supervisor/evaluations", p.id),
        ),
        ProjectSection::Knowledge => {
            app.read("Projektwissen", format!("/v1/projects/{}/knowledge", p.id))
        }
        ProjectSection::Context => app.read(
            "Wissenskontext",
            format!("/v1/projects/{}/context?budget_bytes=8192", p.id),
        ),
        ProjectSection::AddKnowledge => {
            app.dialog = Some(Dialog::form(
                "Projektwissen hinzufügen",
                "Wissen wird als Referenz für weitere Aufgaben gespeichert.",
                "POST",
                "/v1/knowledge".into(),
                json!(CreateKnowledge {
                    branch: None,
                    supersedes: None,
                    request_id: RequestId::new(),
                    project_id: p.id,
                    ticket_id: None,
                    title: String::new(),
                    content: String::new(),
                    source: "Desktop".into(),
                    source_command: None,
                    priority: 1
                }),
            ))
        }
        ProjectSection::Ticket => {
            app.dialog = Some(Dialog::form(
                "Technisches Ticket",
                "Für normale Aufgaben den Plus-Button verwenden.",
                "POST",
                "/v1/tickets".into(),
                json!(CreateTicket {
                    request_id: RequestId::new(),
                    project_id: p.id,
                    title: String::new(),
                    description: String::new(),
                    acceptance_criteria: String::new(),
                    priority: 1
                }),
            ))
        }
        ProjectSection::Workspace => {
            let runner = p
                .setup
                .as_ref()
                .map(|s| s.runner_id)
                .or_else(|| app.snapshot.as_ref()?.runners.first().map(|r| r.id))
                .unwrap_or_default();
            app.dialog = Some(Dialog::form(
                "Arbeitsbereich anlegen",
                "Runner und Aufgaben für diesen Arbeitsbereich wählen.",
                "POST",
                "/v1/streams".into(),
                json!(CreateStream {
                    request_id: RequestId::new(),
                    project_id: p.id,
                    goal: String::new(),
                    runner_id: runner,
                    ticket_ids: vec![]
                }),
            ));
        }
        ProjectSection::Commands => {
            let snapshot = app.snapshot.as_ref()?;
            let streams = snapshot
                .streams
                .iter()
                .filter(|w| w.project_id == p.id)
                .cloned()
                .collect::<Vec<_>>();
            let commands = snapshot
                .commands
                .iter()
                .filter(|c| command_project(snapshot, c) == Some(p.id))
                .cloned()
                .collect::<Vec<_>>();
            app.inspect(
                "Arbeitsbereiche und Aufträge",
                json!({"streams":streams,"commands":commands}),
            );
        }
        ProjectSection::Usage => {
            app.read("Nutzung und Kosten", format!("/v1/projects/{}/usage", p.id))
        }
        ProjectSection::Budget => {
            app.read("Tokenbudget", format!("/v1/projects/{}/token-budget", p.id))
        }
        _ => return None,
    }
    let mut result = app.dialog.take();
    restore_form(app, &mut result);
    app.dialog = saved;
    result
}

pub(crate) fn restore_form(app: &App, result: &mut Option<Dialog>) {
    if let Some(Dialog::Form(form)) = result {
        if let Some(draft) = app.settings.form_drafts.iter().find(|d| {
            d.server == app.server
                && d.path == form.path
                && d.method == form.method
                && d.body.get("project_id") == form.body.get("project_id")
        }) {
            form.body = draft.body.clone();
        }
    }
}
